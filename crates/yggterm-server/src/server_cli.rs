//! THE `server <verb>` CLI's SHARED IMPLEMENTATIONS — one owner, both binaries.
//!
//! `server` is dispatched in `apps/yggterm/src/main.rs` AND
//! `apps/yggterm/src/bin/yggterm-headless.rs`, the way `server app` was before
//! `crate::app_control_cli` collapsed it, and nine of its verbs answered from
//! one binary only.
//!
//! ⛔ **THE QUESTION IS ASKED PER VERB, NOT WHOLESALE — but every verb asked so
//! far has answered the same way.** `server app` was collapsed with a
//! structural ban because it was one homogeneous plane. This surface was
//! believed to mix planes, with deploy and relay machinery that was *genuinely*
//! headless-only sitting beside daemon operations that were not, and that
//! belief is why no ban was written here.
//!
//! ⚠ **CORRECTED 2026-08-14: the fork it described does not exist.** The three
//! verbs named as the real fork — `gate-screen`, `relay-boundary`, `wpe` —
//! were read end to end and every one is accidental. They live here now. The
//! per-verb rule stands on its own merits; it no longer stands on a measured
//! counter-example, and nobody should quote one from this file.
//!
//! What makes a verb accidental is visible in its own first lines, and the test
//! has convicted seven of them: it does `ensure_local_server_ready_for_cli` +
//! `cli_server_endpoint` and then talks to the DAEMON over the local socket, or
//! it reads a host fact out of the home directory and talks to nothing. There
//! is no window in any of them, so either binary could always have served them.
//! Neither could, because of which file they were typed into.
//!
//! ⏳ One divergence is left — `connect`, GUI-only. Its verdict is also
//! accidental; what is deferred is the MOVE, because it drags seven private
//! helpers with it. That is a size decision, not a fork.

use anyhow::Context;
use std::io::Read;

use yggterm_core::{SessionStore, cli_flag_value};

use crate::{
    ensure_local_daemon_running, resolve_client_daemon_endpoint, row_order_ledger_report, snapshot,
};

/// The endpoint this CLI should talk to. Duplicated in BOTH binaries before —
/// byte-identical, two lines each, over a resolver that already lived here.
pub fn cli_server_endpoint(home_dir: &std::path::Path) -> crate::ServerEndpoint {
    resolve_client_daemon_endpoint(home_dir).endpoint
}

/// Make sure a daemon is answering before a CLI verb talks to one.
pub fn ensure_local_server_ready_for_cli(store: &SessionStore) -> anyhow::Result<()> {
    let resolved = resolve_client_daemon_endpoint(store.home_dir());
    if resolved.version_mismatch.is_some() {
        // A daemon of another version is live and owns this home's sessions.
        // It is the source of truth; attach to it rather than spawning a peer.
        return Ok(());
    }
    ensure_local_daemon_running(&resolved.endpoint)
}

/// `server ledger` — for BOTH binaries.
pub fn run_server_ledger_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    let scope = args
        .iter()
        .position(|arg| arg == "--scope")
        .and_then(|ix| args.get(ix + 1))
        .map(String::as_str);
    ensure_local_server_ready_for_cli(&store)?;
    let endpoint = cli_server_endpoint(store.home_dir());
    let report = row_order_ledger_report(&endpoint, scope)?;
    match serde_json::from_str::<serde_json::Value>(&report) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(_) => println!("{report}"),
    }
    Ok(())
}

/// `server order` — for BOTH binaries.
pub fn run_server_order_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    ensure_local_server_ready_for_cli(&store)?;
    let endpoint = cli_server_endpoint(store.home_dir());
    let (snapshot, _) = snapshot(&endpoint)?;
    let order: Vec<String> = snapshot
        .live_sessions
        .iter()
        .map(|session| session.session_path.clone())
        .collect();
    if args.iter().any(|arg| arg == "--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "live_session_count": order.len(),
                "order": order,
            }))?
        );
    } else {
        for path in &order {
            println!("{path}");
        }
    }
    Ok(())
}

/// `server reorder` — for BOTH binaries.
pub fn run_server_reorder_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    let scope = args
        .iter()
        .position(|arg| arg == "--scope")
        .and_then(|ix| args.get(ix + 1))
        .cloned();
    let scope_value_ix = args
        .iter()
        .position(|arg| arg == "--scope")
        .map(|ix| ix + 1);
    let ordered: Vec<String> = if args.iter().any(|arg| arg == "--stdin") {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading reorder stdin")?;
        buf.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    } else {
        args[2..]
            .iter()
            .enumerate()
            .filter(|(ix, arg)| !arg.starts_with('-') && Some(ix + 2) != scope_value_ix)
            .map(|(_, arg)| arg.clone())
            .collect()
    };
    if ordered.is_empty() {
        anyhow::bail!(
            "usage: yggterm server reorder <session-path>... | --stdin [--scope <scope>]"
        );
    }
    ensure_local_server_ready_for_cli(&store)?;
    let endpoint = cli_server_endpoint(store.home_dir());
    return run_server_reorder_apply(&endpoint, &ordered, scope.as_deref());
    Ok(())
}

