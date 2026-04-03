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
const APP_CONTROL_RECORDINGS_DIR: &str = "recordings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotTarget {
    App,
    PreviewViewport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlViewMode {
    Preview,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlPreviewLayout {
    Chat,
    Graph,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppControlCommand {
    SetMainZoom {
        value: f32,
        #[serde(default)]
        view_mode: Option<AppControlViewMode>,
    },
    SetSearch {
        query: String,
        #[serde(default)]
        focused: Option<bool>,
    },
    CaptureScreenshot {
        target: ScreenshotTarget,
        output_path: String,
    },
    ScrollPreview {
        #[serde(default)]
        top_px: Option<f64>,
        #[serde(default)]
        ratio: Option<f64>,
    },
    SetPreviewLayout {
        layout: AppControlPreviewLayout,
    },
    CaptureScreenRecording {
        output_path: String,
        duration_secs: u64,
    },
    SetMaximized {
        enabled: bool,
    },
    SetFullscreen {
        enabled: bool,
    },
    Drag {
        command: AppControlDragCommand,
    },
    CreateTerminal {
        #[serde(default)]
        machine_key: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        title_hint: Option<String>,
    },
    SendTerminalInput {
        session_path: String,
        data: String,
    },
    RemoveSession {
        session_path: String,
    },
    SetRowExpanded {
        row_path: String,
        expanded: bool,
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
            Self::SetMainZoom { .. } => "set_main_zoom",
            Self::SetSearch { .. } => "set_search",
            Self::CaptureScreenshot { .. } => "capture_screenshot",
            Self::ScrollPreview { .. } => "scroll_preview",
            Self::SetPreviewLayout { .. } => "set_preview_layout",
            Self::CaptureScreenRecording { .. } => "capture_screen_recording",
            Self::SetMaximized { .. } => "set_maximized",
            Self::SetFullscreen { .. } => "set_fullscreen",
            Self::Drag { .. } => "drag",
            Self::CreateTerminal { .. } => "create_terminal",
            Self::SendTerminalInput { .. } => "send_terminal_input",
            Self::RemoveSession { .. } => "remove_session",
            Self::SetRowExpanded { .. } => "set_row_expanded",
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

pub fn app_control_requests_pending(home: &Path) -> bool {
    let requests_dir = app_control_requests_dir(home);
    let Ok(entries) = fs::read_dir(&requests_dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("inflight-"))
    })
}

pub fn app_control_responses_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_RESPONSES_DIR)
}

pub fn app_control_captures_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_CAPTURES_DIR)
}

pub fn app_control_recordings_dir(home: &Path) -> PathBuf {
    home.join(APP_CONTROL_RECORDINGS_DIR)
}

pub fn default_screenshot_output_path(home: &Path, request_id: &str) -> PathBuf {
    app_control_captures_dir(home).join(format!("app-{request_id}.png"))
}

pub fn default_recording_output_path(home: &Path, request_id: &str) -> PathBuf {
    app_control_recordings_dir(home).join(format!("app-{request_id}.mov"))
}

pub fn enqueue_app_control_request(
    home: &Path,
    command: AppControlCommand,
    preferred_pid: Option<u32>,
) -> Result<AppControlRequest> {
    let requests_dir = app_control_requests_dir(home);
    let captures_dir = app_control_captures_dir(home);
    let recordings_dir = app_control_recordings_dir(home);
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
    fs::create_dir_all(&recordings_dir).with_context(|| {
        format!(
            "creating app control recordings dir {}",
            recordings_dir.display()
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
    recover_stale_inflight_requests(&requests_dir)?;
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
        if request.preferred_pid.is_some_and(|preferred_pid| {
            preferred_pid != worker_pid && process_is_alive(preferred_pid)
        }) {
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

fn recover_stale_inflight_requests(requests_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(requests_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("inflight-")
            || path.extension().and_then(|ext| ext.to_str()) != Some("json")
        {
            continue;
        }
        let Some(worker_pid) = parse_inflight_worker_pid(name) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        if process_is_alive(worker_pid) {
            continue;
        }
        let Some(original_name) = name.splitn(3, '-').nth(2) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        let recovered_path = requests_dir.join(original_name);
        if recovered_path.exists() {
            let _ = fs::remove_file(&path);
            continue;
        }
        let _ = fs::rename(&path, &recovered_path);
    }
    Ok(())
}

fn parse_inflight_worker_pid(file_name: &str) -> Option<u32> {
    let rest = file_name.strip_prefix("inflight-")?;
    let pid = rest.split('-').next()?;
    pid.parse().ok()
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(unix))]
fn process_is_alive(pid: u32) -> bool {
    pid != 0
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

pub fn enqueue_screen_recording_request(
    home: &Path,
    output_path: Option<PathBuf>,
    duration_secs: u64,
    preferred_pid: Option<u32>,
) -> Result<AppControlRequest> {
    let request_id = Uuid::new_v4().to_string();
    let output_path = output_path
        .unwrap_or_else(|| default_recording_output_path(home, &request_id))
        .display()
        .to_string();
    let request = AppControlRequest {
        request_id,
        created_at_ms: current_millis(),
        preferred_pid,
        command: AppControlCommand::CaptureScreenRecording {
            output_path,
            duration_secs: duration_secs.max(1),
        },
    };
    let requests_dir = app_control_requests_dir(home);
    let recordings_dir = app_control_recordings_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating app control requests dir {}",
            requests_dir.display()
        )
    })?;
    fs::create_dir_all(&recordings_dir).with_context(|| {
        format!(
            "creating app control recordings dir {}",
            recordings_dir.display()
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
