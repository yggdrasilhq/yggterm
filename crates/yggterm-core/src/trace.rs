use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const EVENT_TRACE_FILENAME: &str = "event-trace.jsonl";
const EVENT_TRACE_ROTATED_FILENAME: &str = "event-trace.previous.jsonl";
const EVENT_TRACE_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTraceRecord {
    pub ts_ms: u128,
    pub pid: u32,
    pub component: String,
    pub category: String,
    pub name: String,
    #[serde(default)]
    pub payload: Value,
}

pub fn event_trace_path(home: &Path) -> PathBuf {
    home.join(EVENT_TRACE_FILENAME)
}

pub fn append_trace_event(
    home: &Path,
    component: impl Into<String>,
    category: impl Into<String>,
    name: impl Into<String>,
    payload: Value,
) {
    let _ = fs::create_dir_all(home);
    let path = event_trace_path(home);
    rotate_trace_file_if_needed(&path);
    let record = EventTraceRecord {
        ts_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        pid: std::process::id(),
        component: component.into(),
        category: category.into(),
        name: name.into(),
        payload,
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        if let Ok(mut line) = serde_json::to_vec(&record) {
            line.push(b'\n');
            let _ = file.write_all(&line);
        }
    }
}

fn rotate_trace_file_if_needed(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() < EVENT_TRACE_MAX_BYTES {
        return;
    }
    let rotated_path = path.with_file_name(EVENT_TRACE_ROTATED_FILENAME);
    let _ = fs::remove_file(&rotated_path);
    let _ = fs::rename(path, rotated_path);
}

pub struct EventTraceSpan {
    home: PathBuf,
    component: String,
    category: String,
    name: String,
    started_at: Instant,
}

impl EventTraceSpan {
    pub fn start(
        home: impl Into<PathBuf>,
        component: impl Into<String>,
        category: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            home: home.into(),
            component: component.into(),
            category: category.into(),
            name: name.into(),
            started_at: Instant::now(),
        }
    }

    pub fn finish(self, payload: Value) {
        append_trace_event(
            &self.home,
            self.component,
            self.category,
            self.name,
            json!({
                "duration_ms": self.started_at.elapsed().as_secs_f64() * 1000.0,
                "meta": payload,
            }),
        );
    }
}

pub fn read_trace_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines().map_while(Result::ok).collect::<Vec<_>>();
    if lines.len() > max_lines {
        let keep_from = lines.len().saturating_sub(max_lines);
        lines.drain(0..keep_from);
    }
    lines
}

pub fn follow_trace_lines(path: &Path, initial_lines: usize, poll_ms: u64) -> ! {
    let mut emitted = read_trace_tail(path, initial_lines);
    for line in emitted.drain(..) {
        println!("{line}");
    }
    let mut last_len = fs::metadata(path)
        .map(|meta| meta.len())
        .unwrap_or_default();
    loop {
        sleep(Duration::from_millis(poll_ms.max(100)));
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        let current_len = metadata.len();
        if current_len < last_len {
            last_len = 0;
        }
        if current_len == last_len {
            continue;
        }
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let _ = reader.seek_relative(last_len as i64);
        let mut line = String::new();
        while reader
            .read_line(&mut line)
            .ok()
            .is_some_and(|bytes| bytes > 0)
        {
            print!("{line}");
            line.clear();
        }
        let _ = std::io::stdout().flush();
        last_len = current_len;
    }
}
