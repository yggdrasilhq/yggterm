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
    jsonl_read_paths_since(path, None)
}

/// Slack applied when deciding whether a rotated generation can hold a record
/// inside a window. A generation is named for the wall-clock instant its live
/// file was RENAMED, so every record it holds was appended before that instant
/// — but several processes `O_APPEND` into the same live file, so a writer that
/// stamped `ts_ms` just before a rotation can land its line in the generation
/// that was named a few milliseconds earlier. A minute of slack swallows that
/// race (and any small clock adjustment) with room to spare.
pub const JSONL_WINDOW_SLACK_MS: u64 = 60_000;

/// The files of a stream that can contain records at or after `since_ms`,
/// oldest first, live file last. `None` means "the whole stream".
///
/// **This is the one owner of "which files does a windowed read have to open".**
/// It exists because the perf-incident monitor asked `summarize_perf_telemetry`
/// a question about the last 60 SECONDS every 30 s and paid a full read of the
/// retained corpus for it — 104.6 MiB on the live host against a 144 MiB cap,
/// in EVERY daemon, measured as 312.9 MB of `rchar` per 90 s per daemon
/// (2026-07-26). A generation's own filename already carries the newest instant
/// it can hold, so the decision needs no read at all.
pub fn jsonl_read_paths_since(path: &Path, since_ms: Option<u64>) -> Vec<PathBuf> {
    let mut generations = list_generations(path);
    generations.sort_by_key(|generation| generation.ts_ms);
    let floor =
        since_ms.map(|since| u128::from(since).saturating_sub(u128::from(JSONL_WINDOW_SLACK_MS)));
    let mut paths: Vec<PathBuf> = generations
        .into_iter()
        .filter(|generation| floor.is_none_or(|floor| generation.ts_ms >= floor))
        .map(|generation| generation.path)
        .collect();
    paths.push(path.to_path_buf());
    paths
}

/// An UPPER BOUND on a raw JSONL line's own `ts_ms`, read from the bytes
/// without parsing the line.
///
/// Every `"ts_ms":<digits>` occurrence in the line is a candidate; the record's
/// own is one of them, so the maximum is never below the true value. A nested
/// occurrence inside a payload can only raise the bound, never lower it — which
/// is what makes "bound < since ⇒ the record is out of window" sound regardless
/// of key ordering (`serde_json`'s map is a `BTreeMap`, so `ts_ms` sorts LAST in
/// a perf event) and regardless of what a caller stuffed into `payload`.
/// `None` means "no bound available" — the caller must parse.
pub fn jsonl_line_ts_ms_upper_bound(line: &str) -> Option<u64> {
    const NEEDLE: &str = "\"ts_ms\":";
    let mut best: Option<u64> = None;
    let mut rest = line;
    while let Some(at) = rest.find(NEEDLE) {
        let after = &rest[at + NEEDLE.len()..];
        let digits: String = after
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(value) = digits.parse::<u64>() {
            best = Some(best.map_or(value, |best: u64| best.max(value)));
        }
        rest = after;
    }
    best
}

