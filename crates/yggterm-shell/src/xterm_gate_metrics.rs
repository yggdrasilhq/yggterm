// Per [[spec-xterm-gating-ux]] step 1: "Honest optimization first. Measure where
// time-in-gate actually goes ... Remove the actual bottleneck."
//
// Each xterm gate has a begin (arm) and finish (clear) callsite. This module
// owns the bounded per-kind histogram and the app-state JSON shape so any gate
// can opt in by calling `GateMetrics::record_clear(kind, duration_ms)` once.
//
// Out of scope: arming/clearing the gate itself, the user-facing notification,
// and the polling-vs-event-driven debate. Those stay with the gate's owner.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const GATE_DURATION_RING_CAPACITY: usize = 64;

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub(crate) enum GateKind {
    /// Wait for the current daemon endpoint to be reachable before a retained
    /// rehydrate. Lifecycle: `begin_retained_rehydrate_daemon_ready_wait` /
    /// `finish_retained_rehydrate_daemon_ready_wait` (shell.rs).
    RetainedRehydrateDaemonReadyWait,
}

impl GateKind {
    fn as_str(self) -> &'static str {
        match self {
            GateKind::RetainedRehydrateDaemonReadyWait => "retained_rehydrate_daemon_ready_wait",
        }
    }
}

#[derive(Clone, Default)]
struct GateDurationRing {
    samples: Vec<u64>,
    write_idx: usize,
    total_samples: u64,
    last_clear_ms: Option<u64>,
}

impl GateDurationRing {
    fn record(&mut self, duration_ms: u64, clear_ts_ms: u64) {
        if self.samples.len() < GATE_DURATION_RING_CAPACITY {
            self.samples.push(duration_ms);
        } else {
            self.samples[self.write_idx] = duration_ms;
            self.write_idx = (self.write_idx + 1) % GATE_DURATION_RING_CAPACITY;
        }
        self.total_samples = self.total_samples.saturating_add(1);
        self.last_clear_ms = Some(clear_ts_ms);
    }

    fn percentile(&self, pct: f64) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let rank = (pct * (sorted.len() as f64 - 1.0)).round() as usize;
        sorted.get(rank).copied()
    }

    fn max(&self) -> Option<u64> {
        self.samples.iter().copied().max()
    }
}

#[derive(Clone, Default)]
pub(crate) struct GateMetrics {
    rings: HashMap<GateKind, GateDurationRing>,
}

impl GateMetrics {
    pub(crate) fn record_clear(&mut self, kind: GateKind, duration_ms: u64, clear_ts_ms: u64) {
        self.rings
            .entry(kind)
            .or_default()
            .record(duration_ms, clear_ts_ms);
    }

    /// Snapshot of per-kind stats for the app-state probe. Stable JSON shape
    /// because consumers (skill, smoke tests, dashboards) key off it.
    pub(crate) fn to_app_state_json(&self) -> Value {
        let mut entries: Vec<Value> = self
            .rings
            .iter()
            .map(|(kind, ring)| {
                json!({
                    "gate_kind": kind.as_str(),
                    "sample_count": ring.samples.len(),
                    "total_samples": ring.total_samples,
                    "ring_capacity": GATE_DURATION_RING_CAPACITY,
                    "p50_ms": ring.percentile(0.5),
                    "p95_ms": ring.percentile(0.95),
                    "max_ms": ring.max(),
                    "last_clear_ts_ms": ring.last_clear_ms,
                })
            })
            .collect();
        entries.sort_by(|a, b| {
            a["gate_kind"]
                .as_str()
                .unwrap_or("")
                .cmp(b["gate_kind"].as_str().unwrap_or(""))
        });
        Value::Array(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_track_ordered_samples() {
        let mut metrics = GateMetrics::default();
        for ms in [100, 200, 300, 400, 500] {
            metrics.record_clear(GateKind::RetainedRehydrateDaemonReadyWait, ms, ms + 1000);
        }
        let snapshot = metrics.to_app_state_json();
        let entry = &snapshot.as_array().unwrap()[0];
        assert_eq!(entry["sample_count"], 5);
        assert_eq!(entry["p50_ms"], 300);
        assert_eq!(entry["p95_ms"], 500);
        assert_eq!(entry["max_ms"], 500);
        assert_eq!(entry["last_clear_ts_ms"], 1500);
    }

    #[test]
    fn ring_wraps_at_capacity_but_preserves_total() {
        let mut metrics = GateMetrics::default();
        for ms in 0..(GATE_DURATION_RING_CAPACITY as u64 + 10) {
            metrics.record_clear(GateKind::RetainedRehydrateDaemonReadyWait, ms, ms);
        }
        let snapshot = metrics.to_app_state_json();
        let entry = &snapshot.as_array().unwrap()[0];
        assert_eq!(entry["sample_count"], GATE_DURATION_RING_CAPACITY);
        assert_eq!(
            entry["total_samples"].as_u64().unwrap(),
            GATE_DURATION_RING_CAPACITY as u64 + 10
        );
        // Oldest 10 samples evicted; max stays at the highest pushed value.
        assert_eq!(
            entry["max_ms"].as_u64().unwrap(),
            GATE_DURATION_RING_CAPACITY as u64 + 9
        );
    }

    #[test]
    fn empty_metrics_serialize_as_empty_array() {
        let metrics = GateMetrics::default();
        assert_eq!(metrics.to_app_state_json(), json!([]));
    }
}
