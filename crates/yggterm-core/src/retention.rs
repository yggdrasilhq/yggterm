//! Generational retention for the append-only JSONL diagnostic streams
//! (event-trace, ui-telemetry, perf-telemetry).
//!
//! The old scheme kept exactly one `.previous.jsonl` per stream, so total
//! coverage was 2x the live cap — ~13 hours on a busy day, far too short to
//! correlate sporadic incidents (agent resume UUID conflicts, the working-dot
//! lag) across sessions. Rather than one giant file (slow scans, unbounded
//! growth), a full live file is renamed to a timestamped GENERATION
//! (`<stem>.g<ts_ms>.jsonl`) and generations are pruned by BOTH rules:
//!   - age: anything older than the cap (3 days) is deleted, even if small —
//!     the window is "at most 3 days", not "at least"
//!   - total size: oldest generations go first once the stream's byte budget
//!     is exceeded, so a pathological flood cannot eat the disk
//! Pruning runs only at rotation time (every ~8-16 MiB written) plus once at
//! the first write per process, so the per-event I/O cost is unchanged: one
//! append. The legacy single `.previous.jsonl` file is treated as a generation
//! (aged by mtime) so it drains out of existence on its own.

use serde_json::Value;
use std::fs::{self, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Hard age cap for rotated diagnostic generations: at most 3 days.
pub const DIAGNOSTIC_RETENTION_MAX_AGE_MS: u128 = 3 * 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug)]
pub struct JsonlRetention {
    /// Rotate the live file into a generation once it reaches this size.
    pub live_max_bytes: u64,
    /// Total byte budget across rotated generations (live file not counted).
    pub generations_max_bytes: u64,
    /// Delete generations older than this. Almost always
    /// [`DIAGNOSTIC_RETENTION_MAX_AGE_MS`]; a field so tests can shrink it.
    pub max_age_ms: u128,
}

pub fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// `event-trace.jsonl` -> `event-trace.g<ts_ms>.jsonl` next to it.
fn generation_path(path: &Path, ts_ms: u128) -> PathBuf {
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.strip_suffix(".jsonl").unwrap_or(name))
        .unwrap_or("diagnostics");
    path.with_file_name(format!("{stem}.g{ts_ms}.jsonl"))
}

/// One rotated generation on disk: its path, birth timestamp, and size.
struct Generation {
    path: PathBuf,
    ts_ms: u128,
    bytes: u64,
}

fn list_generations(path: &Path) -> Vec<Generation> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let Some(stem) = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.strip_suffix(".jsonl").unwrap_or(name))
    else {
        return Vec::new();
    };
    let generation_prefix = format!("{stem}.g");
    let legacy_name = format!("{stem}.previous.jsonl");
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut generations = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let ts_ms = if let Some(ts_text) = name
            .strip_prefix(&generation_prefix)
            .and_then(|rest| rest.strip_suffix(".jsonl"))
        {
            let Ok(ts_ms) = ts_text.parse::<u128>() else {
                continue;
            };
            ts_ms
        } else if name == legacy_name {
            // The pre-generation single rotated file: age it by mtime so it
            // drains under the same rules instead of living forever.
            entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis())
                .unwrap_or_default()
        } else {
            continue;
        };
        let bytes = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        generations.push(Generation {
            path: entry.path(),
            ts_ms,
            bytes,
        });
    }
    generations
}

/// Every on-disk file of a stream in chronological order: rotated generations
/// (plus the legacy `.previous.jsonl`, if any) oldest first, then the live
/// file. For readers that scan history (desktop-identity lookup, scripts).
pub fn jsonl_read_paths(path: &Path) -> Vec<PathBuf> {
    let mut generations = list_generations(path);
    generations.sort_by_key(|generation| generation.ts_ms);
    let mut paths: Vec<PathBuf> = generations
        .into_iter()
        .map(|generation| generation.path)
        .collect();
    paths.push(path.to_path_buf());
    paths
}

/// Delete generations that violate the age cap or, oldest first, the total
/// byte budget. Called at rotation and once per process on first write.
pub fn prune_jsonl_generations(path: &Path, retention: JsonlRetention, now_ms: u128) {
    let mut generations = list_generations(path);
    generations.sort_by_key(|generation| generation.ts_ms);
    let mut total_bytes: u64 = generations.iter().map(|generation| generation.bytes).sum();
    for generation in &generations {
        let expired = now_ms.saturating_sub(generation.ts_ms) > retention.max_age_ms;
        let over_budget = total_bytes > retention.generations_max_bytes;
        if !expired && !over_budget {
            break;
        }
        if fs::remove_file(&generation.path).is_ok() {
            total_bytes = total_bytes.saturating_sub(generation.bytes);
        }
    }
}

