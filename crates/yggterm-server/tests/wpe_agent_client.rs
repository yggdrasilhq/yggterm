//! Lane-A increment 3 locks: the daemon's WPE client and its supervision.
//!
//! Every test here drives the REAL [`WpeAgentClient`] over a REAL Unix socket
//! against the `fake-wpe-agent` binary. The double exists because the failure
//! modes under test — hang forever, die mid-request, refuse to start — are
//! exactly the ones a real browser will not perform on demand, and because this
//! crate's suite must stay green on the fleet machines and CI that have no WPE
//! stack at all. The real-engine end-to-end lives in `crates/yggterm-wpe`'s own
//! suite, on the one host that carries libwpewebkit.
//!
//! Each test names the MUTATION that turns it red — a lock that can only pass
//! is worth nothing.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use yggterm_server::ServerResponse;
use yggterm_server::wpe_agent::{WpeAgentClient, WpeAgentReport, WpeOutcome};

const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_fake-wpe-agent");

/// A socket path short enough for `SUN_LEN` even when `$TMPDIR` is deep, and
/// unique per test so parallel tests never share an agent.
fn socket_path(label: &str) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.subsec_nanos())
        .unwrap_or_default();
    base.join(format!(
        "ygt-wpe-{label}-{}-{nanos}.sock",
        std::process::id()
    ))
}

/// A client wired to the fake, scripted with the given extra arguments.
///
/// The script is handed over in a SIDECAR DATA FILE beside the socket, not by
/// wrapping the binary, for two reasons. The production contract is that the
/// client spawns `<binary> <socket>` and nothing else, and a test that needed a
/// wrapper would be testing a spawn path nobody ships. And the first cut, which
/// wrote a little `exec` shim per test, was intermittently `ETXTBSY`: writing an
/// executable while other threads `fork` means a sibling child inherits the
/// still-open write fd, and `exec` on that path then fails with "Text file
/// busy". A data file cannot be busy.
fn client_with(label: &str, script: &[&str]) -> (WpeAgentClient, PathBuf) {
    let socket = socket_path(label);
    std::fs::write(format!("{}.script", socket.display()), script.join(" "))
        .expect("write the double's script");
    (
        WpeAgentClient::new(Some(PathBuf::from(FAKE_AGENT)), socket.clone()),
        socket,
    )
}

fn params(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

/// A short agent deadline, so a timeout test does not pay the production
/// default. The client always waits strictly longer than this.
fn quick() -> Map<String, Value> {
    params(&[("timeout_ms", Value::from(200u64))])
}

// ---------------------------------------------------------------------------

/// LOCK: the agent's `ok:false` line becomes a TYPED failure carrying the
/// agent's own words — never an empty success.
///
/// MUTATION that turns this red: in `classify`, map `Some(false)` to
/// `Ok(WpeOutcome::Answer { response: value })` (i.e. treat every well-formed
/// line as an answer). The verb then "succeeds" with a body that says it
/// failed, and every caller that checks `is_answer()` proceeds on a refusal.
#[test]
fn an_error_line_is_a_typed_failure_not_an_empty_success() {
    let (mut client, _socket) = client_with("errline", &["--mode", "error"]);
    let outcome = client.verb("navigate", quick());
    match &outcome {
        WpeOutcome::VerbFailed { message } => {
            assert!(
                message.contains("refuses navigate"),
                "the agent's own error text must survive: {message:?}",
            );
        }
        other => panic!("expected VerbFailed, got {other:?}"),
    }
    assert!(
        !outcome.is_answer(),
        "a refusal that reports itself as an answer is the whole bug this locks",
    );
}

/// LOCK: an agent that dies mid-request gives the caller `AgentDead` naming the
/// pid and the exit status — not a hang, and not a bare transport error.
///
/// MUTATION that turns this red: delete the `confirm_death()` call in the
/// `Ok(0)` (EOF) arm of `round_trip`. The caller then gets
/// `Transport { "the agent closed the connection without answering" }`, which
/// names neither the process nor how it died, and — worse — does not latch, so
/// the next verb silently spawns a replacement.
#[test]
fn an_agent_that_dies_mid_request_is_named_dead_with_its_exit_status() {
    // Request 1 succeeds (so we learn the pid), request 2 kills the process.
    let (mut client, _socket) = client_with(
        "diemid",
        &["--mode", "die", "--mode-from", "2", "--exit-code", "23"],
    );
    let first = client.verb("status", quick());
    assert!(first.is_answer(), "first verb should answer: {first:?}");
    let live_pid = client.report().pid.expect("a spawned agent has a pid");

    let started = Instant::now();
    let outcome = client.verb("status", quick());
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "a death must be reported promptly, not waited out as a timeout",
    );
    match &outcome {
        WpeOutcome::AgentDead { pid, exit } => {
            assert_eq!(*pid, live_pid, "the dead pid must be the one that died");
            assert!(
                exit.contains("23"),
                "the exit status must be NAMED, not merely asserted: {exit:?}",
            );
        }
        other => panic!("expected AgentDead, got {other:?}"),
    }
}

