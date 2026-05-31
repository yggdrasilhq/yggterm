//! Automations — scheduled, app-control-driven agent-CLI sessions.
//!
//! An [`Automation`] re-prompts a single linked keep-alive session on a cadence
//! (cron/systemd-timer-like), driven through the same session-open + send path
//! agents use via yggui app control. The session it drives carries an
//! `automation_id` back-reference, which is the SSOT for whether a session
//! shows in the **Automated Sessions** group vs **Live Sessions** — the cwd
//! tree never moves. See `docs/automations.md`.
//!
//! Scheduling is **deterministic** (no-non-determinism rule): the next run time
//! is computed once into `next_run_at_ms` and stored; the scheduler only checks
//! `now >= next_run_at_ms`. Jitter is a seeded offset derived from the
//! automation id + run window, so recomputing it yields the same value — it is
//! never re-rolled per scheduler tick.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::SessionKind;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Filename for the automations store, under the daemon home dir
/// (`~/.yggterm/`). Kept separate from `server-state.json` — automations are a
/// distinct concern from per-session runtime state.
pub const AUTOMATIONS_FILE: &str = "automations.json";

/// Path to the automations store for a daemon home dir.
pub fn automations_path(home_dir: &Path) -> PathBuf {
    home_dir.join(AUTOMATIONS_FILE)
}

/// Load automations from `~/.yggterm/automations.json`. A missing file is an
/// empty set (first run); a corrupt file is logged-as-empty by the caller's
/// choice — here it returns the parse error so the caller can decide.
pub fn load_automations(home_dir: &Path) -> std::io::Result<Vec<Automation>> {
    let path = automations_path(home_dir);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Persist automations to `~/.yggterm/automations.json` atomically
/// (write-temp-then-rename) so a crash mid-write can't corrupt the store.
pub fn save_automations(home_dir: &Path, automations: &[Automation]) -> std::io::Result<()> {
    let path = automations_path(home_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(automations)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)
}

/// How often an automation fires. Monthly is approximated as 30 days
/// (exact-calendar-month support is a future refinement; determinism is the
/// invariant that matters here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationCadence {
    Daily,
    Weekly,
    Monthly,
    EveryNDays { days: u32 },
}

impl AutomationCadence {
    /// Base interval between runs, in milliseconds.
    pub fn base_interval_ms(self) -> u64 {
        match self {
            AutomationCadence::Daily => DAY_MS,
            AutomationCadence::Weekly => 7 * DAY_MS,
            AutomationCadence::Monthly => 30 * DAY_MS,
            AutomationCadence::EveryNDays { days } => u64::from(days.max(1)) * DAY_MS,
        }
    }
}

/// A scheduled automation. Holds the schedule definition plus a back-link to the
/// single keep-alive session it re-prompts each cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Automation {
    pub id: String,
    /// Which agent CLI the linked session runs (Codex / Claude Code / …).
    pub agent_kind: SessionKind,
    pub machine_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The prompt sent to the session on each fire (e.g. "some time has passed,
    /// can you upgrade again").
    pub prompt: String,
    pub cadence: AutomationCadence,
    /// Max ± random jitter in days applied to each computed `next_run_at_ms`
    /// (the user's "± random days"). 0 = exact cadence.
    #[serde(default)]
    pub jitter_days: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at_ms: Option<u64>,
    pub next_run_at_ms: u64,
    /// The keep-alive session this automation re-prompts. `None` until the first
    /// fire creates/links it. The session also stores this automation's `id`,
    /// which places it in the Automated Sessions group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_session_id: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Stable FNV-1a hash of the bytes, mixed with `salt` — used only to derive a
/// reproducible jitter offset (NOT for security).
fn seeded_hash(bytes: &[u8], salt: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64; // FNV offset basis
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3); // FNV prime
    }
    hash ^= salt;
    hash.wrapping_mul(0x0000_0100_0000_01B3)
}

/// Compute the next run time deterministically: `after_ms` + one cadence
/// interval, offset by a reproducible ± jitter bounded by `jitter_days`. The
/// result is always strictly after `after_ms`. Pure + deterministic so the same
/// inputs always yield the same instant (the value is computed once and stored
/// in `next_run_at_ms`; the scheduler never re-rolls it).
pub fn compute_next_run_at_ms(automation: &Automation, after_ms: u64) -> u64 {
    let base = automation.cadence.base_interval_ms();
    let target = after_ms.saturating_add(base);
    if automation.jitter_days == 0 {
        return target;
    }
    let span_ms = u64::from(automation.jitter_days) * DAY_MS;
    // Salt with the run window so successive runs get different (but stable)
    // jitter rather than the same offset every cycle.
    let window = target / base.max(1);
    let hash = seeded_hash(automation.id.as_bytes(), window);
    let offset = (hash % (2 * span_ms + 1)) as i64 - span_ms as i64;
    let jittered = target as i64 + offset;
    // Never schedule in the past relative to `after_ms`.
    jittered.max(after_ms as i64 + 1) as u64
}

