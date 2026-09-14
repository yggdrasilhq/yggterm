//! [11.6.0] acceptance — the wrapper-level ledger-served reattach SLA run.
//!
//! Stone: `docs/cli-integration-layer.md` §9 — "The reattach SLA is measured
//! per class on a forced same-version rotation — adopted-PTY rows attach
//! with zero /proc polling (the `external_agent_resume_*` scans must be
//! unreachable from the ledger-served path)." The writer half was already
//! proven live on a real rotation (2026-09-10, pending-bugs §11.6.0: the
//! superseded-self-retire sweep wrote a correct ledger under a real
//! handover). This harness locks the CONSUMER half at the wrapper level,
//! deterministically and in CI:
//!
//! - **HALF A (the ledger vouches):** a real `adopted` record — written
//!   through the real writer verbs (`handoff_ownership_records` +
//!   `record_handoff`) — must make [`run_remote_resume_codex`] trace
//!   `reattach_ledger_served`, never enter the /proc wait
//!   (`external_active_wait*` absent from the trace), and fail fast into the
//!   clean ensure tail (no daemon lives on the scratch home) instead of
//!   burning the deadline.
//! - **HALF B (the counterfactual, ledger empty):** the same state with NO
//!   record must take the pre-ledger path — announce `external_active_wait`
//!   against a real holder process, burn the (env-shortened) deadline, trace
//!   `external_active_wait_deadline`, and refuse. That contrast IS the SLA
//!   measurement the acceptance asks for.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yggterm_server::ownership_ledger;
use yggterm_server::{run_remote_resume_codex, SessionKind};

/// Unique and greppable on purpose: the /proc fd-arm keys on this substring,
/// so nothing else on the host can be mistaken for this session's holder.
const SESSION_ID: &str = "sla-ledger-served-4f2a9c31be75";
const RUNTIME_KEY: &str = "codex-runtime://sla-ledger-served-4f2a9c31be75";
const SHORT_DEADLINE_MS: u64 = 1_500;

fn scratch_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yggterm-sla-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis(),
        tag
    ));
    fs::create_dir_all(&dir).expect("scratch home");
    dir
}

/// Seed the saved-session probe: a codex rollout whose CONTENT names the id
/// (the probe parses identity fields; it does not trust the filename).
fn seed_saved_codex_session(codex_home: &Path, session_id: &str) {
    let sessions = codex_home.join("sessions").join("2026").join("01");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::write(
        sessions.join(format!("rollout-{session_id}.jsonl")),
        format!("{{\"id\":\"{session_id}\",\"cwd\":\"/tmp\"}}\n"),
    )
    .expect("seed rollout");
}

/// The trace plane is file-shaped under the home; read every small file under
/// it so the assertions do not re-derive the exact event-file layout.
fn trace_text_under(home: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![home.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(text) = fs::read_to_string(&path) {
                out.push_str(&text);
            }
        }
    }
    out
}

