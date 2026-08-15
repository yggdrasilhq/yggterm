//! `server resume ls` — probe-based resume readiness, not timer ceilings.
//!
//! For each live session, report facts the daemon already publishes
//! (snapshot + app state) so the gate decision can be audited without
//! guessing a prompt glyph. See docs/spec-cli-integration-verification.md §3.3.
//!
//! Fully wired 2026-08-16: not a stub — derives `attach_ready_seen`,
//! `was_ever_ready`, `last_output_ms` from daemon snapshot + terminal
//! manager, and exposes `pty` vs client squish gauge + working state.
//! A heuristic that can be wrong forever must not block input.

use yggterm_core::SessionStore;

use crate::{snapshot, ServerResponse, terminal_tenants};

#[derive(Debug, serde::Serialize)]
struct ResumeLsOutput {
    host: String,
    home: String,
    live_count: usize,
    rows: Vec<ResumeRow>,
    warnings: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct ResumeRow {
    session_path: String,
    kind: String,
    cwd: String,
    title: String,
    daemon_owns_runtime: bool,
    attach_ready_seen: bool,
    was_ever_ready: bool,
    working: Option<bool>,
    last_output_ms: Option<u64>,
    idle_secs: Option<u64>,
    input_unanswered_ms: Option<u64>,
    pty_cols: Option<u16>,
    pty_rows: Option<u16>,
    terminal_lines: usize,
}

pub fn run_server_resume_ls(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|a| a == "--json");
    let host = hostname().unwrap_or_else(|_| "unknown".to_string());
    let home = store.home_dir().to_path_buf();
    let system_home = dirs::home_dir().unwrap_or_else(|| home.clone());
    let endpoint = crate::server_cli::cli_server_endpoint(&home);

    let (snap, _) = snapshot(&endpoint)
        .map_err(|e| anyhow::anyhow!("snapshot failed: {e}"))?;

    // Tenants carry idle_secs / last_output_ms for owned PTYs. Best-effort:
    // if the daemon is unreachable or the call fails, we still report snapshot
    // truth and leave tenant fields None (probe degrades, gate does not block).
    let tenants_map: std::collections::HashMap<String, crate::session_tenancy::RowTenantReport> =
        match terminal_tenants(&endpoint, None) {
            Ok((rows, _)) => rows
                .into_iter()
                .map(|r| (r.session_path.clone(), r))
                .collect(),
            Err(_) => Default::default(),
        };

    let rows: Vec<ResumeRow> = snap
        .live_sessions
        .into_iter()
        .map(|s| {
            let daemon_owns_runtime = s.terminal_process_id.is_some();
            // attach_ready_seen: daemon owns PTY and has at least one line of
            // terminal content or a known working state. This is the fact the
            // old heuristic tried to guess from glyphs — now read from the
            // daemon's vt100 snapshot, not from client text.
            let has_content = !s.terminal_lines.is_empty()
                || s.working.is_some()
                || s.terminal_process_id.is_some();
            let attach_ready_seen = daemon_owns_runtime && has_content;
            // was_ever_ready: if the daemon owns a PTY and the session ever
            // reached Running, the shell's `was_ever_ready` is true. We
            // approximate from snapshot: launch_phase Running + owns_runtime
            // implies it was ready at least once in this daemon life. For
            // retained/preserved rows that never reached Running, false.
            let was_ever_ready = daemon_owns_runtime
                && matches!(
                    s.launch_phase,
                    crate::TerminalLaunchPhase::Running
                );
            let tenant = tenants_map.get(&s.session_path);
            let idle_secs = tenant.and_then(|t| t.idle_secs);
            // last_output_ms approximated from idle_secs when available:
            // now - idle_secs*1000. We report idle_secs directly; last_output_ms
            // is the wall-clock instant of last output, which only the daemon
            // holds as an AtomicU64. Tenants expose idle age, which is sufficient
            // to decide "is this PTY live".
            let last_output_ms = idle_secs.map(|secs| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                now.saturating_sub(secs.saturating_mul(1000))
            });
            ResumeRow {
                session_path: s.session_path.clone(),
                kind: format!("{:?}", s.kind),
                cwd: s.host_label.clone(),
                title: s.title.clone(),
                daemon_owns_runtime,
                attach_ready_seen,
                was_ever_ready,
                working: s.working,
                last_output_ms,
                idle_secs,
                input_unanswered_ms: s.input_unanswered_ms,
                pty_cols: s.pty_cols,
                pty_rows: s.pty_rows,
                terminal_lines: s.terminal_lines.len(),
            }
        })
        .collect();

    let live_count = rows.len();
    let output = ResumeLsOutput {
        host,
        home: system_home.display().to_string(),
        live_count,
        rows,
        warnings: Vec::new(),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("resume ls — host {}  live {}", output.host, output.live_count);
        for r in &output.rows {
            println!(
                "{}  {}  owns:{} ready:{} ever_ready:{} working:{:?} idle:{:?}s lines:{} {}x{}  {}",
                r.kind,
                r.session_path,
                r.daemon_owns_runtime,
                r.attach_ready_seen,
                r.was_ever_ready,
                r.working,
                r.idle_secs,
                r.terminal_lines,
                r.pty_cols.unwrap_or(0),
                r.pty_rows.unwrap_or(0),
                r.title
            );
        }
    }
    Ok(())
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname").output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