/// LOCK: no auto-respawn. After a death, every verb keeps answering `AgentDead`
/// and NO new process appears until an explicit `agent restart`.
///
/// MUTATION that turns this red: in `verb`, move the `self.dead` check below
/// `ensure_started()`. `ensure_started` sees no live process, spawns one, and
/// the plane silently heals — which is the invisible respawn loop the doctrine
/// forbids, because a crash that keeps coming back healthy is a fault nobody
/// can see or time-box.
#[test]
fn a_dead_agent_is_never_respawned_without_an_explicit_restart() {
    let (mut client, _socket) = client_with("norespawn", &["--mode", "die", "--mode-from", "2"]);
    let first = client.verb("status", quick());
    assert!(first.is_answer(), "first verb should answer: {first:?}");
    let first_pid = client.report().pid.expect("a spawned agent has a pid");
    assert!(matches!(
        client.verb("status", quick()),
        WpeOutcome::AgentDead { .. }
    ));

    for attempt in 0..3 {
        let outcome = client.verb("status", quick());
        assert!(
            matches!(outcome, WpeOutcome::AgentDead { .. }),
            "attempt {attempt} should still report the death, got {outcome:?}",
        );
        let report = client.report();
        assert_eq!(report.state, "dead");
        assert_eq!(
            report.pid, None,
            "a dead plane must own no process; a pid here means one was respawned",
        );
        assert_eq!(report.last_exit_pid, Some(first_pid));
    }

    // …and the explicit restart is what brings it back, on a NEW pid.
    let report = client.restart_agent().expect("restart should spawn");
    assert_eq!(report.state, "running");
    let new_pid = report.pid.expect("a restarted agent has a pid");
    assert_ne!(
        new_pid, first_pid,
        "restart must spawn a successor, not resurrect the corpse",
    );
    assert!(
        client.verb("status", quick()).is_answer(),
        "the plane must answer verbs again after an explicit restart",
    );
}

/// LOCK: no agent binary on this host is `NotProvisioned` — a named answer, not
/// an error cascade, and never a workspace build requirement.
///
/// MUTATION that turns this red: make `ensure_started` return
/// `WpeOutcome::Transport` (or bubble an `anyhow` error) when `resolve_binary`
/// fails. Every machine without the WPE stack — which is most of the fleet —
/// then reports a transport fault for a plane it was simply never given.
#[test]
fn an_absent_binary_is_not_provisioned_and_says_where_it_looked() {
    let missing = socket_path("absent").with_extension("nonexistent-binary");
    let mut client = WpeAgentClient::new(Some(missing.clone()), socket_path("absent"));
    match client.verb("status", quick()) {
        WpeOutcome::NotProvisioned { detail, .. } => {
            assert!(
                detail.contains(&missing.display().to_string()),
                "the answer must name the path it could not use: {detail:?}",
            );
        }
        other => panic!("expected NotProvisioned, got {other:?}"),
    }
    let report = client.report();
    assert_eq!(report.state, "not_spawned");
    assert!(report.binary.is_none());
    assert!(
        report.provisioning_detail.is_some(),
        "status must explain WHY there is no binary, or an operator cannot fix it",
    );
}