/// `server write-lock` — for BOTH binaries.
pub fn run_server_write_lock_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    ensure_local_server_ready_for_cli(&store)?;
    let endpoint = cli_server_endpoint(store.home_dir());
    let verb = args.get(2).map(String::as_str).unwrap_or("");
    let profile = cli_flag_value(&args, "--profile");
    let pid = std::process::id();
    match verb {
        "acquire" | "hold" => {
            let status = crate::acquire_profile_write_lock(&endpoint, profile, pid)?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            if verb == "hold" {
                if !status.writable {
                    // Did not get the lock (a peer holds it): nothing to hold.
                    std::process::exit(1);
                }
                // Flush before parking: stdout is block-buffered when piped, so
                // without this the JSON above never reaches a redirected log.
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
                eprintln!(
                    "holding profile write-lock {:?} as pid {} — SIGTERM/Ctrl-C to release",
                    status.profile, pid
                );
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
            return Ok(());
        }
        "report" => {
            let status = crate::profile_write_lock_report(&endpoint)?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }
        "release" => {
            let status = crate::release_profile_write_lock(&endpoint, profile, pid)?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }
        other => anyhow::bail!(
            "usage: yggterm server write-lock <acquire|hold|report|release> \
             [--profile <name>] (got {other:?})"
        ),
    }
    Ok(())
}