/// Visit every record of a generational JSONL stream whose `ts_ms` is at or
/// after `since_ms`, oldest file first, in file order within each file.
///
/// The one owner of the windowed read: it picks the files
/// ([`jsonl_read_paths_since`]), skips lines the raw bytes already rule out
/// ([`jsonl_line_ts_ms_upper_bound`]) so `serde_json` is never paid for them,
/// and applies the authoritative `ts_ms` filter on the parsed record. Callers
/// that re-implement any of those three steps re-introduce the corpus-wide read
/// this replaced.
pub fn for_each_jsonl_record_since(
    path: &Path,
    since_ms: Option<u64>,
    mut visit: impl FnMut(Value),
) {
    for file in jsonl_read_paths_since(path, since_ms) {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(since) = since_ms
                && jsonl_line_ts_ms_upper_bound(line).is_some_and(|bound| bound < since)
            {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(since) = since_ms
                && record.get("ts_ms").and_then(Value::as_u64).unwrap_or(0) < since
            {
                continue;
            }
            visit(record);
        }
    }
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
        let dir = std::env::temp_dir().join(format!(
            "ygg-retention-{tag}-{}-{nanos}",
            std::process::id()
        ));
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
        assert!(rotate_jsonl_with_retention(
            &live,
            retention(64, 1024, 1_000),
            42
        ));
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

    #[test]
    fn windowed_read_paths_skip_generations_that_closed_before_the_window() {
        let dir = temp_dir("window-paths");
        let live = dir.join("stream.jsonl");
        for ts in [1_000_000u64, 2_000_000, 3_000_000] {
            fs::write(dir.join(format!("stream.g{ts}.jsonl")), b"{}\n").unwrap();
        }
        // Window opens at 2_500_000. g1_000_000 and g2_000_000 both closed
        // before it (even allowing the slack) and cannot hold a record in it.
        let picked = jsonl_read_paths_since(&live, Some(2_500_000));
        assert_eq!(
            picked,
            vec![dir.join("stream.g3000000.jsonl"), live.clone()],
            "only the spanning generation and the live file may be opened"
        );
        // The generation that STRADDLES the boundary must survive: its name is
        // the instant it closed, so a window opening just before it contains
        // part of it.
        let picked = jsonl_read_paths_since(&live, Some(2_000_000));
        assert!(
            picked.contains(&dir.join("stream.g2000000.jsonl")),
            "the boundary generation must still be read: {picked:?}"
        );
        // Slack is honoured: a generation that closed inside the slack margin
        // is kept even though its name sorts before the window.
        let picked = jsonl_read_paths_since(&live, Some(2_000_000 + JSONL_WINDOW_SLACK_MS - 1));
        assert!(picked.contains(&dir.join("stream.g2000000.jsonl")));
        // No window means the whole stream, unchanged.
        assert_eq!(jsonl_read_paths_since(&live, None), jsonl_read_paths(&live));
        assert_eq!(jsonl_read_paths(&live).len(), 4);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn raw_line_ts_bound_is_never_below_the_records_own_ts() {
        // `serde_json`'s map is a `BTreeMap`, so `ts_ms` serializes LAST.
        let line = serde_json::to_string(&json!({
            "ts_ms": 500u64,
            "category": "render",
            "name": "gui",
            "payload": { "duration_ms": 1.5 },
        }))
        .unwrap();
        assert_eq!(jsonl_line_ts_ms_upper_bound(&line), Some(500));
        // A nested `ts_ms` inside a caller-supplied payload may only RAISE the
        // bound — lowering it would silently drop an in-window record.
        let nested = serde_json::to_string(&json!({
            "ts_ms": 500u64,
            "category": "daemon",
            "name": "chore",
            "payload": { "ts_ms": 9_000u64, "duration_ms": 1.0 },
        }))
        .unwrap();
        assert!(jsonl_line_ts_ms_upper_bound(&nested).unwrap() >= 500);
        // Written by hand, key order NOT serde's: the bound must not depend on
        // where in the line the record's own `ts_ms` happens to sit. Reading
        // the LAST occurrence here would return 1 and silently drop a record
        // that is inside the window.
        assert_eq!(
            jsonl_line_ts_ms_upper_bound(r#"{"ts_ms":500,"payload":{"ts_ms":1}}"#),
            Some(500),
            "an older nested ts_ms must not lower the bound"
        );
        assert_eq!(
            jsonl_line_ts_ms_upper_bound(r#"{"payload":{"ts_ms":1},"ts_ms":500}"#),
            Some(500),
            "the bound must be order-independent"
        );
        assert_eq!(jsonl_line_ts_ms_upper_bound("{\"category\":\"x\"}"), None);
    }

    #[test]
    fn windowed_record_read_keeps_in_window_rows_from_a_straddling_generation() {
        let dir = temp_dir("window-records");
        let live = dir.join("stream.jsonl");
        let row = |ts: u64, name: &str| {
            format!(
                "{}\n",
                serde_json::to_string(&json!({ "ts_ms": ts, "name": name })).unwrap()
            )
        };
        // A generation named 2_000_000 holds rows from before AND after the
        // window opens at 1_900_000 — a reader that skipped it to "get faster"
        // would lose `in_generation`.
        fs::write(
            dir.join("stream.g2000000.jsonl"),
            format!(
                "{}{}",
                row(1_000_000, "too_old"),
                row(1_950_000, "in_generation")
            ),
        )
        .unwrap();
        fs::write(&live, row(2_100_000, "in_live")).unwrap();
        let mut names = Vec::new();
        for_each_jsonl_record_since(&live, Some(1_900_000), |record| {
            names.push(
                record
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
        });
        assert_eq!(names, vec!["in_generation", "in_live"]);
        let _ = fs::remove_dir_all(dir);
    }

    /// The DAEMON-1 defect itself, measured the way it was measured live: the
    /// bytes the process reads. `summarize_perf_telemetry` asked a question
    /// about the last 60 SECONDS and paid a read of the whole retained corpus
    /// for it, every 30 s, in every daemon — 312.9 MB of `rchar` per 90 s on
    /// guihost (2026-07-26). A correctness test cannot see this; only the byte
    /// counter can.
    #[cfg(target_os = "linux")]
    #[test]
    fn windowed_read_does_not_pay_for_generations_outside_the_window() {
        fn rchar() -> u64 {
            fs::read_to_string("/proc/self/io")
                .ok()
                .and_then(|text| {
                    text.lines()
                        .find_map(|line| line.strip_prefix("rchar: "))
                        .and_then(|value| value.trim().parse::<u64>().ok())
                })
                .unwrap_or(0)
        }

        let dir = temp_dir("window-rchar");
        let live = dir.join("stream.jsonl");
        // Three "rotated generations" of ~1 MiB each, all closed long before
        // the window, plus a small live file inside it.
        let filler: String = (0..4_000u64)
            .map(|i| {
                format!(
                    "{}\n",
                    serde_json::to_string(&json!({
                        "ts_ms": 1_000_000u64 + i,
                        "category": "daemon_request",
                        "name": "status",
                        "payload": { "duration_ms": 0.1, "pad": "x".repeat(200) },
                    }))
                    .unwrap()
                )
            })
            .collect();
        for ts in [1_100_000u64, 1_200_000, 1_300_000] {
            fs::write(dir.join(format!("stream.g{ts}.jsonl")), &filler).unwrap();
        }
        let corpus: u64 = [1_100_000u64, 1_200_000, 1_300_000]
            .iter()
            .map(|ts| {
                fs::metadata(dir.join(format!("stream.g{ts}.jsonl")))
                    .map(|meta| meta.len())
                    .unwrap_or(0)
            })
            .sum();
        assert!(corpus > 2 * 1024 * 1024, "corpus too small: {corpus} bytes");
        fs::write(
            &live,
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "ts_ms": 9_000_000u64,
                    "category": "render",
                    "name": "gui",
                    "payload": { "duration_ms": 4.0 },
                }))
                .unwrap()
            ),
        )
        .unwrap();

        let before = rchar();
        let mut seen = 0usize;
        for_each_jsonl_record_since(&live, Some(8_999_000), |_| seen += 1);
        let read_bytes = rchar().saturating_sub(before);
        assert_eq!(seen, 1, "the in-window record must still arrive");
        assert!(
            read_bytes < corpus / 4,
            "the windowed read touched {read_bytes} bytes of a {corpus}-byte \
             out-of-window corpus — this is the whole-corpus re-read that cost \
             312.9 MB per 90 s per daemon on guihost"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// The same measurement against a REAL corpus, because a synthetic one can
    /// always be accused of being shaped to pass. Point it at a copy of a live
    /// `~/.yggterm` (a plain read — never the host's own directory, the test
    /// must not race a writer) and it prints the bytes a 60 s incident-monitor
    /// window costs:
    ///
    /// ```sh
    /// rsync -a <host>:'~/.yggterm/perf-telemetry*.jsonl' /tmp/corpus/
    /// YGGTERM_PERF_REPLAY_DIR=/tmp/corpus cargo test -p yggterm-core \
    ///     --lib replay_a_real_perf_corpus -- --ignored --nocapture
    /// ```
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs YGGTERM_PERF_REPLAY_DIR pointing at a copied corpus"]
    fn replay_a_real_perf_corpus_reports_the_bytes_one_window_costs() {
        let Some(dir) = std::env::var_os("YGGTERM_PERF_REPLAY_DIR").map(PathBuf::from) else {
            panic!("set YGGTERM_PERF_REPLAY_DIR to a directory holding perf-telemetry*.jsonl");
        };
        let live = dir.join(crate::perf::PERF_TELEMETRY_FILENAME);
        let corpus: u64 = jsonl_read_paths(&live)
            .iter()
            .filter_map(|path| fs::metadata(path).ok().map(|meta| meta.len()))
            .sum();
        let newest = jsonl_read_paths(&live)
            .iter()
            .filter_map(|path| {
                fs::read_to_string(path)
                    .ok()
                    .and_then(|text| text.lines().filter_map(jsonl_line_ts_ms_upper_bound).max())
            })
            .max()
            .expect("corpus must hold at least one timestamped record");
        // The incident monitor's actual question: the last 60 seconds.
        let since = newest.saturating_sub(60_000);
        let rchar = || -> u64 {
            fs::read_to_string("/proc/self/io")
                .ok()
                .and_then(|text| {
                    text.lines()
                        .find_map(|line| line.strip_prefix("rchar: "))
                        .and_then(|value| value.trim().parse::<u64>().ok())
                })
                .unwrap_or(0)
        };
        let before = rchar();
        let mut seen = 0usize;
        for_each_jsonl_record_since(&live, Some(since), |_| seen += 1);
        let read_bytes = rchar().saturating_sub(before);
        println!(
            "corpus={corpus} bytes across {} files; one 60 s window read {read_bytes} bytes \
             and yielded {seen} records ({:.1}% of the corpus)",
            jsonl_read_paths(&live).len(),
            100.0 * read_bytes as f64 / corpus as f64
        );
        assert!(
            seen > 0,
            "a 60 s window at the corpus head must hold records"
        );
        assert!(
            read_bytes < corpus / 4,
            "read {read_bytes} of {corpus} bytes for a 60 s question"
        );
    }
}
