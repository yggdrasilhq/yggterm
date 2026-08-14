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
//! ✅ **ALL NINE ARE HERE, 2026-08-14.** `connect` was the last, and its
//! deferral was never about a verdict — the verdict was accidental from the
//! first reading; it was about SIZE, seven private helpers and an enum. What
//! made it tractable was MEASURING the cluster instead of counting it: 220
//! contiguous lines of pure functions over `ServerUiSnapshot`, every daemon
//! request already public on this crate, nothing private to the binary coming
//! with them.
//!
//! ⇒ **Nine of nine divergences measured, nine accidental, ZERO forks found.**
//! The per-verb rule that kept this surface from being collapsed wholesale
//! never had a counter-example, and now the set it was protecting is empty. A
//! structural ban like `server app`'s is defensible here on the evidence; it is
//! deliberately NOT written, because "no fork has been found" is not the same
//! claim as "no fork can exist", and the next verb should still be asked.

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

// ============================================================================
// `server connect` — moved here 2026-08-14, the ninth and last divergence.
//
// ⛔ Its verdict was recorded as ACCIDENTAL long before this move: it reads a
// snapshot and asks the daemon to place a row. No window, no app-control round
// trip, no process spawn. What deferred it was SIZE — the verb drags a cluster
// of seven helpers and an enum, and an earlier attempt was reverted rather than
// half-landed, which was the right call at the time.
//
// ⭐ What made it tractable in the end was measuring the cluster instead of
// counting it: the helpers are 220 CONTIGUOUS lines of pure functions over
// `ServerUiSnapshot`, and every daemon request they issue was ALREADY public on
// this crate. Nothing private to the binary came with them. ⇒ "seven helpers"
// sounded like the work and was not; the work is deciding that, which is one
// grep per callee.
// ============================================================================
/// The trailing session identifier of a session path (`.../<uuid>` → `<uuid>`),
/// used to match a requested path against the daemon's canonical key regardless
/// of scheme/prefix normalization.
fn connect_path_session_uuid(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Parse a remote-SCANNED Codex path (`remote-session://<machine>/<uuid>`) into
/// `(machine_key, id)`. Deliberately does NOT match `remote-cc://`: mirroring the
/// GUI's `open_session_row`, only a scanned Codex row goes through
/// OpenRemoteSession; a Claude Code row is opened as a stored session (its path
/// is not a remote-scanned path, and OpenRemoteSession would look it up as a
/// Codex transcript and fail with "saved Codex session is no longer available").
fn parse_remote_scanned_connect_path(path: &str) -> Option<(String, String)> {
    let rest = path
        .trim_start_matches('/')
        .strip_prefix("remote-session://")?;
    let (machine, id) = rest.split_once('/')?;
    if machine.is_empty() || id.is_empty() {
        return None;
    }
    Some((machine.to_string(), id.to_string()))
}

/// Session kind for a path we are opening as a stored session — the CLI twin of
/// the GUI's `session_kind_for_row`.
fn connect_session_kind_for_path(path: &str) -> yggterm_core::SessionKind {
    if path.starts_with("remote-cc://") || path.contains("/.claude/projects/") {
        yggterm_core::SessionKind::ClaudeCode
    } else {
        yggterm_core::SessionKind::Codex
    }
}

/// The scanned `(cwd, title)` for a session id, looked up from the daemon's
/// remote scans. The resume needs the right cwd (`claude -r` / `codex resume`
/// run inside the session's directory), so pass it through like the GUI does.
fn connect_scanned_metadata(
    snapshot: &crate::ServerUiSnapshot,
    path: &str,
) -> (Option<String>, Option<String>) {
    let want = connect_path_session_uuid(path);
    snapshot
        .remote_machines
        .iter()
        .flat_map(|machine| machine.sessions.iter())
        .find(|scanned| {
            scanned.session_id == want || connect_path_session_uuid(&scanned.session_path) == want
        })
        .map(|scanned| {
            let cwd = (!scanned.cwd.trim().is_empty()).then(|| scanned.cwd.clone());
            let title = (!scanned.title_hint.trim().is_empty()).then(|| scanned.title_hint.clone());
            (cwd, title)
        })
        .unwrap_or((None, None))
}

fn connect_session_is_active(snapshot: &crate::ServerUiSnapshot, path: &str) -> bool {
    let want = connect_path_session_uuid(path);
    snapshot
        .active_session_path
        .as_deref()
        .is_some_and(|active| active == path || connect_path_session_uuid(active) == want)
}

fn connect_session_key_is_known(snapshot: &crate::ServerUiSnapshot, path: &str) -> bool {
    let want = connect_path_session_uuid(path);
    connect_session_is_active(snapshot, path)
        || snapshot.live_sessions.iter().any(|session| {
            session.session_path == path || connect_path_session_uuid(&session.session_path) == want
        })
}

/// Where a freshly connected row lands in the Live Sessions order.
pub enum ConnectPlacement {
    /// Preserve the existing order; put the connected row last. Default: a
    /// connect must never rewrite an ordering the user arranged.
    End,
    /// Preserve the existing order; put the connected row directly after `anchor`.
    After(String),
    /// Daemon-native behavior: the row is prepended to the top.
    Top,
}

/// Restore `before` as the Live Sessions order, with `connected` placed per
/// `placement`. The daemon appends any live row we omit, so this can never drop
/// a row; rows in `before` that are no longer live simply resolve to nothing.
fn connect_desired_order(
    before: &[String],
    connected: &str,
    placement: &ConnectPlacement,
) -> Vec<String> {
    let want = connect_path_session_uuid(connected);
    let same =
        |candidate: &str| candidate == connected || connect_path_session_uuid(candidate) == want;
    // If the row was already live, leave it exactly where the user had it.
    if before.iter().any(|path| same(path)) {
        return before.to_vec();
    }
    let mut order = Vec::with_capacity(before.len() + 1);
    match placement {
        ConnectPlacement::Top => {
            order.push(connected.to_string());
            order.extend(before.iter().cloned());
        }
        ConnectPlacement::End => {
            order.extend(before.iter().cloned());
            order.push(connected.to_string());
        }
        ConnectPlacement::After(anchor) => {
            let anchor_uuid = connect_path_session_uuid(anchor);
            let mut placed = false;
            for path in before {
                order.push(path.clone());
                if !placed && (path == anchor || connect_path_session_uuid(path) == anchor_uuid) {
                    order.push(connected.to_string());
                    placed = true;
                }
            }
            if !placed {
                order.push(connected.to_string());
            }
        }
    }
    order
}

/// `yggterm server connect <session-path>`: connect an existing session into the
/// live set + GUI. Reuses the same daemon requests the GUI issues on a click.
fn run_server_connect_apply(
    endpoint: &crate::ServerEndpoint,
    path: &str,
    view: crate::WorkspaceViewMode,
    placement: ConnectPlacement,
) -> anyhow::Result<()> {
    // Capture the row order BEFORE anything opens/focuses — both paths prepend a
    // newly-live row, so this is the only chance to know where the user's rows sat.
    let before_order: Vec<String> = crate::snapshot(endpoint)?
        .0
        .live_sessions
        .iter()
        .map(|session| session.session_path.clone())
        .collect();
    // FocusLive reveals + resumes any session the daemon already tracks — even a
    // row the runtime-truth filter is currently suppressing, since launching its
    // runtime un-hides it — and is kind-agnostic (it uses the row the daemon
    // holds). FocusLive is a silent no-op on an unknown key, so a session that is
    // only in the remote scan falls through to the open path below.
    let (mut snapshot, mut message) = crate::focus_live_with_view(endpoint, path, Some(view))?;
    if !connect_session_is_active(&snapshot, path) {
        // Mirror the GUI's `open_session_row` exactly (one source of truth): a
        // scanned CODEX row (remote-session://) goes through OpenRemoteSession;
        // everything else — notably a Claude Code row (remote-cc://), whose path
        // is not a remote-scanned path — is opened as a stored session carrying
        // its kind, id, cwd and title.
        let (cwd, title) = connect_scanned_metadata(&snapshot, path);
        let (opened, opened_message) =
            if let Some((machine_key, session_id)) = parse_remote_scanned_connect_path(path) {
                crate::open_remote_session_with_view(
                    endpoint,
                    &machine_key,
                    &session_id,
                    cwd.as_deref(),
                    title.as_deref(),
                    Some(view),
                )?
            } else {
                crate::open_stored_session_with_view(
                    endpoint,
                    connect_session_kind_for_path(path),
                    path,
                    Some(connect_path_session_uuid(path)),
                    cwd.as_deref(),
                    title.as_deref(),
                    Some(view),
                )?
            };
        snapshot = opened;
        message = opened_message;
    }
    let connected =
        connect_session_is_active(&snapshot, path) || connect_session_key_is_known(&snapshot, path);
    // Put the user's rows back where they were. The daemon prepended the new row;
    // unless the caller asked for --top, restore `before_order` and place the
    // connected row per `placement`. Reorder never drops a row (unlisted live
    // rows are appended), so this is safe even if the scan added rows meanwhile.
    let mut placed_at = "top";
    if connected && !matches!(placement, ConnectPlacement::Top) && !before_order.is_empty() {
        let desired = connect_desired_order(&before_order, path, &placement);
        let (reordered, _) = crate::reorder_live_sessions_scoped(endpoint, &desired, None)?;
        snapshot = reordered;
        placed_at = match placement {
            ConnectPlacement::After(_) => "after_anchor",
            _ => "end",
        };
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "connected": connected,
            "requested_path": path,
            "active_session_path": snapshot.active_session_path,
            "view": match view {
                crate::WorkspaceViewMode::Terminal => "terminal",
                crate::WorkspaceViewMode::Rendered => "preview",
            },
            "row_placement": placed_at,
            "order_preserved": placed_at != "top",
            "live_session_count": snapshot.live_sessions.len(),
            "message": message,
        }))?
    );
    if !connected {
        anyhow::bail!(
            "could not connect {path}: not tracked as a live session and not found in remote scans (run `yggterm server connect --list` to see connectable sessions)"
        );
    }
    Ok(())
}


