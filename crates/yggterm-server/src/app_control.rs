use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const APP_CONTROL_REQUESTS_DIR: &str = "app-control-requests";
const APP_CONTROL_RESPONSES_DIR: &str = "app-control-responses";
const APP_CONTROL_CAPTURES_DIR: &str = "screenshots";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotTarget {
    App,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlViewMode {
    Preview,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlDragPlacement {
    Before,
    Into,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AppControlDragCommand {
    Begin {
        row_path: String,
    },
    Hover {
        row_path: String,
        placement: AppControlDragPlacement,
    },
    Drop,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppControlCommand {
    CaptureScreenshot {
        target: ScreenshotTarget,
        output_path: String,
    },
    Drag {
        command: AppControlDragCommand,
    },
    DescribeRows,
    OpenPath {
        session_path: String,
        #[serde(default)]
        view_mode: Option<AppControlViewMode>,
    },
    FocusWindow,
    DescribeState,
}

impl AppControlCommand {
    pub fn name(&self) -> &'static str {
        match self {
            Self::CaptureScreenshot { .. } => "capture_screenshot",
            Self::Drag { .. } => "drag",
            Self::DescribeRows => "describe_rows",
            Self::OpenPath { .. } => "open_path",
            Self::FocusWindow => "focus_window",
            Self::DescribeState => "describe_state",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppControlRequest {
    pub request_id: String,
    pub created_at_ms: u128,
    #[serde(default)]
    pub preferred_pid: Option<u32>,
    pub command: AppControlCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppControlResponse {
    pub request_id: String,
    pub handled_by_pid: u32,
    pub completed_at_ms: u128,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn app_control_requests_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_REQUESTS_DIR)
}

pub fn app_control_responses_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_RESPONSES_DIR)
}

pub fn app_control_captures_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_CAPTURES_DIR)
}

pub fn default_screenshot_output_path(home: &Path, request_id: &str) -> PathBuf {
    app_control_captures_dir(home).join(format!("app-{request_id}.png"))
}

pub fn enqueue_app_control_request(
    home: &Path,
    command: AppControlCommand,
    preferred_pid: Option<u32>,
) -> Result<AppControlRequest> {
    let requests_dir = app_control_requests_dir(home);
    let captures_dir = app_control_captures_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    fs::create_dir_all(&captures_dir).with_context(|| {
        format!(
            "creating app control captures dir {}",
            captures_dir.display()
        )
    })?;
    let request = AppControlRequest {
        request_id: Uuid::new_v4().to_string(),
        created_at_ms: current_millis(),
        preferred_pid,
        command,
    };
    let final_path = requests_dir.join(format!("{}.json", request.request_id));
    let temp_path = requests_dir.join(format!("{}.json.tmp", request.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(&request)?)
        .with_context(|| format!("writing app control request {}", temp_path.display()))?;
    fs::rename(&temp_path, &final_path)
        .with_context(|| format!("publishing app control request {}", final_path.display()))?;
    Ok(request)
}

pub fn take_next_app_control_request(
    home: &Path,
    worker_pid: u32,
) -> Result<Option<(PathBuf, AppControlRequest)>> {
    let requests_dir = app_control_requests_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    let mut entries = fs::read_dir(&requests_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("inflight-"))
        })
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let request = match serde_json::from_slice::<AppControlRequest>(&bytes) {
            Ok(request) => request,
            Err(_) => {
                let _ = fs::remove_file(&path);
                continue;
            }
        };
        if request
            .preferred_pid
            .is_some_and(|preferred_pid| preferred_pid != worker_pid)
        {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("request.json");
        let inflight_path = requests_dir.join(format!("inflight-{worker_pid}-{file_name}"));
        if fs::rename(&path, &inflight_path).is_err() {
            continue;
        }
        return Ok(Some((inflight_path, request)));
    }
    Ok(None)
}

pub fn complete_app_control_request(
    home: &Path,
    inflight_path: &Path,
    response: &AppControlResponse,
) -> Result<PathBuf> {
    let responses_dir = app_control_responses_dir(home);
    fs::create_dir_all(&responses_dir).with_context(|| {
        format!(
            "creating app control responses dir {}",
            responses_dir.display()
        )
    })?;
    let response_path = responses_dir.join(format!("{}.json", response.request_id));
    let temp_path = responses_dir.join(format!("{}.json.tmp", response.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(response)?)
        .with_context(|| format!("writing app control response {}", temp_path.display()))?;
    fs::rename(&temp_path, &response_path).with_context(|| {
        format!(
            "publishing app control response {}",
            response_path.display()
        )
    })?;
    let _ = fs::remove_file(inflight_path);
    Ok(response_path)
}

pub fn wait_for_app_control_response(
    home: &Path,
    request_id: &str,
    timeout: Duration,
) -> Result<AppControlResponse> {
    let response_path = app_control_responses_dir(home).join(format!("{request_id}.json"));
    let started = std::time::Instant::now();
    while started.elapsed() <= timeout {
        if response_path.is_file() {
            let bytes = fs::read(&response_path).with_context(|| {
                format!("reading app control response {}", response_path.display())
            })?;
            let response =
                serde_json::from_slice::<AppControlResponse>(&bytes).with_context(|| {
                    format!("parsing app control response {}", response_path.display())
                })?;
            let _ = fs::remove_file(&response_path);
            return Ok(response);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "timed out waiting for app control response {} after {} ms",
        request_id,
        timeout.as_millis()
    )
}

pub fn enqueue_screenshot_request(
    home: &Path,
    target: ScreenshotTarget,
    output_path: Option<PathBuf>,
    preferred_pid: Option<u32>,
) -> Result<AppControlRequest> {
    let request_id = Uuid::new_v4().to_string();
    let output_path = output_path
        .unwrap_or_else(|| default_screenshot_output_path(home, &request_id))
        .display()
        .to_string();
    let request = AppControlRequest {
        request_id,
        created_at_ms: current_millis(),
        preferred_pid,
        command: AppControlCommand::CaptureScreenshot {
            target,
            output_path,
        },
    };
    let requests_dir = app_control_requests_dir(home);
    let captures_dir = app_control_captures_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    fs::create_dir_all(&captures_dir).with_context(|| {
        format!(
            "creating app control captures dir {}",
            captures_dir.display()
        )
    })?;
    let final_path = requests_dir.join(format!("{}.json", request.request_id));
    let temp_path = requests_dir.join(format!("{}.json.tmp", request.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(&request)?)
        .with_context(|| format!("writing app control request {}", temp_path.display()))?;
    fs::rename(&temp_path, &final_path)
        .with_context(|| format!("publishing app control request {}", final_path.display()))?;
    Ok(request)
}

pub fn current_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