#[test]
fn ledger_served_resume_skips_the_proc_wait_and_the_empty_ledger_burns_it() {
    // ---- HALF A — the ledger vouches ------------------------------------
    let home = scratch_home("served");
    let codex_home = scratch_home("served-codex");
    // Single test fn on purpose: env vars are process-global, so both halves
    // run sequentially on this thread instead of racing other threads.
    unsafe {
        std::env::set_var("YGGTERM_HOME", &home);
        std::env::set_var("CODEX_HOME", &codex_home);
    }
    seed_saved_codex_session(&codex_home, SESSION_ID);

    // The writer half, through the real verbs: this row crossed to "the
    // successor" — this process stands in as the adopter, which is exactly
    // the fact the record asserts (by_pid alive; the liveness contract the
    // consumer re-derives on every read).
    let records = ownership_ledger::handoff_ownership_records(
        &[(
            RUNTIME_KEY.to_string(),
            SessionKind::Codex,
            SESSION_ID.to_string(),
        )],
        &[(RUNTIME_KEY.to_string(), (std::process::id(), 0))],
        ownership_ledger::now_ms(),
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
    );
    ownership_ledger::record_handoff(&home, records).expect("record handoff");
    assert!(
        !ownership_ledger::load(&home).is_empty(),
        "the ledger must hold the record it just wrote"
    );

    let started = Instant::now();
    let outcome = run_remote_resume_codex(SESSION_ID, None, false);
    let served_elapsed = started.elapsed();

    let trace = trace_text_under(&home);
    assert!(
        trace.contains("reattach_ledger_served"),
        "the wrapper must consult the ledger and trace the answer; trace tail: {:?}",
        &trace[trace.len().saturating_sub(400)..]
    );
    assert!(
        trace.contains("\"adopted\""),
        "the served disposition must be the adopted record"
    );
    assert!(
        !trace.contains("external_active_wait"),
        "a ledger-served resume must never reach the /proc wait"
    );
    assert!(
        served_elapsed < Duration::from_secs(45),
        "the served resume must fail fast into the ensure tail, took {served_elapsed:?}"
    );
    // No daemon lives on the scratch home, so the ensure tail cannot succeed;
    // whatever error it names, it is not the deadline wait that predates the ledger.
    assert!(
        outcome.is_err(),
        "no daemon on the scratch home: the ensure tail must refuse"
    );

    // ---- HALF B — the counterfactual: the same state, ledger empty -------
    let home_b = scratch_home("burned");
    let codex_home_b = scratch_home("burned-codex");
    unsafe {
        std::env::set_var("YGGTERM_HOME", &home_b);
        std::env::set_var("CODEX_HOME", &codex_home_b);
        std::env::set_var(
            "YGGTERM_EXTERNAL_ACTIVE_WAIT_DEADLINE_MS",
            SHORT_DEADLINE_MS.to_string(),
        );
    }
    seed_saved_codex_session(&codex_home_b, SESSION_ID);

    // A fake holder: a plain process holding an open path that carries the
    // session id — the fd arm classifies it as the holder (External: no
    // yggterm environ marker, no yggterm ancestor). This is the process the
    // pre-ledger path burns its deadline against.
    let held_path = home_b.join(format!("rollout-writer-{SESSION_ID}.lock"));
    fs::write(&held_path, "held\n").expect("holder path");
    let held = fs::File::open(&held_path).expect("open holder path");
    let mut holder = std::process::Command::new("sleep")
        .arg("120")
        .stdin(held)
        .spawn()
        .expect("spawn fake holder");

    let started_b = Instant::now();
    let outcome_b = run_remote_resume_codex(SESSION_ID, None, false);
    let burned_elapsed = started_b.elapsed();

    let _ = holder.kill();
    let _ = holder.wait();

    let trace_b = trace_text_under(&home_b);
    assert!(
        trace_b.contains("external_active_wait"),
        "the pre-ledger path must announce the /proc wait"
    );
    assert!(
        trace_b.contains("external_active_wait_deadline"),
        "the pre-ledger path must burn to the deadline and refuse"
    );
    assert!(
        !trace_b.contains("reattach_ledger_served"),
        "an empty ledger must not answer"
    );
    assert!(
        burned_elapsed >= Duration::from_millis(SHORT_DEADLINE_MS),
        "the deadline is a refusal, not a licence — it must be burned: {burned_elapsed:?}"
    );
    assert!(
        outcome_b.is_err(),
        "the deadline path must refuse the resume"
    );

    println!(
        "SLA measured: ledger-served resume reached its clean refusal in \
         {served_elapsed:?} (reattach_ledger_served traced, external_active_wait ABSENT); \
         empty-ledger resume burned the {SHORT_DEADLINE_MS} ms deadline in \
         {burned_elapsed:?} (wait announced + deadline traced)"
    );

    let _ = fs::remove_dir_all(&home);
    let _ = fs::remove_dir_all(&codex_home);
    let _ = fs::remove_dir_all(&home_b);
    let _ = fs::remove_dir_all(&codex_home_b);
}