/// `yggterm server connect --list`: enumerate sessions that EXIST (remote scans)
/// but are NOT currently in the live set — the connectable "void", newest first.
fn run_server_connect_list_apply(endpoint: &crate::ServerEndpoint) -> anyhow::Result<()> {
    let (snapshot, _) = crate::snapshot(endpoint)?;
    let live_uuids: Vec<&str> = snapshot
        .live_sessions
        .iter()
        .map(|session| connect_path_session_uuid(&session.session_path))
        .collect();
    let mut connectable: Vec<&crate::RemoteScannedSession> = snapshot
        .remote_machines
        .iter()
        .flat_map(|machine| machine.sessions.iter())
        .filter(|scanned| !live_uuids.contains(&connect_path_session_uuid(&scanned.session_path)))
        .collect();
    // Newest first, so the sessions the user was most recently working with are
    // at the top of what can be a large scan (a busy host has hundreds).
    connectable.sort_by(|a, b| b.modified_epoch.cmp(&a.modified_epoch));
    let items: Vec<serde_json::Value> = connectable
        .iter()
        .map(|scanned| {
            serde_json::json!({
                "path": scanned.session_path,
                "title": scanned.title_hint,
                "cwd": scanned.cwd,
                "modified_epoch": scanned.modified_epoch,
                "live_runtime": scanned.live_runtime,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "connectable_count": items.len(),
            "live_session_count": snapshot.live_sessions.len(),
            "connectable": items,
        }))?
    );
    Ok(())
}


