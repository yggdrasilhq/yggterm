use serde_json::{Value, json};
use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const PERF_TELEMETRY_FILENAME: &str = "perf-telemetry.jsonl";

pub fn perf_telemetry_path(home: &Path) -> PathBuf {
    home.join(PERF_TELEMETRY_FILENAME)
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
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{event}");
    }
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
