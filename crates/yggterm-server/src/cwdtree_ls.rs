//! `server cwdtree ls` — lie detector for the cwd tree's grouped view.
//!
//! The shell's `load_codex_tree` (née `build_local_cwd_tree`) groups every
//! durable agent-CLI session by its `cwd` into a folder node, then the GUI flattens
//! it into rows with `session_kind`-driven icons. This verb re-derives the same
//! grouping from the stores via the same `AGENT_CLIS` descriptors so a new CLI
//! cannot fall out — the `store_scan_gap` that hid Muse hid it from BOTH the
//! startpage and the tree, and the same Python oracle (`check-cwdtree.py`)
//! walks raw files independently.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;
use yggterm_core::startpage::scan_all_durable_sessions;

#[derive(Debug, serde::Serialize)]
struct CwdtreeLsOutput {
    host: String,
    home: String,
    durable_count: usize,
    group_count: usize,
    live_count: usize,
    groups: Vec<CwdtreeGroup>,
    warnings: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct CwdtreeGroup {
    cwd: String,
    session_count: usize,
    sessions: Vec<CwdtreeRow>,
}

#[derive(Debug, serde::Serialize)]
struct CwdtreeRow {
    session_id: String,
    kind: String,
    icon_glyph: String,
    brand_color: Option<String>,
    title: Option<String>,
    effective_title: Option<String>,
    detail: Option<String>,
    modified_epoch_ms: u128,
    storage_path: String,
    display_path: String,
}

pub fn run_server_cwdtree_ls(store: &yggterm_core::SessionStore, args: &[String]) -> anyhow::Result<()> {
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
    let total_durable = rows.len();
    let total_groups_pre_limit = {
        let mut uniq: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for r in &rows { uniq.insert(r.cwd.as_str()); }
        uniq.len()
    };
    // Cwdtree: groups by cwd, sessions within groups and groups themselves by recency.
    // Recency is live > modified_epoch (so a running session's group leads even if mtime is 0).
    let live_set: std::collections::HashSet<String> = match crate::snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
        Ok((snap, _)) => {
            let mut s = std::collections::HashSet::new();
            for sess in &snap.live_sessions { s.insert(sess.session_path.clone()); }
            // Also index by session_id for display/storage path matching
            for row in &rows { s.insert(row.session_id.clone()); }
            s
        },
        Err(_) => std::collections::HashSet::new(),
    };
    // Helper: effective epoch with live promotion (live sessions use current time as epoch when mtime==0)
    let effective_epoch = |row: &yggterm_core::startpage::StartpageDurableRow| -> (bool, u128) {
        let is_live = live_set.contains(&row.display_path) || live_set.contains(&row.storage_path) || live_set.contains(&row.session_id);
        let epoch = if is_live && row.modified_epoch_ms == 0 {
            // Live but no mtime yet — treat as most recent
            u128::MAX
        } else {
            row.modified_epoch_ms
        };
        (is_live, epoch)
    };
    rows.sort_by(|a, b| {
        let (a_live, a_epoch) = effective_epoch(a);
        let (b_live, b_epoch) = effective_epoch(b);
        b_live.cmp(&a_live)
            .then_with(|| b_epoch.cmp(&a_epoch))
            .then_with(|| a.cwd.cmp(&b.cwd))
    });
    let mut groups_map: BTreeMap<String, Vec<yggterm_core::startpage::StartpageDurableRow>> = BTreeMap::new();
    for row in rows {
        groups_map.entry(row.cwd.clone()).or_default().push(row);
    }
    let mut groups: Vec<(String, Vec<yggterm_core::startpage::StartpageDurableRow>)> = groups_map.into_iter().collect();
    groups.sort_by(|a, b| {
        let a_max_live = a.1.iter().any(|r| effective_epoch(r).0);
        let b_max_live = b.1.iter().any(|r| effective_epoch(r).0);
        let a_max = a.1.iter().map(|r| effective_epoch(r).1).max().unwrap_or(0);
        let b_max = b.1.iter().map(|r| effective_epoch(r).1).max().unwrap_or(0);
        b_max_live.cmp(&a_max_live)
            .then_with(|| b_max.cmp(&a_max))
    });

    let warnings = collect_warnings(&system_home);
    let live_count = match crate::snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
        Ok((snap, _)) => snap.live_sessions.len(),
        Err(_) => 0,
    };

    // Apply limit to total sessions, not groups
    let mut remaining = limit;
    let mut out_groups = Vec::new();
    for (cwd, mut sessions) in groups {
        if remaining == 0 { break; }
        sessions.sort_by(|a, b| {
            let (a_live, a_epoch) = effective_epoch(a);
            let (b_live, b_epoch) = effective_epoch(b);
            b_live.cmp(&a_live).then_with(|| b_epoch.cmp(&a_epoch))
        });
        if sessions.len() > remaining { sessions.truncate(remaining); }
        remaining -= sessions.len();
        let cwdtree_rows = sessions.into_iter().map(|r| CwdtreeRow {
            session_id: r.session_id.clone(),
            kind: r.kind_label().to_string(),
            icon_glyph: yggterm_core::agent_cli::agent_cli_descriptor(r.kind).map(|d| d.icon_glyph.to_string()).unwrap_or_else(|| "  ".to_string()),
            brand_color: yggterm_core::agent_cli::agent_cli_descriptor(r.kind).map(|d| d.brand_color.to_string()),
            title: r.title.clone(),
            effective_title: r.effective_title.clone(),
            detail: r.detail.clone(),
            modified_epoch_ms: r.modified_epoch_ms,
            storage_path: r.storage_path.clone(),
            display_path: r.display_path.clone(),
        }).collect::<Vec<_>>();
        let count = cwdtree_rows.len();
        out_groups.push(CwdtreeGroup { cwd: cwd.clone(), session_count: count, sessions: cwdtree_rows });
    }

    let output = CwdtreeLsOutput {
        host,
        home: system_home.display().to_string(),
        durable_count: total_durable,
        group_count: total_groups_pre_limit,
        live_count,
        groups: out_groups,
        warnings,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("cwdtree ls — host {}  home {}", output.host, output.home);
        println!("durable {} groups {} live {}", output.durable_count, output.group_count, output.live_count);
        for w in &output.warnings { println!("warn: {w}"); }
        for group in &output.groups {
            println!("{}  ({} sessions)", group.cwd, group.session_count);
            for row in &group.sessions {
                let title = row.effective_title.as_deref().or(row.title.as_deref()).unwrap_or("<no title>");
                println!("  {} {} {} {}", row.icon_glyph, row.kind, &row.session_id[..8.min(row.session_id.len())], title.chars().take(60).collect::<String>());
            }
        }
    }
    Ok(())
}

fn collect_warnings(home: &PathBuf) -> Vec<String> {
    let mut warnings = Vec::new();
    for desc in yggterm_core::agent_cli::AGENT_CLIS {
        if let Some(gap) = desc.store_scan_gap {
            warnings.push(format!("{} store not scanned: {}", desc.display_name, gap));
        }
        if desc.session_store_globs.is_empty() && desc.store_scan_gap.is_none() {
            warnings.push(format!("{} has no store globs and no declared gap — sessions will be invisible", desc.display_name));
        }
    }
    if !home.exists() { warnings.push(format!("home {} does not exist", home.display())); }
    warnings
}

trait KindLabel { fn kind_label(&self) -> &'static str; }
impl KindLabel for yggterm_core::startpage::StartpageDurableRow {
    fn kind_label(&self) -> &'static str { yggterm_core::agent_cli::session_kind_label(self.kind) }
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname").output().context("hostname")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