/// Apply a row order and REPORT what actually moved.
///
/// Moved with its verb: it lived in the GUI binary, which is why `server
/// reorder` could not answer from the headless CLI even though every line of
/// it talks to the daemon.
pub(crate) fn run_server_reorder_apply(
    endpoint: &crate::ServerEndpoint,
    ordered_paths: &[String],
    client_scope: Option<&str>,
) -> anyhow::Result<()> {
    let (before, _) = snapshot(endpoint)?;
    let before_order: Vec<String> = before
        .live_sessions
        .iter()
        .map(|session| session.session_path.clone())
        .collect();
    let (after, message) = crate::reorder_live_sessions_scoped(endpoint, ordered_paths, client_scope)?;
    let after_order: Vec<String> = after
        .live_sessions
        .iter()
        .map(|session| session.session_path.clone())
        .collect();
    let update = message
        .as_deref()
        .and_then(crate::LiveSessionOrderUpdate::from_message);
    let mut report = serde_json::json!({
        "requested": ordered_paths,
        "live_session_count": after_order.len(),
        "changed": before_order != after_order,
        "order": after_order,
    });
    match &update {
        Some(update) => {
            report["applied"] = serde_json::json!(update.applied);
            report["skipped"] = serde_json::json!(update.skipped);
            report["message"] = serde_json::json!(update.summary());
        }
        // An older daemon cannot say what it applied. Report the gap rather
        // than inventing an `applied` list out of the request.
        None => {
            report["applied"] = serde_json::Value::Null;
            report["skipped"] = serde_json::Value::Null;
            report["applied_unreported_by_daemon"] = serde_json::json!(true);
            report["message"] = serde_json::json!(message);
        }
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(update) = update
        && !update.skipped.is_empty()
    {
        anyhow::bail!("{}", update.summary());
    }
    Ok(())
}

/// `server gate-screen` — for BOTH binaries.
///
/// §3's audit instrument. Read-only, on demand, and connected directly like the
/// other read-only diagnostics — a verb that spawned a daemon in order to ask
/// what a daemon is looking at would answer about a process that did not exist
/// when the question was asked.
///
/// ⛔ NOT WRITTEN ANYWHERE. The screens go to this stdout and nowhere else —
/// see `HotRestartGateScreen`. A caller harvesting a corpus owns where it lands
/// and how long it lives.
pub fn run_server_gate_screen_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    let endpoint = cli_server_endpoint(store.home_dir());
    let path = args.get(2).filter(|arg| !arg.starts_with("--"));
    let tail_lines = cli_flag_value(args, "--tail").and_then(|value| value.parse().ok());
    let sessions =
        crate::hot_restart_gate_screens(&endpoint, path.map(String::as_str), tail_lines)?;
    if args.iter().any(|arg| arg == "--json") {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("no sessions owned by this daemon match");
        return Ok(());
    }
    for session in &sessions {
        let verdict = match session.blocker.as_ref() {
            Some(blocker) => format!(
                "{kind}{permanent}, idle {idle}",
                kind = blocker.kind,
                permanent = if blocker.permanent { " (permanent)" } else { "" },
                idle = blocker
                    .idle_ms
                    .map(|ms| format!("{}s", ms / 1000))
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            None => "not blocking".to_string(),
        };
        println!(
            "== {key}\n   gate verdict: {verdict}\n   screen_text_shows_agent_working: {working}\n   screen: {screen}",
            key = session.session_key,
            working = session.shows_agent_working,
            screen = if session.screen_available {
                "readable"
            } else {
                "UNREADABLE — the gate is classifying this one blind"
            },
        );
        for line in session.screen_tail.iter().flatten() {
            println!("   | {line}");
        }
    }
    Ok(())
}

/// `server relay-boundary` — for BOTH binaries.
///
/// §2 of docs/spec-hot-restart-relay-gate.md — *"a relay hand-off is a genuine,
/// declared, zero-cost quiet point … the gate stops being a search and becomes
/// an appointment."*
///
/// ⛔ It does NOT spawn a daemon (no `ensure_local_server_ready_for_cli`) and it
/// does not talk to one. The queue is a HOST fact in a file, and making the verb
/// reach a daemon would mean choosing which of the several a stale host is
/// running — the exact question §4 moved out of any one daemon's status. A
/// drainer picks the boundary up on its next 20 s poll.
pub fn run_server_relay_boundary_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|arg| arg == "--json");
    let declared_by = args
        .iter()
        .position(|arg| arg == "--by")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_else(|| "relay_boundary".to_string());
    let wait_secs = args
        .iter()
        .position(|arg| arg == "--wait-secs")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let outcome =
        crate::hot_restart_queue::declare_relay_boundary(store.home_dir(), now_ms, &declared_by);
    let (owed, target_version, waiting_ms) = match &outcome {
        crate::hot_restart_queue::RelayBoundaryOutcome::Declared {
            target_version,
            waiting_ms,
        } => (true, Some(target_version.clone()), Some(*waiting_ms)),
        crate::hot_restart_queue::RelayBoundaryOutcome::NothingOwed => (false, None, None),
    };
    // ⚠ The drainer polls every 20 s, so a wait shorter than that can only ever
    // time out — say so rather than reporting a converged host as still-owing.
    // Waiting is opt-in because the common case is a converged host with nothing
    // to wait for.
    let mut converged = !owed;
    if owed && wait_secs > 0 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if crate::hot_restart_queue::load(store.home_dir()).is_none() {
                converged = true;
                break;
            }
        }
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "declared_by": declared_by,
                "swap_owed": owed,
                "target_version": target_version,
                "waiting_ms": waiting_ms,
                "waited_for_secs": wait_secs,
                "converged": converged,
            }))?
        );
    } else if let Some(target_version) = target_version {
        let waiting_min = waiting_ms.unwrap_or(0) / 60_000;
        if converged {
            println!("relay boundary declared by {declared_by}; swap to {target_version} converged");
        } else {
            println!(
                "relay boundary declared by {declared_by}; swap to {target_version} \
                 (owed {waiting_min}m) is due at the next drainer poll"
            );
        }
    } else {
        println!("relay boundary declared by {declared_by}; no swap is owed on this host");
    }
    Ok(())
}

/// `server wpe <verb>` — for BOTH binaries.
///
/// The plane lives INSIDE the daemon (it owns the agent process), so unlike the
/// read-only diagnostics this needs a daemon to exist.
pub fn run_server_wpe_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    use crate::wpe_agent::{WpeOutcome, params_from_flags};

    ensure_local_server_ready_for_cli(store)?;
    let endpoint = cli_server_endpoint(store.home_dir());

    if args[0] == "agent" {
        let action = args
            .get(1)
            .map(String::as_str)
            .context("usage: server wpe agent <status|restart|stop>")?;
        return match crate::wpe_agent_control(&endpoint, action)? {
            Ok(report) => {
                println!("{}", serde_json::to_string_pretty(&report)?);
                Ok(())
            }
            Err(outcome) => {
                print_wpe_failure("agent", &outcome)?;
                std::process::exit(1);
            }
        };
    }

    let verb = args[0].as_str();
    let params = params_from_flags(&args[1..]).map_err(|message| anyhow::anyhow!(message))?;
    match crate::wpe_verb(&endpoint, verb, params)? {
        WpeOutcome::Answer { response } => {
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        outcome => {
            print_wpe_failure(verb, &outcome)?;
            std::process::exit(1);
        }
    }
}

/// One printer for every failure arm, so the shape a script parses does not
/// depend on which way the plane failed.
fn print_wpe_failure(verb: &str, outcome: &crate::wpe_agent::WpeOutcome) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": false,
            "verb": verb,
            "summary": outcome.summary(),
            "failure": outcome,
        }))?
    );
    Ok(())
}
