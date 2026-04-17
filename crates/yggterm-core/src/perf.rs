use serde_json::{Value, json};
use std::fs::{self, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const PERF_TELEMETRY_FILENAME: &str = "perf-telemetry.jsonl";
pub const PERF_TELEMETRY_ROTATED_FILENAME: &str = "perf-telemetry.previous.jsonl";
pub const PERF_TELEMETRY_MAX_BYTES: u64 = 16 * 1024 * 1024;

pub fn perf_telemetry_path(home: &Path) -> PathBuf {
    home.join(PERF_TELEMETRY_FILENAME)
}

pub fn append_bounded_jsonl_record(
    path: &Path,
    rotated_filename: &str,
    max_bytes: u64,
    record: &Value,
) {
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = create_dir_all(parent);
    let Ok(mut line) = serde_json::to_vec(record) else {
        return;
    };
    line.push(b'\n');
    rotate_jsonl_if_needed(path, rotated_filename, max_bytes, line.len() as u64);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(&line);
    }
}

pub fn append_perf_event(home: &Path, category: &str, name: &str, payload: Value) {
    let _ = create_dir_all(home);
    let path = perf_telemetry_path(home);
    let event = json!({
        "ts_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        "category": category,
        "name": name,
        "payload": payload,
    });
    append_bounded_jsonl_record(
        &path,
        PERF_TELEMETRY_ROTATED_FILENAME,
        PERF_TELEMETRY_MAX_BYTES,
        &event,
    );
}

fn rotate_jsonl_if_needed(path: &Path, rotated_filename: &str, max_bytes: u64, incoming_len: u64) {
    let rotated_path = path.with_file_name(rotated_filename);
    if fs::metadata(&rotated_path)
        .map(|metadata| metadata.len() > max_bytes)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&rotated_path);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() > max_bytes {
        let _ = fs::remove_file(path);
        return;
    }
    if metadata.len().saturating_add(incoming_len) <= max_bytes {
        return;
    }
    let _ = fs::remove_file(&rotated_path);
    let _ = fs::rename(path, rotated_path);
}

pub struct PerfSpan {
    home: PathBuf,
    category: String,
    name: String,
    started_at: Instant,
}

impl PerfSpan {
    pub fn start(
        home: impl Into<PathBuf>,
        category: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            home: home.into(),
            category: category.into(),
            name: name.into(),
            started_at: Instant::now(),
        }
    }

    pub fn finish(self, payload: Value) {
        append_perf_event(
            &self.home,
            &self.category,
            &self.name,
            json!({
                "duration_ms": self.started_at.elapsed().as_secs_f64() * 1000.0,
                "meta": payload,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yggterm-perf-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn append_bounded_jsonl_record_rotates_when_file_would_overflow() {
        let dir = temp_test_dir("rotate");
        let path = dir.join("test.jsonl");
        let first = json!({ "message": "a".repeat(90) });
        let second = json!({ "message": "b".repeat(90) });

        append_bounded_jsonl_record(&path, "test.previous.jsonl", 120, &first);
        append_bounded_jsonl_record(&path, "test.previous.jsonl", 120, &second);

        let rotated = dir.join("test.previous.jsonl");
        let current_text = fs::read_to_string(&path).expect("read current file");
        let rotated_text = fs::read_to_string(&rotated).expect("read rotated file");

        assert!(current_text.contains(&"b".repeat(20)));
        assert!(rotated_text.contains(&"a".repeat(20)));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn append_bounded_jsonl_record_discards_pathological_oversized_file() {
        let dir = temp_test_dir("oversized");
        let path = dir.join("test.jsonl");
        fs::write(&path, "x".repeat(512)).expect("seed oversized file");

        append_bounded_jsonl_record(&path, "test.previous.jsonl", 120, &json!({ "ok": true }));

        let current_text = fs::read_to_string(&path).expect("read current file");
        let rotated = dir.join("test.previous.jsonl");
        assert!(current_text.contains("\"ok\":true"));
        assert!(!rotated.exists());

        let _ = fs::remove_dir_all(dir);
    }
}
