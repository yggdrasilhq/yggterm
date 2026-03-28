use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const SCREENSHOT_REQUESTS_DIR: &str = "screenshot-requests";
const SCREENSHOT_RESPONSES_DIR: &str = "screenshot-responses";
const SCREENSHOT_CAPTURES_DIR: &str = "screenshots";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotTarget {
    App,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotRequest {
    pub request_id: String,
    pub target: ScreenshotTarget,
    pub created_at_ms: u128,
    pub output_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResponse {
    pub request_id: String,
    pub handled_by_pid: u32,
    pub completed_at_ms: u128,
    pub output_path: Option<String>,
    pub error: Option<String>,
}

pub fn screenshot_requests_dir(home: &Path) -> PathBuf {
    home.join(SCREENSHOT_REQUESTS_DIR)
}

pub fn screenshot_responses_dir(home: &Path) -> PathBuf {
    home.join(SCREENSHOT_RESPONSES_DIR)
}

pub fn screenshot_captures_dir(home: &Path) -> PathBuf {
    home.join(SCREENSHOT_CAPTURES_DIR)
}

pub fn default_screenshot_output_path(home: &Path, request_id: &str) -> PathBuf {
    screenshot_captures_dir(home).join(format!("app-{request_id}.png"))
}

pub fn enqueue_screenshot_request(
    home: &Path,
    target: ScreenshotTarget,
    output_path: Option<PathBuf>,
) -> Result<ScreenshotRequest> {
    let requests_dir = screenshot_requests_dir(home);
    let captures_dir = screenshot_captures_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating screenshot requests dir {}",
            requests_dir.display()
        )
    })?;
    fs::create_dir_all(&captures_dir).with_context(|| {
        format!(
            "creating screenshot captures dir {}",
            captures_dir.display()
        )
    })?;
    let request_id = Uuid::new_v4().to_string();
    let request = ScreenshotRequest {
        output_path: output_path
            .unwrap_or_else(|| default_screenshot_output_path(home, &request_id))
            .display()
            .to_string(),
        request_id: request_id.clone(),
        target,
        created_at_ms: current_millis(),
    };
    let final_path = requests_dir.join(format!("{}.json", request.request_id));
    let temp_path = requests_dir.join(format!("{}.json.tmp", request.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(&request)?)
        .with_context(|| format!("writing screenshot request {}", temp_path.display()))?;
    fs::rename(&temp_path, &final_path)
        .with_context(|| format!("publishing screenshot request {}", final_path.display()))?;
    Ok(request)
}

pub fn take_next_screenshot_request(
    home: &Path,
    worker_pid: u32,
) -> Result<Option<(PathBuf, ScreenshotRequest)>> {
    let requests_dir = screenshot_requests_dir(home);
    fs::create_dir_all(&requests_dir).with_context(|| {
        format!(
            "creating screenshot requests dir {}",
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
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("request.json");
        let inflight_path = requests_dir.join(format!("inflight-{worker_pid}-{file_name}"));
        if fs::rename(&path, &inflight_path).is_err() {
            continue;
        }
        let bytes = fs::read(&inflight_path)
            .with_context(|| format!("reading screenshot request {}", inflight_path.display()))?;
        let request = serde_json::from_slice::<ScreenshotRequest>(&bytes)
            .with_context(|| format!("parsing screenshot request {}", inflight_path.display()))?;
        return Ok(Some((inflight_path, request)));
    }
    Ok(None)
}

pub fn complete_screenshot_request(
    home: &Path,
    inflight_path: &Path,
    response: &ScreenshotResponse,
) -> Result<PathBuf> {
    let responses_dir = screenshot_responses_dir(home);
    fs::create_dir_all(&responses_dir).with_context(|| {
        format!(
            "creating screenshot responses dir {}",
            responses_dir.display()
        )
    })?;
    let response_path = responses_dir.join(format!("{}.json", response.request_id));
    let temp_path = responses_dir.join(format!("{}.json.tmp", response.request_id));
    fs::write(&temp_path, serde_json::to_vec_pretty(response)?)
        .with_context(|| format!("writing screenshot response {}", temp_path.display()))?;
    fs::rename(&temp_path, &response_path)
        .with_context(|| format!("publishing screenshot response {}", response_path.display()))?;
    let _ = fs::remove_file(inflight_path);
    Ok(response_path)
}

pub fn wait_for_screenshot_response(
    home: &Path,
    request_id: &str,
    timeout: Duration,
) -> Result<ScreenshotResponse> {
    let response_path = screenshot_responses_dir(home).join(format!("{request_id}.json"));
    let started = std::time::Instant::now();
    while started.elapsed() <= timeout {
        if response_path.is_file() {
            let bytes = fs::read(&response_path).with_context(|| {
                format!("reading screenshot response {}", response_path.display())
            })?;
            let response =
                serde_json::from_slice::<ScreenshotResponse>(&bytes).with_context(|| {
                    format!("parsing screenshot response {}", response_path.display())
                })?;
            let _ = fs::remove_file(&response_path);
            return Ok(response);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "timed out waiting for screenshot response {} after {} ms",
        request_id,
        timeout.as_millis()
    )
}

fn current_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