/// `server connect <session-path> | --list` — for BOTH binaries.
///
/// ⛔ The ARGUMENT PARSING lives here too, not just the two implementations.
/// Sharing only the bodies would have left ~40 lines of flag handling to be
/// copied into the second binary, and a second copy of "what does `--after`
/// mean" is the same defect one layer out — which is exactly how this surface
/// came to have nine divergences in the first place.
pub fn run_server_connect_cli(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    let rest = &args[2..];
    let listing = rest.iter().any(|arg| arg == "--list" || arg == "-l");
    // Validate the invocation BEFORE touching the daemon: `ensure_local_...`
    // would otherwise spawn a daemon just to print a usage error.
    let path = if listing {
        None
    } else {
        Some(
            rest.iter()
                .find(|arg| !arg.starts_with('-'))
                .context("usage: yggterm server connect <session-path> | --list")?
                .clone(),
        )
    };
    ensure_local_server_ready_for_cli(store)?;
    let endpoint = cli_server_endpoint(store.home_dir());
    let Some(path) = path else {
        return run_server_connect_list_apply(&endpoint);
    };
    let view = match cli_flag_value(args, "--view") {
        Some("preview") | Some("rendered") => crate::WorkspaceViewMode::Rendered,
        _ => crate::WorkspaceViewMode::Terminal,
    };
    // Row placement. The daemon's open/focus path PREPENDS a newly-live row,
    // which silently rewrites the user's Live Sessions ordering on every
    // connect (live-caught: a 15-session batch buried a 28-row list). Default
    // to preserving the existing order and placing the row LAST; `--top`
    // restores the old prepend, `--after <path>` places it under an anchor.
    let placement = if args.iter().any(|arg| arg == "--top") {
        ConnectPlacement::Top
    } else if let Some(anchor) = cli_flag_value(args, "--after") {
        ConnectPlacement::After(anchor.to_string())
    } else {
        ConnectPlacement::End
    };
    run_server_connect_apply(&endpoint, &path, view, placement)
}