/// Whether the automation should fire now.
pub fn automation_is_due(automation: &Automation, now_ms: u64) -> bool {
    automation.enabled && now_ms >= automation.next_run_at_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(jitter_days: u32, cadence: AutomationCadence) -> Automation {
        Automation {
            id: "auto-infra-upgrade".to_string(),
            agent_kind: SessionKind::Codex,
            machine_key: "jojo".to_string(),
            cwd: Some("/home/pi/gh/yggterm".to_string()),
            prompt: "some time has passed, can you upgrade again".to_string(),
            cadence,
            jitter_days,
            enabled: true,
            created_at_ms: 1_000_000,
            last_run_at_ms: None,
            next_run_at_ms: 0,
            linked_session_id: None,
        }
    }

    #[test]
    fn next_run_without_jitter_is_exactly_one_interval() {
        let a = sample(0, AutomationCadence::Weekly);
        assert_eq!(
            compute_next_run_at_ms(&a, 10 * DAY_MS),
            10 * DAY_MS + 7 * DAY_MS
        );
    }

    #[test]
    fn next_run_is_deterministic_for_same_inputs() {
        let a = sample(5, AutomationCadence::Monthly);
        let first = compute_next_run_at_ms(&a, 100 * DAY_MS);
        let again = compute_next_run_at_ms(&a, 100 * DAY_MS);
        assert_eq!(first, again, "jitter must be reproducible, never re-rolled");
    }

    #[test]
    fn jitter_is_bounded_by_jitter_days_and_in_the_future() {
        let a = sample(3, AutomationCadence::Monthly);
        let after = 365 * DAY_MS;
        let next = compute_next_run_at_ms(&a, after);
        let target = after + 30 * DAY_MS;
        let span = 3 * DAY_MS;
        assert!(next >= target - span && next <= target + span, "within ±jitter");
        assert!(next > after, "always strictly in the future");
    }

    #[test]
    fn successive_windows_get_different_jitter() {
        // Different `after_ms` (different run windows) should generally produce
        // different jitter offsets, not the same fixed shift every cycle.
        let a = sample(4, AutomationCadence::Weekly);
        let n1 = compute_next_run_at_ms(&a, 7 * DAY_MS) as i64 - (7 * DAY_MS + 7 * DAY_MS) as i64;
        let n2 = compute_next_run_at_ms(&a, 14 * DAY_MS) as i64 - (14 * DAY_MS + 7 * DAY_MS) as i64;
        let n3 = compute_next_run_at_ms(&a, 21 * DAY_MS) as i64 - (21 * DAY_MS + 7 * DAY_MS) as i64;
        assert!(
            !(n1 == n2 && n2 == n3),
            "jitter should vary across run windows (got {n1},{n2},{n3})"
        );
    }

    #[test]
    fn due_respects_enabled_and_next_run() {
        let mut a = sample(0, AutomationCadence::Daily);
        a.next_run_at_ms = 5_000;
        assert!(!automation_is_due(&a, 4_999));
        assert!(automation_is_due(&a, 5_000));
        a.enabled = false;
        assert!(!automation_is_due(&a, 10_000), "disabled never fires");
    }

    #[test]
    fn save_then_load_roundtrips_and_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("ygg-auto-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Missing file => empty set (first run).
        assert!(load_automations(&dir).unwrap().is_empty());
        let mut a = sample(5, AutomationCadence::Monthly);
        a.linked_session_id = Some("019e-cafe".to_string());
        a.next_run_at_ms = compute_next_run_at_ms(&a, a.created_at_ms);
        save_automations(&dir, std::slice::from_ref(&a)).unwrap();
        let loaded = load_automations(&dir).unwrap();
        assert_eq!(loaded, vec![a]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cadence_intervals() {
        assert_eq!(AutomationCadence::Daily.base_interval_ms(), DAY_MS);
        assert_eq!(AutomationCadence::Weekly.base_interval_ms(), 7 * DAY_MS);
        assert_eq!(AutomationCadence::Monthly.base_interval_ms(), 30 * DAY_MS);
        assert_eq!(
            AutomationCadence::EveryNDays { days: 10 }.base_interval_ms(),
            10 * DAY_MS
        );
        assert_eq!(
            AutomationCadence::EveryNDays { days: 0 }.base_interval_ms(),
            DAY_MS,
            "0 days clamps to 1"
        );
    }
}
