//! THE `server <verb>` CLI's SHARED IMPLEMENTATIONS — one owner, both binaries.
//!
//! `server` is dispatched in `apps/yggterm/src/main.rs` AND
//! `apps/yggterm/src/bin/yggterm-headless.rs`, the way `server app` was before
//! `crate::app_control_cli` collapsed it, and nine of its verbs answered from
//! one binary only.
//!
//! ⛔ **BUT THIS SURFACE IS NOT COLLAPSED WHOLESALE, AND THAT IS DELIBERATE.**
//! Unlike `server app` — one homogeneous plane where every verb belonged on
//! both — `server` MIXES planes: deploy and relay machinery that is genuinely
//! headless-only (`gate-screen`, `relay-boundary`, `wpe`) sits beside daemon
//! operations that are not. A structural ban on a second dispatcher here would
//! forbid a fork that is real. ⇒ The question is asked PER VERB, and only the
//! verbs answered accidentally live here.
//!
//! What made these four accidental is visible in their own first lines: each
//! does `ensure_local_server_ready_for_cli` + `cli_server_endpoint` and then
//! talks to the DAEMON over the local socket. There is no window in any of
//! them, so the headless CLI — the binary agents drive, and the one that most
//! wants to reorder rows and hold a write lock — could always have served them.
//! It could not, because of which file they were typed into.

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
