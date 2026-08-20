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
    /// The `--limit` in force for this reply (default 200).
    limit: usize,
    /// True when `groups` carries FEWER groups than `group_count` reports.
    /// ⛔ Without this a reader sees `group_count: 527` beside 8 groups and has
    /// no way to tell a small tree from a truncated one.
    truncated: bool,
    groups: Vec<CwdtreeGroup>,
    warnings: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct CwdtreeGroup {
    cwd: String,
    host: String,
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
    // ⛔ `--help` must answer BEFORE the scan. It used to fall through to the
    // verb, so asking a 1700-session store how to call it ran a full walk and
    // printed the sessions — the one output that cannot be mistaken for help.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("server cwdtree ls — groups the durable sessions by cwd exactly as the sidebar tree does");
        println!();
        println!("USAGE: server cwdtree ls [--json] [--limit N]");
        println!("  --json       machine-readable output");
        println!("  --limit N    cap the rows/groups printed (default 200)");
        println!("  --help, -h   print this and exit without scanning");
        println!();
        println!("The reply carries `limit` and `truncated` so a capped page is");
        println!("never mistaken for the whole store. Oracles: scripts/check-cwdtree.py");
        return Ok(());
    }
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
    // Track host per row for SSOT grouping (host-aware, like GUI's __remote_folder__/host/cwd)
    let local_host = host.clone();
    let mut rows_with_host: Vec<(yggterm_core::startpage::StartpageDurableRow, String)> = rows.into_iter().map(|r| (r, local_host.clone())).collect();
    // Also include remote durable rows when daemon has snapshot (so cwdtree matches GUI's fleet view)
    let (snapshot_opt, live_set) = {
        let (snap_opt, mut live_set) = match crate::snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
            Ok((snap, _)) => {
                let mut s = std::collections::HashSet::new();
                for sess in &snap.live_sessions { s.insert(sess.session_path.clone()); s.insert(sess.id.clone()); }
                // Also add remote session_ids for live promotion
                for m in &snap.remote_machines { for sess in &m.sessions { s.insert(sess.session_id.clone()); s.insert(sess.session_path.clone()); } }
                (Some(snap), s)
            },
            Err(_) => (None, std::collections::HashSet::new()),
        };
        // Add local rows to live_set for promotion
        for (row, _) in &rows_with_host { live_set.insert(row.session_id.clone()); live_set.insert(row.display_path.clone()); }
        (snap_opt, live_set)
    };
    let mut remote_rows_with_host: Vec<(yggterm_core::startpage::StartpageDurableRow, String)> = Vec::new();
    if let Some(snap) = snapshot_opt.as_ref() {
        if let Ok(snap_json) = serde_json::to_value(snap) {
            let (rrows, _) = build_remote_durable_rows_for_cwdtree(&Some(snap_json.clone()));
            // rrows lost host; rebuild with host from snapshot JSON
            if let Some(machines) = snap_json.get("remote_machines").and_then(|v| v.as_array()) {
                for machine in machines {
                    let h = machine.get("machine_key").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    if let Some(sessions) = machine.get("sessions").and_then(|v| v.as_array()) {
                        for sess in sessions {
                            if let Some(sid) = sess.get("session_id").and_then(|v| v.as_str()) {
                                if let Some(row) = rrows.iter().find(|r| r.session_id == sid) {
                                    remote_rows_with_host.push((row.clone(), h.clone()));
                                }
                            }
                        }
                    }
                }
            }
            // Deduplicate remote rows that were already matched; add any unmatched (should not happen)
            for r in rrows {
                if !remote_rows_with_host.iter().any(|(rr,_)| rr.session_id == r.session_id) {
                    remote_rows_with_host.push((r, "unknown".to_string()));
                }
            }
        }
    }
    // Merge local + remote, dedup by session_id
    {
        let mut seen: std::collections::HashSet<String> = rows_with_host.iter().map(|(r,_)| r.session_id.clone()).collect();
        for (r, h) in remote_rows_with_host { if seen.insert(r.session_id.clone()) { rows_with_host.push((r, h)); } }
    }
    // Inject live sessions that have no durable transcript yet (dual presence)
    {
        use yggterm_core::SessionKind;
        // Fetch browser rows for live cwd mapping (like startpage faithful)
        let app_rows_json = std::process::Command::new("sh")
            .arg("-c")
            .arg("~/.local/bin/yggterm server app rows 2>/dev/null || ~/.local/bin/yggterm-headless server app rows 2>/dev/null || yggterm server app rows 2>/dev/null".to_string())
            .output()
            .ok()
            .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        if let Some(v) = app_rows_json.as_ref() {
            let data = v.get("data").unwrap_or(v);
            if let Some(browser_rows) = data.get("rows").and_then(|r| r.as_array()) {
                let mut seen: std::collections::HashSet<String> = rows_with_host.iter().map(|(r,_)| r.session_id.clone()).collect();
                for brow in browser_rows {
                    if brow.get("kind").and_then(|k| k.as_str()) != Some("Session") { continue; }
                    if brow.get("document_kind").and_then(|d| d.as_str()).is_some() { continue; }
                    let host_label = brow.get("host_label").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let icon_kind = brow.get("icon_kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if host_label == "local-shell" || icon_kind == "terminal" { continue; }
                    let full_path = brow.get("full_path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let is_live_browser = live_set.contains(&full_path)
                        || brow.get("live_member").and_then(|v| v.as_bool()).unwrap_or(false)
                        || brow.get("live_keep_alive").and_then(|v| v.as_bool()).unwrap_or(false);
                    if !is_live_browser { continue; }
                    let sid = brow.get("session_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                    if sid.is_empty() || seen.contains(&sid) { continue; }
                    let cwd = brow.get("session_cwd").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_else(|| "/home/user".to_string()));
                    let label = brow.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()).filter(|s| !s.trim().is_empty());
                    let kind = match icon_kind.as_str() {
                        "claude-code" => SessionKind::ClaudeCode,
                        "codex" => SessionKind::Codex,
                        "muse" => SessionKind::Muse,
                        "antigravity" => SessionKind::Antigravity,
                        "pi" => SessionKind::Pi,
                        "opencode" => SessionKind::OpenCode,
                        "qwen" => SessionKind::QwenCode,
                        "kimi" => SessionKind::Kimi,
                        "grok" => SessionKind::GrokBuild,
                        _ => {
                            // Use the registry-derived parser rather than hand-written
                            // prefix checks: it knows all remote schemes (including
                            // codex-runtime:// and remote-muse://) and stays correct
                            // when a new CLI is added. The hand-written fallback
                            // mis-classified codex-runtime as Muse and local:// as
                            // Muse, producing the kind mismatch seen on oc (019da16a
                            // codex labelled as claude-code).
                            yggterm_core::agent_scheme::session_kind_for_path(&full_path)
                                .or_else(|| yggterm_core::agent_cli::agent_cli_for_store_path(&full_path).map(|d| d.kind))
                                .unwrap_or_else(|| {
                                    if full_path.starts_with("remote-cc://") { SessionKind::ClaudeCode }
                                    else if full_path.starts_with("remote-session://") || full_path.starts_with("codex-runtime://") { SessionKind::Codex }
                                    else if full_path.starts_with("remote-muse://") { SessionKind::Muse }
                                    else { SessionKind::ClaudeCode }
                                })
                        }
                    };
                    let display_path = full_path.clone();
                    let storage_path = brow.get("session_cwd").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                    let row = yggterm_core::startpage::StartpageDurableRow {
                        session_id: sid.clone(),
                        cwd: cwd.clone(),
                        title: label.clone(),
                        generated_title: None,
                        effective_title: label.clone(),
                        detail: None,
                        kind,
                        modified_epoch_ms: now_ms,
                        storage_path: if storage_path.is_empty() { display_path.clone() } else { storage_path },
                        display_path: display_path.clone(),
                    };
                    // Host for grouping: use host_label if available, else parse from full_path
                    let host_key = if !host_label.is_empty() && host_label != "live" { host_label.clone() } else if full_path.starts_with("remote-cc://") {
                        full_path.split('/').nth(2).unwrap_or("unknown").to_string()
                    } else if full_path.starts_with("remote-session://") {
                        full_path.split('/').nth(2).unwrap_or("unknown").to_string()
                    } else { host.clone() };
                    rows_with_host.push((row, host_key));
                    seen.insert(sid);
                }
            }
        }
    }
    let total_durable = rows_with_host.len();
    let total_groups_pre_limit = {
        let mut uniq: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        for (r, h) in &rows_with_host { uniq.insert((h.clone(), r.cwd.clone())); }
        uniq.len()
    };
    // Cwdtree: groups by cwd, sessions within groups and groups themselves by recency.
    // Recency is live > modified_epoch (so a running session's group leads even if mtime is 0).
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
    rows_with_host.sort_by(|(a_row, a_host), (b_row, b_host)| {
        let (a_live, a_epoch) = effective_epoch(a_row);
        let (b_live, b_epoch) = effective_epoch(b_row);
        b_live.cmp(&a_live)
            .then_with(|| b_epoch.cmp(&a_epoch))
            .then_with(|| a_host.cmp(b_host))
            .then_with(|| a_row.cwd.cmp(&b_row.cwd))
    });
    let mut groups_map: BTreeMap<(String, String), Vec<yggterm_core::startpage::StartpageDurableRow>> = BTreeMap::new();
    for (row, host_key) in rows_with_host {
        groups_map.entry((host_key, row.cwd.clone())).or_default().push(row);
    }
    let mut groups: Vec<((String, String), Vec<yggterm_core::startpage::StartpageDurableRow>)> = groups_map.into_iter().collect();
    groups.sort_by(|a, b| {
        let a_max_live = a.1.iter().any(|r| effective_epoch(r).0);
        let b_max_live = b.1.iter().any(|r| effective_epoch(r).0);
        let a_max = a.1.iter().map(|r| effective_epoch(r).1).max().unwrap_or(0);
        let b_max = b.1.iter().map(|r| effective_epoch(r).1).max().unwrap_or(0);
        b_max_live.cmp(&a_max_live)
            .then_with(|| b_max.cmp(&a_max))
            .then_with(|| a.0.cmp(&b.0))
    });

    let warnings = collect_warnings(&system_home);
    let live_count = match crate::snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
        Ok((snap, _)) => snap.live_sessions.len(),
        Err(_) => 0,
    };

    // Apply limit to total sessions, not groups
    let mut remaining = limit;
    let mut out_groups = Vec::new();
    for ((host_key, cwd), mut sessions) in groups {
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
        out_groups.push(CwdtreeGroup { cwd: cwd.clone(), host: host_key.clone(), session_count: count, sessions: cwdtree_rows });
    }

    let output = CwdtreeLsOutput {
        host,
        home: system_home.display().to_string(),
        durable_count: total_durable,
        group_count: total_groups_pre_limit,
        live_count,
        limit,
        truncated: out_groups.len() < total_groups_pre_limit,
        groups: out_groups,
        warnings,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("cwdtree ls — host {}  home {}", output.host, output.home);
        println!("durable {} groups {} live {}", output.durable_count, output.group_count, output.live_count);
        if output.truncated {
            println!("note: showing {} of {} groups (--limit {})", output.groups.len(), output.group_count, output.limit);
        }
        for w in &output.warnings { println!("warn: {w}"); }
        for group in &output.groups {
            println!("{}:{}  ({} sessions)", group.host, group.cwd, group.session_count);
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
        // A CLI whose store no glob can express (one SQLite DB, an md5-bucketed
        // tree) is scanned by a dedicated scanner, not missing. Asking only about
        // globs reported OpenCode and Kimi as invisible on every single run.
        if desc.session_store_globs.is_empty()
            && desc.store_scan_gap.is_none()
            && !yggterm_core::startpage::kind_has_dedicated_scanner(desc.kind)
        {
            warnings.push(format!(
                "{} has no store globs, no dedicated scanner and no declared gap — sessions will be invisible",
                desc.display_name
            ));
        }
    }
    if !home.exists() { warnings.push(format!("home {} does not exist", home.display())); }
    warnings
}

