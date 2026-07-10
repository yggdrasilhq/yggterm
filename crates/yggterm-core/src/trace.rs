use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::retention::{
    DIAGNOSTIC_RETENTION_MAX_AGE_MS, JsonlRetention, now_epoch_ms, prune_jsonl_generations,
    rotate_jsonl_with_retention,
};

pub const EVENT_TRACE_FILENAME: &str = "event-trace.jsonl";
const EVENT_TRACE_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Rotated event-trace generations: up to 3 days of history, hard-capped at
/// 256 MiB total so a trace flood (a reveal loop, a render storm) cannot eat
/// the disk. On a heavy day that budget still holds ~3 days at the observed
/// ~80 MiB/day write rate.
const EVENT_TRACE_RETENTION: JsonlRetention = JsonlRetention {
    live_max_bytes: EVENT_TRACE_MAX_BYTES,
    generations_max_bytes: 256 * 1024 * 1024,
    max_age_ms: DIAGNOSTIC_RETENTION_MAX_AGE_MS,
};

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

/// A cached, append-mode handle to one trace file plus an in-memory byte
/// counter. Keeping the handle open lets `append_trace_event` skip the
/// `create_dir_all` + `metadata` stat + `open` + `close` syscalls it used to
/// pay on every single call — under a reveal/forward-loop flood that per-call
/// cost was the dominant on-thread I/O. We still issue one `write` per event so
/// followers (`follow_trace_lines`) see records immediately.
struct TraceWriter {
    file: File,
    bytes_written: u64,
}

fn open_trace_writer(path: &Path) -> Option<TraceWriter> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let bytes_written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Some(TraceWriter {
        file,
        bytes_written,
    })
}

/// Per-path cache of open trace handles. Keyed by path so a process that writes
/// to more than one home directory stays correct; in practice there is one
/// entry. The mutex serializes writers, so append-mode writes never interleave.
fn trace_writers() -> &'static Mutex<HashMap<PathBuf, TraceWriter>> {
    static WRITERS: OnceLock<Mutex<HashMap<PathBuf, TraceWriter>>> = OnceLock::new();
    WRITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn append_trace_event(
    home: &Path,
    component: impl Into<String>,
    category: impl Into<String>,
    name: impl Into<String>,
    payload: Value,
) {
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
    let Ok(mut line) = serde_json::to_vec(&record) else {
        return;
    };
    line.push(b'\n');
    write_trace_line(home, &line);
}

fn write_trace_line(home: &Path, line: &[u8]) {
    let path = event_trace_path(home);
    let mut writers = trace_writers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if !writers.contains_key(&path) {
        let _ = fs::create_dir_all(home);
        // First write through this process: sweep expired generations once so
        // the "at most 3 days" cap holds even across idle stretches where the
        // live file never reaches the rotation size.
        prune_jsonl_generations(&path, EVENT_TRACE_RETENTION, now_epoch_ms());
        match open_trace_writer(&path) {
            Some(writer) => {
                writers.insert(path.clone(), writer);
            }
            None => return,
        }
    }

    // Rotate off the in-memory counter so we never stat per call. This matches
    // the original "rotate when the existing file is already at the cap"
    // behavior; the new line then lands in a fresh file.
    if writers
        .get(&path)
        .is_some_and(|writer| writer.bytes_written >= EVENT_TRACE_MAX_BYTES)
    {
        // Close the handle before renaming the inode, otherwise we would keep
        // appending to the rotated-away file.
        writers.remove(&path);
        rotate_jsonl_with_retention(&path, EVENT_TRACE_RETENTION, now_epoch_ms());
        let _ = fs::create_dir_all(home);
        match open_trace_writer(&path) {
            Some(writer) => {
                writers.insert(path.clone(), writer);
            }
            None => return,
        }
    }

    if let Some(writer) = writers.get_mut(&path) {
        if writer.file.write_all(line).is_ok() {
            writer.bytes_written = writer.bytes_written.saturating_add(line.len() as u64);
        } else {
            // A failed write almost always means the handle is stale (file
            // removed/replaced underneath us); drop it so the next call reopens.
            writers.remove(&path);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_home(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("ygg-trace-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn cached_writer_appends_every_event() {
        let home = unique_home("append");
        for i in 0..5 {
            append_trace_event(&home, "test", "unit", "ev", json!({ "i": i }));
        }
        // A separate reader (mimicking the follower process) sees all records,
        // proving each event is flushed to disk, not held in an in-memory buffer.
        let lines = read_trace_tail(&event_trace_path(&home), 100);
        assert_eq!(lines.len(), 5, "expected 5 trace lines, got {lines:?}");
        for (i, line) in lines.iter().enumerate() {
            let rec: EventTraceRecord = serde_json::from_str(line).expect("valid jsonl");
            assert_eq!(rec.name, "ev");
            assert_eq!(rec.payload["i"], json!(i));
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn cached_writer_reuses_one_handle() {
        let home = unique_home("reuse");
        let path = event_trace_path(&home);
        append_trace_event(&home, "test", "unit", "first", json!({}));
        // After the first write the handle is cached; a second write must not
        // create a new entry, and the byte counter must reflect both lines.
        append_trace_event(&home, "test", "unit", "second", json!({}));
        let on_disk = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let counted = {
            let writers = trace_writers().lock().unwrap();
            writers.get(&path).map(|w| w.bytes_written).unwrap_or(0)
        };
        assert_eq!(
            counted, on_disk,
            "in-memory byte counter must track the real file size"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn cached_writer_rotates_at_cap() {
        // Use a dedicated home so the global cache entry is isolated.
        let home = unique_home("rotate");
        let path = event_trace_path(&home);

        // Pre-seed the live file just past the cap, then force the cache to
        // adopt it by writing once (open picks up the existing size).
        let _ = fs::create_dir_all(&home);
        {
            let big = vec![b'x'; (EVENT_TRACE_MAX_BYTES + 16) as usize];
            fs::write(&path, &big).unwrap();
        }
        // Drop any stale cached handle from a prior run of this path.
        trace_writers().lock().unwrap().remove(&path);

        append_trace_event(&home, "test", "unit", "after-cap", json!({ "k": 1 }));

        let has_generation = fs::read_dir(&home).unwrap().flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("event-trace.g") && name.ends_with(".jsonl")
        });
        assert!(
            has_generation,
            "a timestamped generation should exist after rotation"
        );
        // The fresh live file holds only the post-rotation record.
        let lines = read_trace_tail(&path, 100);
        assert_eq!(lines.len(), 1, "fresh file should hold one record");
        let rec: EventTraceRecord = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(rec.name, "after-cap");

        trace_writers().lock().unwrap().remove(&path);
        let _ = fs::remove_dir_all(&home);
    }
}
