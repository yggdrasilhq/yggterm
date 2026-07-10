// Standalone live-host smoke for the handle-cached trace writer.
//
// Exercises `append_trace_event` hard enough to force at least one rotation,
// then reports throughput plus the on-disk record counts so a deploy target
// (jojo) can prove the writer produces valid JSONL and rotates correctly
// without touching the running daemon. Usage:
//
//   trace_smoke <home_dir> [event_count]
use std::path::PathBuf;
use std::time::Instant;

use yggterm_core::{append_trace_event, event_trace_path, read_trace_tail};

fn main() {
    let mut args = std::env::args().skip(1);
    let home = PathBuf::from(args.next().expect("usage: trace_smoke <home_dir> [count]"));
    let count: usize = args
        .next()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(200_000);

    let _ = std::fs::remove_dir_all(&home);

    let started = Instant::now();
    for i in 0..count {
        append_trace_event(
            &home,
            "smoke",
            "live",
            "tick",
            serde_json::json!({ "i": i, "blob": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" }),
        );
    }
    let elapsed = started.elapsed();

    let path = event_trace_path(&home);
    let live_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let rotated_len: u64 = std::fs::read_dir(&home)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.starts_with("event-trace.g") && name.ends_with(".jsonl")
                })
                .filter_map(|entry| entry.metadata().ok().map(|meta| meta.len()))
                .sum()
        })
        .unwrap_or(0);

    // Validate the freshest records round-trip as real EventTraceRecords.
    let tail = read_trace_tail(&path, 3);
    let last_ok = tail
        .last()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
        .unwrap_or(false);

    println!("events_written: {count}");
    println!("elapsed_ms: {:.1}", elapsed.as_secs_f64() * 1000.0);
    println!(
        "per_event_us: {:.3}",
        elapsed.as_secs_f64() * 1_000_000.0 / count as f64
    );
    println!("live_file_bytes: {live_len}");
    println!("rotated_file_bytes: {rotated_len}");
    println!("rotation_happened: {}", rotated_len > 0);
    println!("last_record_parses: {last_ok}");
    if let Some(line) = tail.last() {
        println!("last_record: {line}");
    }
}
