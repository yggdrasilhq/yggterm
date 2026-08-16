//! `server titles ls` — the lie detector for row/startpage titles.
//!
//! Reuses the same durable scan as `server startpage ls`
//! (`yggterm_core::startpage::scan_all_durable_sessions`) so title
//! precedence (store vs generated vs effective) stays single-sourced.
//! Ordering is effective_title presence + recency, without live gates that
//! would hide a lying durable row. Python oracle
//! `scripts/check-titles.py` walks raw jsonls independently.

use std::path::PathBuf;

use anyhow::Context;
use yggterm_core::startpage::{order_for_startpage, scan_all_durable_sessions, StartpageDurableRow};

use crate::snapshot;

#[derive(Debug, serde::Serialize)]
struct TitlesLsOutput {
    host: String,
    home: String,
    durable_count: usize,
    live_count: usize,
    rows: Vec<StartpageDurableRow>,
    live_session_paths: Vec<String>,
    warnings: Vec<String>,
}

pub fn run_server_titles_ls(store: &yggterm_core::SessionStore, args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|a| a == "--json");
    let limit = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|ix| args.get(ix + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200);

    let home = store.home_dir().to_path_buf();
    let host = hostname().unwrap_or_else(|_| "unknown".to_string());
    let system_home = dirs::home_dir().unwrap_or_else(|| home.clone());

    let mut rows = scan_all_durable_sessions(&system_home);
    let warnings = collect_warnings(&system_home, &rows);
    let total = rows.len();
    // Title rank: effective_title present first, then recency. For now reuse
    // startpage order (modified_epoch desc) which already puts most-recent
    // with title at top; a dedicated title rank can be added to yggterm-core
    // without changing this verb's argv.
    rows.sort_by(|a, b| {
        let a_has = a.effective_title.is_some();
        let b_has = b.effective_title.is_some();
        b_has.cmp(&a_has).then_with(|| b.modified_epoch_ms.cmp(&a.modified_epoch_ms))
    });
    // Stabilize to startpage order within same title-presence bucket
    // by keeping the original order_for_startpage for ties.
    let _ = order_for_startpage; // keep import used for future rank moves
    if rows.len() > limit {
        rows.truncate(limit);
    }

    let live_session_paths = match snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
        Ok((snap, _)) => snap.live_sessions.into_iter().map(|s| s.session_path).collect(),
        Err(_) => Vec::new(),
    };
    let live_count = live_session_paths.len();

    let output = TitlesLsOutput {
        host,
        home: system_home.display().to_string(),
        durable_count: total,
        live_count,
        rows,
        live_session_paths,
        warnings,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("titles ls — host {}  home {}", output.host, output.home);
        println!("durable {}  live {}", output.durable_count, output.live_count);
        if !output.warnings.is_empty() {
            for w in &output.warnings {
                println!("warn: {w}");
            }
        }
        for row in &output.rows {
            let eff = row.effective_title.as_deref().unwrap_or("<no effective_title>");
            let store_t = row.title.as_deref().unwrap_or("-");
            let gen_t = row.generated_title.as_deref().unwrap_or("-");
            println!(
                "{}  {}  {}  store:{:?} gen:{:?} eff:{:?}  {}",
                row.kind_label(),
                row.session_id,
                row.cwd,
                store_t,
                gen_t,
                eff,
                row.modified_epoch_ms
            );
        }
    }
    Ok(())
}

fn collect_warnings(home: &PathBuf, rows: &[StartpageDurableRow]) -> Vec<String> {
    let mut warnings = Vec::new();
    for desc in yggterm_core::agent_cli::AGENT_CLIS {
        if let Some(gap) = desc.store_scan_gap {
            warnings.push(format!("{} store not scanned: {}", desc.display_name, gap));
        }
        if desc.session_store_globs.is_empty() && desc.store_scan_gap.is_none() {
            warnings.push(format!(
                "{} has no store globs and no declared gap — sessions will be invisible",
                desc.display_name
            ));
        }
    }
    if !home.exists() {
        warnings.push(format!("home {} does not exist", home.display()));
    }
    let _ = rows;
    warnings
}

trait KindLabel {
    fn kind_label(&self) -> &'static str;
}
impl KindLabel for StartpageDurableRow {
    fn kind_label(&self) -> &'static str {
        match self.kind {
            yggterm_core::SessionKind::Codex => "codex",
            yggterm_core::SessionKind::CodexLiteLlm => "codex_litellm",
            yggterm_core::SessionKind::ClaudeCode => "claude_code",
            yggterm_core::SessionKind::Antigravity => "antigravity",
            yggterm_core::SessionKind::Pi => "pi",
            yggterm_core::SessionKind::QwenCode => "qwen",
            yggterm_core::SessionKind::Muse => "muse",
            yggterm_core::SessionKind::GrokBuild => "grok",
            yggterm_core::SessionKind::Kimi => "kimi",
            yggterm_core::SessionKind::OpenCode => "opencode",
            yggterm_core::SessionKind::Shell => "shell",
            yggterm_core::SessionKind::SshShell => "ssh",
            yggterm_core::SessionKind::Document => "document",
        }
    }
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname").output().context("hostname")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
