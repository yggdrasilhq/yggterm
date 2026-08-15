//! `server startpage ls` — the lie detector for the Startpage's RECENT WORK list.
//!
//! Reuses the store half of the startpage code (`yggterm_core::agent_cli`
//! descriptors + `read_store_entry`) so a new CLI is not a new place to
//! remember. The ordering is the same `modified_epoch` rank the shell uses,
//! without the shell-only live/scope gates that would hide a lying durable
//! row. A Python oracle (`scripts/check-startpage.py`) walks the raw jsonls
//! independently and compares.

use std::path::PathBuf;

use anyhow::Context;
use yggterm_core::{SessionStore, startpage::{order_for_startpage, scan_all_durable_sessions}};

use crate::snapshot;

#[derive(Debug, serde::Serialize)]
struct StartpageLsOutput {
    host: String,
    home: String,
    durable_count: usize,
    live_count: usize,
    rows: Vec<yggterm_core::startpage::StartpageDurableRow>,
    live_session_paths: Vec<String>,
    warnings: Vec<String>,
}

pub fn run_server_startpage_ls(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|a| a == "--json");
    let limit = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|ix| args.get(ix + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200);

    let home = store.home_dir().to_path_buf();
    let host = hostname().unwrap_or_else(|_| "unknown".to_string());
    // System HOME is where the CLI stores live (~/.codex, ~/.claude, …),
    // not YGGTERM_HOME (which is ~/.yggterm). The earlier version passed
    // the yggterm home and therefore walked the wrong tree (0 rows).
    let system_home = dirs::home_dir().unwrap_or_else(|| home.clone());

    // Durable sessions — ground truth from the stores, via the descriptors.
    let mut rows = scan_all_durable_sessions(&system_home);
    let warnings = collect_warnings(&system_home, &rows);
    rows = order_for_startpage(rows);
    if rows.len() > limit {
        rows.truncate(limit);
    }

    // Live sessions from the daemon snapshot — what the shell's live-first
    // block would promote above the durables.
    let live_session_paths = match snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
        Ok((snap, _)) => snap.live_sessions.into_iter().map(|s| s.session_path).collect(),
        Err(_) => Vec::new(),
    };
    let live_count = live_session_paths.len();

    let output = StartpageLsOutput {
        host,
        home: system_home.display().to_string(),
        durable_count: rows.len(),
        live_count,
        rows,
        live_session_paths,
        warnings,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("startpage ls — host {}  home {}", output.host, output.home);
        println!("durable {}  live {}", output.durable_count, output.live_count);
        if !output.warnings.is_empty() {
            for w in &output.warnings {
                println!("warn: {w}");
            }
        }
        for row in &output.rows {
            let title = row.title.as_deref().unwrap_or("<no title>");
            let mtime = row.modified_epoch_ms;
            println!(
                "{}  {}  {}  {}  {}",
                row.kind_label(),
                row.session_id,
                row.cwd,
                title,
                mtime
            );
        }
    }
    Ok(())
}

fn collect_warnings(home: &PathBuf, rows: &[yggterm_core::startpage::StartpageDurableRow]) -> Vec<String> {
    let mut warnings = Vec::new();
    // Check for gaps the descriptors declare as unscanned.
    for desc in yggterm_core::agent_cli::AGENT_CLIS {
        if let Some(gap) = desc.store_scan_gap {
            warnings.push(format!("{} store not scanned: {}", desc.display_name, gap));
        }
        if desc.session_store_globs.is_empty() && desc.store_scan_gap.is_none() {
            // Historical missing: not declared scanned nor gap — should be either.
            warnings.push(format!(
                "{} has no store globs and no declared gap — sessions will be invisible",
                desc.display_name
            ));
        }
    }
    // Home missing is a warning, not an error.
    if !home.exists() {
        warnings.push(format!("home {} does not exist", home.display()));
    }
    let _ = rows;
    warnings
}

trait KindLabel {
    fn kind_label(&self) -> &'static str;
}

impl KindLabel for yggterm_core::startpage::StartpageDurableRow {
    fn kind_label(&self) -> &'static str {
        yggterm_core::agent_cli::session_kind_label(self.kind)
    }
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname").output().context("hostname")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