/// Rotate the live file into a fresh generation if it reached the cap, then
/// prune. Returns true when a rotation happened (callers holding an open
/// handle must reopen).
pub fn rotate_jsonl_with_retention(path: &Path, retention: JsonlRetention, now_ms: u128) -> bool {
    let live_bytes = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let mut rotated = false;
    if live_bytes >= retention.live_max_bytes {
        rotated = fs::rename(path, generation_path(path, now_ms)).is_ok();
    }
    prune_jsonl_generations(path, retention, now_ms);
    rotated
}

/// Append one JSON record to a stream governed by generational retention.
/// Open-per-call variant for the low-frequency writers (ui-telemetry,
/// perf-telemetry); the event-trace hot path keeps its cached handle and
/// drives rotation itself via [`rotate_jsonl_with_retention`].
pub fn append_retained_jsonl_record(path: &Path, retention: JsonlRetention, record: &Value) {
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = create_dir_all(parent);
    let Ok(mut line) = serde_json::to_vec(record) else {
        return;
    };
    line.push(b'\n');
    let live_bytes = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if live_bytes > 0 && live_bytes.saturating_add(line.len() as u64) > retention.live_max_bytes {
        let now_ms = now_epoch_ms();
        let _ = fs::rename(path, generation_path(path, now_ms));
        prune_jsonl_generations(path, retention, now_ms);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("ygg-retention-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn retention(live: u64, total: u64, age_ms: u128) -> JsonlRetention {
        JsonlRetention {
            live_max_bytes: live,
            generations_max_bytes: total,
            max_age_ms: age_ms,
        }
    }

    #[test]
    fn rotation_moves_live_file_into_timestamped_generation() {
        let dir = temp_dir("rotate");
        let live = dir.join("stream.jsonl");
        fs::write(&live, vec![b'x'; 64]).unwrap();
        assert!(rotate_jsonl_with_retention(&live, retention(64, 1024, 1_000), 42));
        assert!(!live.exists());
        let generation = dir.join("stream.g42.jsonl");
        assert!(generation.exists(), "expected {generation:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_deletes_generations_older_than_the_age_cap_even_when_small() {
        let dir = temp_dir("age");
        let live = dir.join("stream.jsonl");
        fs::write(dir.join("stream.g100.jsonl"), b"old").unwrap();
        fs::write(dir.join("stream.g900.jsonl"), b"new").unwrap();
        prune_jsonl_generations(&live, retention(64, 1024, 500), 1_000);
        assert!(!dir.join("stream.g100.jsonl").exists(), "expired must go");
        assert!(dir.join("stream.g900.jsonl").exists(), "fresh must stay");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_deletes_oldest_first_when_over_the_byte_budget() {
        let dir = temp_dir("budget");
        let live = dir.join("stream.jsonl");
        fs::write(dir.join("stream.g100.jsonl"), vec![b'a'; 60]).unwrap();
        fs::write(dir.join("stream.g200.jsonl"), vec![b'b'; 60]).unwrap();
        fs::write(dir.join("stream.g300.jsonl"), vec![b'c'; 60]).unwrap();
        // 180 bytes on disk, budget 130: only the oldest generation must go.
        prune_jsonl_generations(&live, retention(64, 130, u128::MAX), 1_000);
        assert!(!dir.join("stream.g100.jsonl").exists());
        assert!(dir.join("stream.g200.jsonl").exists());
        assert!(dir.join("stream.g300.jsonl").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_previous_file_is_pruned_by_mtime() {
        let dir = temp_dir("legacy");
        let live = dir.join("stream.jsonl");
        fs::write(dir.join("stream.previous.jsonl"), b"legacy").unwrap();
        // An mtime of "now" is far younger than a huge age cap -> stays.
        prune_jsonl_generations(&live, retention(64, 1024, u128::MAX), now_epoch_ms());
        assert!(dir.join("stream.previous.jsonl").exists());
        // With a zero age cap it counts as expired -> goes.
        prune_jsonl_generations(&live, retention(64, 1024, 0), now_epoch_ms() + 10);
        assert!(!dir.join("stream.previous.jsonl").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn append_retained_rotates_before_the_live_file_would_overflow() {
        let dir = temp_dir("append");
        let live = dir.join("stream.jsonl");
        let first = json!({ "message": "a".repeat(90) });
        let second = json!({ "message": "b".repeat(90) });
        let policy = retention(120, 4096, u128::MAX);
        append_retained_jsonl_record(&live, policy, &first);
        append_retained_jsonl_record(&live, policy, &second);
        let generations = list_generations(&live);
        assert_eq!(generations.len(), 1, "first record must be rotated out");
        let rotated_text = fs::read_to_string(&generations[0].path).unwrap();
        assert!(rotated_text.contains(&"a".repeat(20)));
        let live_text = fs::read_to_string(&live).unwrap();
        assert!(live_text.contains(&"b".repeat(20)));
        let _ = fs::remove_dir_all(dir);
    }
}
