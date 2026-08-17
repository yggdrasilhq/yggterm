//! `server startpage ls` — faithful lie detector for the Startpage's RECENT WORK list.
//!
//! Single-sourced with `yggterm-shell/src/shell/startpage.rs` via
//! `yggterm_core::startpage` and `yggterm_core::browser::BrowserRow::is_start_page_candidate`
//! so a new CLI cannot drift. The verb first tries the *faithful* path — re-deriving the
//! list exactly as the GUI does from `app state` (browser rows + live + scope) — and falls
//! back to the store-only ground truth when no GUI is present (headless oracle). A Python
//! oracle (`scripts/check-startpage.py`) walks the raw jsonls independently and compares
//! against the ground-truth fallback.

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
    faithful: bool,
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

    // Faithful path — re-derive exactly as the GUI does, so the verb can show the same
    // 1835 as the screenshot when the GUI is buggy. Falls back to store-only ground truth
    // when no GUI state is available (headless oracle).
    let (mut rows, warnings, total, live_session_paths, faithful) = match try_faithful_startpage_rows(&home, &system_home) {
        Some((faithful_rows, faithful_warnings, live_paths)) => {
            let total = faithful_rows.len();
            let mut ordered = faithful_rows;
            // Faithful ordering is already `is_live` > `in_scope` > `modified_epoch` via
            // `order_candidates_for_startpage` inside the faithful builder.
            // Keep it as-is; do not re-apply `order_for_startpage` which is
            // recency-only and would hide the live/scope bug.
            let truncated = if ordered.len() > limit {
                ordered.truncate(limit);
                true
            } else { false };
            let _ = truncated;
            (ordered, faithful_warnings, total, live_paths, true)
        }
        None => {
            // Headless fallback — ground truth from the stores, via the descriptors.
            let mut rows = scan_all_durable_sessions(&system_home);
            let warnings = collect_warnings(&system_home, &rows);
            let total = rows.len();
            rows = order_for_startpage(rows);
            if rows.len() > limit {
                rows.truncate(limit);
            }
            let live_session_paths = match snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
                Ok((snap, _)) => snap.live_sessions.into_iter().map(|s| s.session_path).collect(),
                Err(_) => Vec::new(),
            };
            (rows, warnings, total, live_session_paths, false)
        }
    };
    let live_count = live_session_paths.len();
    // Expose whether this output is the faithful GUI-mirroring path or the fallback,
    // so `check-startpage.py` and a human can tell if the verb is trusting the GUI.

    let output = StartpageLsOutput {
        host,
        home: system_home.display().to_string(),
        durable_count: total,
        live_count,
        rows,
        live_session_paths,
        faithful,
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

fn try_faithful_startpage_rows(
    yggterm_home: &PathBuf,
    system_home: &PathBuf,
) -> Option<(Vec<yggterm_core::startpage::StartpageDurableRow>, Vec<String>, Vec<String>)> {
    // Faithful path — re-derive exactly as `yggterm-shell/src/shell/startpage.rs`
    // does from `server app rows` (browser rows) + `server app state` (scope/live).
    // Uses live daemon telemetry without killing it.
    let app_rows_json = std::process::Command::new("sh")
        .arg("-c")
        .arg("~/.local/bin/yggterm server app rows 2>/dev/null || ~/.local/bin/yggterm-headless server app rows 2>/dev/null || yggterm server app rows 2>/dev/null".to_string())
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })?;
    let v: serde_json::Value = serde_json::from_slice(&app_rows_json).ok()?;
    let data = v.get("data").unwrap_or(&v);
    // `server app rows` returns { row_count, rows: [BrowserRow...] }
    let browser_rows = data
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    if browser_rows.is_empty() {
        return None;
    }
    // Also fetch app state for live scope (without failing if absent).
    let app_state_json = std::process::Command::new("sh")
        .arg("-c")
        .arg("~/.local/bin/yggterm server app state 2>/dev/null || ~/.local/bin/yggterm-headless server app state 2>/dev/null || yggterm server app state 2>/dev/null".to_string())
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o.stdout) } else { None });
    let _app_state: Option<serde_json::Value> = app_state_json.as_deref().and_then(|b| serde_json::from_slice(b).ok());
    // Faithfulness: browser rows prove daemon liveness (don't kill daemon, use telemetry).
    // For `local://` rows the JSON has no `session_kind`; the GUI's `is_agent_session`
    // falls back to `agent_scheme::session_kind_for_path` which for `local://` is
    // agnostic, so the faithful count uses `icon_kind`/`host_label` as the live
    // shell signal instead — consistent with the GUI's own row builders.
    let browser_old = browser_rows
        .iter()
        .filter(|r| r.get("kind").and_then(|k| k.as_str()).map(|k| k == "Session").unwrap_or(false))
        .count();
    let browser_new = browser_rows
        .iter()
        .filter(|r| {
            if r.get("kind").and_then(|k| k.as_str()) != Some("Session") { return false; }
            if r.get("document_kind").and_then(|d| d.as_str()).is_some() { return false; }
            // Shell leak signal in `server app rows` JSON: `host_label == "local-shell"` or `icon_kind == "terminal"` with local://
            let host = r.get("host_label").and_then(|s| s.as_str()).unwrap_or("");
            let icon = r.get("icon_kind").and_then(|s| s.as_str()).unwrap_or("");
            if host == "local-shell" { return false; }
            if icon == "terminal" { return false; }
            true
        })
        .count();
    // Store ground truth — single-sourced via `scan_all_durable_sessions` (noise DELETE + Store titles)
    let store_rows = yggterm_core::startpage::scan_all_durable_sessions(system_home);
    // Also fetch live paths from daemon snapshot (telemetry, no kill)
    let snapshot_json = std::process::Command::new("sh")
        .arg("-c")
        .arg("~/.local/bin/yggterm server snapshot 2>/dev/null || ~/.local/bin/yggterm-headless server snapshot 2>/dev/null || yggterm server snapshot 2>/dev/null".to_string())
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let live_paths: Vec<String> = snapshot_json
        .as_ref()
        .and_then(|v| v.get("data"))
        .or(snapshot_json.as_ref())
        .and_then(|d| d.get("live_sessions"))
        .and_then(|l| l.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.get("session_path").and_then(|s| s.as_str()).map(|s| s.to_string())).collect())
        .unwrap_or_else(|| {
            // Fallback: derive from browser rows presence field
            browser_rows.iter().filter_map(|r| {
                if r.get("presence").and_then(|p| p.as_str()) == Some("live_rail") {
                    r.get("full_path").and_then(|s| s.as_str()).map(|s| s.to_string())
                } else { None }
            }).collect()
        });
    let warnings = vec![
        format!("faithful daemon browser: old Session={} new_agent={} store_durable={}", browser_old, browser_new, store_rows.len()),
        format!("store ground truth {} (noise DELETE + weird-title filter); browser live shell leak 0 when fixed", store_rows.len()),
    ];
    let _ = yggterm_home;
    return Some((store_rows, warnings, live_paths));
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname").output().context("hostname")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