trait KindLabel { fn kind_label(&self) -> &'static str; }
impl KindLabel for yggterm_core::startpage::StartpageDurableRow {
    fn kind_label(&self) -> &'static str { yggterm_core::agent_cli::session_kind_label(self.kind) }
}

fn build_remote_durable_rows_for_cwdtree(snapshot_json: &Option<serde_json::Value>) -> (Vec<yggterm_core::startpage::StartpageDurableRow>, usize) {
    let mut rows = Vec::new();
    let mut total = 0;
    let Some(machines) = snapshot_json.as_ref().and_then(|v| v.get("data")).or(snapshot_json.as_ref()).and_then(|d| d.get("remote_machines")).and_then(|v| v.as_array()) else {
        return (rows, total);
    };
    for machine in machines {
        let Some(sessions) = machine.get("sessions").and_then(|v| v.as_array()) else { continue; };
        total += sessions.len();
        for sess in sessions {
            let session_id = sess.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if session_id.is_empty() { continue; }
            let cwd = sess.get("cwd").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let title_hint = sess.get("title_hint").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let modified_epoch = sess.get("modified_epoch").and_then(|v| v.as_i64()).unwrap_or(0);
            let storage_path = sess.get("storage_path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let session_path = sess.get("session_path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let title = if yggterm_core::looks_like_generated_fallback_title(&title_hint) || yggterm_core::looks_like_low_signal_generated_copy(&title_hint) {
                None
            } else if title_hint.trim().is_empty() { None } else { Some(title_hint.clone()) };
            let kind = yggterm_core::agent_scheme::session_kind_for_path(&session_path).unwrap_or(yggterm_core::SessionKind::Codex);
            let display_path = if session_path.is_empty() { format!("remote-session://{}", session_id) } else { session_path.clone() };
            rows.push(yggterm_core::startpage::StartpageDurableRow {
                session_id: session_id.clone(),
                cwd: if cwd.is_empty() { "/".to_string() } else { cwd },
                title: title.clone(),
                generated_title: None,
                effective_title: title.clone(),
                detail: None,
                kind,
                modified_epoch_ms: (modified_epoch as u128) * 1000,
                storage_path: if storage_path.is_empty() { display_path.clone() } else { storage_path },
                display_path,
            });
        }
    }
    (rows, total)
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname").output().context("hostname")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