/// LOCK: an agent that never answers times out, the connection is RECYCLED, and
/// the next verb gets its own answer.
///
/// MUTATION that turns this red: in `verb`, stop clearing `self.connection` on
/// `RoundTripError::Recycle` (keep the socket). The hung connection is reused,
/// the second verb writes into a socket the agent has stopped reading, and it
/// times out too — the plane is dead for the rest of the daemon's life after
/// one slow page.
#[test]
fn a_hang_times_out_and_the_next_verb_still_answers() {
    // Only request 1 hangs; request 2 is answered normally — but only if the
    // client reconnects, because the fake parked that first connection.
    let (mut client, _socket) = client_with("hang", &["--mode", "hang", "--mode-to", "1"]);
    let started = Instant::now();
    match client.verb("eval", quick()) {
        WpeOutcome::Timeout { verb, waited_ms } => {
            assert_eq!(verb, "eval", "the timeout must name WHICH verb hung");
            assert!(waited_ms >= 200, "waited_ms should report the real wait");
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the client deadline must be bounded, not the agent's 30s default",
    );

    let outcome = client.verb("status", quick());
    assert!(
        outcome.is_answer(),
        "after a timeout the plane must still be usable: {outcome:?}",
    );
    assert_eq!(
        client.report().state,
        "running",
        "a timeout is not a death; the process is still ours",
    );
}

/// LOCK: an answer carrying somebody else's `id` is REFUSED, not returned.
///
/// This is the second guard on the same hazard as recycling: if a late answer
/// ever does reach a reused connection, verb N+1 must not be handed verb N's
/// result. Both guards exist because a mislabelled answer is worse than no
/// answer — it is internally well-formed and silently wrong.
///
/// MUTATION that turns this red: delete the `echoed != id` check in `classify`.
/// The stale line then parses cleanly and is returned as this verb's answer.
#[test]
fn an_answer_with_the_wrong_id_is_refused() {
    let (mut client, _socket) = client_with("staleid", &["--mode", "stale-id"]);
    match client.verb("read-back", quick()) {
        WpeOutcome::Transport { message } => {
            assert!(
                message.contains("does not match request id"),
                "the refusal must say the ids disagreed: {message:?}",
            );
        }
        other => panic!("expected a transport refusal, got {other:?}"),
    }
}

/// LOCK: a binary that exits during bring-up is `StartFailed`, quoting the
/// agent's own stderr diagnosis, and is LATCHED so the daemon does not fork a
/// doomed process on every verb.
///
/// MUTATION that turns this red: in `wait_until_serving`, drop the
/// `self.dead = Some(...)` latch on the start-failure path. Each verb then
/// re-spawns an agent that cannot start — a fork loop on every host whose WPE
/// stack is broken, driven by ordinary use.
#[test]
fn an_agent_that_cannot_start_says_why_and_does_not_fork_forever() {
    let (mut client, _socket) =
        client_with("diestart", &["--mode", "die-on-start", "--exit-code", "42"]);
    match client.verb("ensure", quick()) {
        WpeOutcome::StartFailed { exit, detail } => {
            assert!(
                exit.contains("42"),
                "the exit status must be named: {exit:?}"
            );
            assert!(
                detail.contains("bring-up failed"),
                "the agent's own diagnosis must be quoted back: {detail:?}",
            );
        }
        other => panic!("expected StartFailed, got {other:?}"),
    }
    // Latched: no second spawn attempt.
    assert!(
        matches!(client.verb("ensure", quick()), WpeOutcome::AgentDead { .. }),
        "a start failure must latch like any other death",
    );
    assert_eq!(client.report().state, "dead");
}

/// LOCK: `agent stop` releases the process and returns the plane to
/// `not_spawned`, from which the next verb lazily spawns.
///
/// Stop is the one death that does NOT latch, because it is the only one
/// nobody needs to be told about — the operator caused it. MUTATION that turns
/// this red: make `stop_agent` set the `dead` latch (treating a deliberate stop
/// as a fault). Every stop then wedges the plane until an explicit restart,
/// and lazy spawn — the documented behaviour — is unreachable.
#[test]
fn stop_releases_the_process_and_the_next_verb_lazily_spawns() {
    let (mut client, _socket) = client_with("stop", &["--mode", "ok"]);
    let first = client.verb("status", quick());
    assert!(first.is_answer(), "first verb should answer: {first:?}");
    let first_pid = client.report().pid.expect("a spawned agent has a pid");

    let report = client.stop_agent();
    assert_eq!(report.state, "not_spawned");
    assert_eq!(report.pid, None);
    assert_eq!(
        report.last_exit, None,
        "a deliberate stop is not a fault and must not be reported as one",
    );

    assert!(
        client.verb("status", quick()).is_answer(),
        "the next verb must lazily spawn a fresh agent",
    );
    assert_ne!(client.report().pid, Some(first_pid));
}

/// LOCK: params reach the agent VERBATIM, and the client adds only `id` and
/// `verb`.
///
/// MUTATION that turns this red: have `verb()` rewrite or drop any param (for
/// example, stringifying `width`). The daemon would then be a second, drifting
/// encoding of a vocabulary the agent owns — the thing this module exists not
/// to be.
#[test]
fn params_reach_the_agent_verbatim() {
    let (mut client, _socket) = client_with("verbatim", &["--mode", "ok"]);
    let sent = params(&[
        ("session", Value::from("a")),
        ("width", Value::from(800u64)),
        ("selector", Value::from("#go")),
        ("timeout_ms", Value::from(200u64)),
    ]);
    let WpeOutcome::Answer { response } = client.verb("ensure", sent) else {
        panic!("expected an answer");
    };
    let echo = response.get("echo").expect("the fake echoes the request");
    assert_eq!(echo.get("session").and_then(Value::as_str), Some("a"));
    assert_eq!(echo.get("width").and_then(Value::as_u64), Some(800));
    assert_eq!(echo.get("selector").and_then(Value::as_str), Some("#go"));
    assert_eq!(echo.get("verb").and_then(Value::as_str), Some("ensure"));
    assert!(
        echo.get("id").and_then(Value::as_str).is_some(),
        "the client supplies the id so callers never have to",
    );
}

/// LOCK: one agent process serves many verbs — the client does not respawn per
/// request.
///
/// MUTATION that turns this red: drop the `if self.process.is_some()` early
/// return in `ensure_started`. Every verb then pays a full WebKit bring-up and
/// loses the session table the whole verb plane is built on (`ensure` is
/// idempotent per session key only while the process survives).
#[test]
fn one_process_serves_every_verb() {
    let (mut client, _socket) = client_with("reuse", &["--mode", "ok"]);
    let mut pids = Vec::new();
    for round in 0..4 {
        match client.verb("status", quick()) {
            WpeOutcome::Answer { response } => {
                pids.push(response.get("pid").and_then(Value::as_u64))
            }
            other => panic!("round {round} did not answer: {other:?}"),
        }
    }
    assert!(
        pids.windows(2).all(|pair| pair[0] == pair[1]),
        "every verb must be served by the SAME agent process, got {pids:?}",
    );
}

/// LOCK: every WPE answer survives the DAEMON wire, in both directions.
///
/// This one is here because a live round trip found what nine unit tests could
/// not. `ServerResponse` is internally tagged `kind` and so is [`WpeOutcome`],
/// and the first cut carried the outcome as a NEWTYPE variant — which flattens
/// the inner tag into the outer object. Every answer went out as
/// `{"kind":"wpe","kind":"answer",…}` and every client died on
/// `duplicate field 'kind'`. The plane worked perfectly; nothing could read it.
///
/// MUTATION that turns this red: change the variants back to
/// `Wpe(WpeOutcome)` / `WpeAgent(WpeAgentReport)`. Serialization still
/// succeeds — that is what made the bug invisible — and only the parse back
/// fails, which is exactly what the client does.
#[test]
fn every_wpe_response_survives_the_daemon_wire() {
    let outcomes = [
        WpeOutcome::Answer {
            response: serde_json::json!({"id": "1", "ok": true, "view": 0}),
        },
        WpeOutcome::VerbFailed {
            message: "no view for session \"a\"".into(),
        },
        WpeOutcome::NotProvisioned {
            searched: "yggterm-wpe-agent".into(),
            detail: "not on PATH".into(),
        },
        WpeOutcome::StartFailed {
            exit: "exited 1".into(),
            detail: "headless bring-up failed".into(),
        },
        WpeOutcome::AgentDead {
            pid: 4242,
            exit: "killed by signal 9".into(),
        },
        WpeOutcome::Timeout {
            verb: "navigate".into(),
            waited_ms: 30_000,
        },
        WpeOutcome::Transport {
            message: "socket gone".into(),
        },
    ];
    for outcome in outcomes {
        let wire = serde_json::to_string(&ServerResponse::Wpe {
            outcome: outcome.clone(),
        })
        .expect("a response must serialize");
        let parsed: ServerResponse = serde_json::from_str(&wire)
            .unwrap_or_else(|error| panic!("{wire} must parse back, got {error}"));
        match parsed {
            ServerResponse::Wpe { outcome: back } => assert_eq!(
                back, outcome,
                "the outcome must survive the wire unchanged: {wire}",
            ),
            other => panic!("expected a wpe response, got {other:?}"),
        }
    }

    let report = WpeAgentReport {
        state: "dead".into(),
        binary: Some("/usr/bin/yggterm-wpe-agent".into()),
        provisioning_detail: None,
        socket: "/run/user/1000/yggterm-wpe-7.sock".into(),
        log: "/run/user/1000/yggterm-wpe-7.log".into(),
        pid: None,
        spawned_at_ms: None,
        last_exit: Some("exited 23".into()),
        last_exit_pid: Some(4242),
    };
    let wire = serde_json::to_string(&ServerResponse::WpeAgent {
        report: report.clone(),
    })
    .expect("a report must serialize");
    match serde_json::from_str::<ServerResponse>(&wire)
        .unwrap_or_else(|error| panic!("{wire} must parse back, got {error}"))
    {
        ServerResponse::WpeAgent { report: back } => assert_eq!(back, report),
        other => panic!("expected a wpe agent response, got {other:?}"),
    }
}
