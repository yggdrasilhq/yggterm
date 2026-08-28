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
use yggterm_core::{SessionStore, append_trace_event, startpage::scan_all_durable_sessions};

use crate::snapshot;

/// Startpage's GUI witnesses are optional. A busy or absent GUI must not make
/// the store oracle wait behind the app-control plane's ordinary 15s command
/// budget, much less launch another CLI that can start or hand off a daemon.
const STARTPAGE_APP_OBSERVER_TIMEOUT_MS: u64 = 1_000;

#[derive(Debug, serde::Serialize)]
struct StartpageLsOutput {
    host: String,
    home: String,
    durable_count: usize,
    live_count: usize,
    /// The `--limit` in force for this reply (default 200).
    limit: usize,
    /// True when `rows` carries FEWER rows than `durable_count` reports, so no
    /// reader can mistake a truncated page for the whole store.
    truncated: bool,
    rows: Vec<yggterm_core::startpage::StartpageDurableRow>,
    live_session_paths: Vec<String>,
    faithful: bool,
    warnings: Vec<String>,
}

pub fn run_server_startpage_ls(store: &SessionStore, args: &[String]) -> anyhow::Result<()> {
    // ⛔ `--help` must answer BEFORE the scan. It used to fall through to the
    // verb, so asking a 1700-session store how to call it ran a full walk and
    // printed the sessions — the one output that cannot be mistaken for help.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("server startpage ls — re-derives the startpage RECENT WORK list from the stores");
        println!();
        println!("USAGE: server startpage ls [--json] [--limit N]");
        println!("  --json       machine-readable output");
        println!("  --limit N    cap the rows/groups printed (default 200)");
        println!("  --help, -h   print this and exit without scanning");
        println!();
        println!("The reply carries `limit` and `truncated` so a capped page is");
        println!("never mistaken for the whole store. Oracles: scripts/check-startpage.py");
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
    // System HOME is where the CLI stores live (~/.codex, ~/.claude, …),
    // not YGGTERM_HOME (which is ~/.yggterm). The earlier version passed
    // the yggterm home and therefore walked the wrong tree (0 rows) — and the
    // GUI's local tree later repeated the same mistake, which is why the
    // resolution now has ONE owner in core.
    let system_home = yggterm_core::startpage::agent_store_home(&home);

    // Faithful path — re-derive exactly as the GUI does, so the verb can show the same
    // 1835 as the screenshot when the GUI is buggy. Falls back to store-only ground truth
    // when no GUI state is available (headless oracle).
    let (mut rows, warnings, total, live_session_paths, faithful) =
        match try_faithful_startpage_rows(&home, &system_home) {
            Some((faithful_rows, faithful_warnings, live_paths)) => {
                let total = faithful_rows.len();
                let mut ordered = faithful_rows;
                // Faithful builder already applied is_live > in_scope > modified_epoch
                // via order_candidates_for_startpage — keep it, do not re-rank recency-only.
                let truncated = if ordered.len() > limit {
                    ordered.truncate(limit);
                    true
                } else {
                    false
                };
                let _ = truncated;
                (ordered, faithful_warnings, total, live_paths, true)
            }
            None => {
                // Headless fallback — ground truth from the stores, via the descriptors.
                // Must use the same ranking as the GUI (live > scope > recency), not
                // recency-only. Also include remote sessions from snapshot when available
                // so headless matches GUI's fleet view (otherwise 73 vs 0).
                let mut rows = scan_all_durable_sessions(&system_home);
                // Also pull remote durable rows from snapshot if daemon has them
                let (snapshot_opt, live_paths_headless) =
                    match snapshot(&crate::server_cli::cli_server_endpoint(&home)) {
                        Ok((snap, _)) => {
                            let live = snap
                                .live_sessions
                                .iter()
                                .map(|s| s.session_path.clone())
                                .collect::<Vec<_>>();
                            (Some(snap), live)
                        }
                        Err(_) => (None, Vec::new()),
                    };
                let mut remote_rows = Vec::new();
                let mut remote_total = 0;
                if let Some(snap) = snapshot_opt.as_ref() {
                    // Build snapshot_json for reuse by helper
                    let snap_json = serde_json::to_value(snap).ok();
                    let (rrows, rtotal) = build_remote_durable_rows(&snap_json);
                    remote_rows = rrows;
                    remote_total = rtotal;
                }
                let warnings = collect_warnings(&system_home, &rows);
                let _ = remote_total;
                let live_set: std::collections::HashSet<String> =
                    live_paths_headless.iter().cloned().collect();
                // Also collect remote epochs for ordering
                let mut remote_epoch_by_id: std::collections::HashMap<String, i64> =
                    std::collections::HashMap::new();
                if let Some(snap) = snapshot_opt.as_ref() {
                    for m in &snap.remote_machines {
                        for s in &m.sessions {
                            remote_epoch_by_id.insert(s.session_id.clone(), s.modified_epoch);
                        }
                    }
                }
                let mut all_rows_map: std::collections::HashMap<
                    String,
                    yggterm_core::startpage::StartpageDurableRow,
                > = std::collections::HashMap::new();
                for row in rows {
                    all_rows_map.insert(row.session_id.clone(), row);
                }
                for row in remote_rows {
                    all_rows_map.entry(row.session_id.clone()).or_insert(row);
                }
                let all_rows: Vec<yggterm_core::startpage::StartpageDurableRow> =
                    all_rows_map.into_values().collect();
                // ⛔ `durable_count` is the size of the DEDUPLICATED set, taken here.
                // It used to be `rows.len() + remote_total`, computed BEFORE this
                // merge, so it counted every session that exists both locally and on
                // a remote machine twice: 1677 local + 717 remote read as 2394 while
                // the row list it shipped alongside held 1742. One question, two
                // answers, in the same JSON object.
                let total = all_rows.len();
                let mut candidates: Vec<(
                    yggterm_core::startpage::StartpageDurableRow,
                    bool,
                    bool,
                    i64,
                    String,
                    usize,
                )> = Vec::new();
                for (idx, row) in all_rows.into_iter().enumerate() {
                    let is_live = live_set.contains(&row.display_path)
                        || live_set.contains(&row.storage_path)
                        || remote_epoch_by_id.contains_key(&row.session_id);
                    let epoch_ms = remote_epoch_by_id
                        .get(&row.session_id)
                        .map(|e| (*e as u128) * 1000)
                        .unwrap_or(row.modified_epoch_ms);
                    let epoch = i64::try_from(epoch_ms / 1000).unwrap_or(0);
                    candidates.push((row, is_live, true, epoch, String::new(), idx));
                }
                let mut rows = yggterm_core::startpage::order_candidates_for_startpage(candidates);
                if rows.len() > limit {
                    rows.truncate(limit);
                }
                let live_session_paths = live_paths_headless;
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
        limit,
        truncated: rows.len() < total,
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
        println!(
            "durable {}  live {}",
            output.durable_count, output.live_count
        );
        if output.truncated {
            println!(
                "note: showing {} of {} rows (--limit {})",
                output.rows.len(),
                output.durable_count,
                output.limit
            );
        }
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

fn collect_warnings(
    home: &PathBuf,
    rows: &[yggterm_core::startpage::StartpageDurableRow],
) -> Vec<String> {
    let mut warnings = Vec::new();
    // Check for gaps the descriptors declare as unscanned.
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
) -> Option<(
    Vec<yggterm_core::startpage::StartpageDurableRow>,
    Vec<String>,
    Vec<String>,
)> {
    // Faithful path — re-derive exactly as `yggterm-shell/src/shell/startpage.rs`
    // does from `server app rows` (browser rows) + `server app state` (scope/live).
    // Uses live daemon telemetry without killing it. Live/session ordering is
    // is_live > in_scope > modified_epoch > started_at > insertion_index
    // via order_candidates_for_startpage — same as the GUI.
    let observer_started = std::time::Instant::now();
    // Ask the already-running GUI directly. The old implementation spawned a
    // nested `yggterm server app rows` process, which could in turn negotiate a
    // daemon startup/handoff. A read-only witness then became a daemon writer.
    let data = crate::request_app_control(
        yggterm_home,
        crate::AppControlCommand::DescribeRows,
        STARTPAGE_APP_OBSERVER_TIMEOUT_MS,
    )
    .ok()
    .filter(|response| response.error.is_none())
    .and_then(|response| response.data)?;
    // `server app rows` returns { row_count, rows: [BrowserRow...] }
    let browser_rows = data
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    if browser_rows.is_empty() {
        return None;
    }
    // Fetch snapshot for live + remote recency + epoch index
    // The daemon endpoint was resolved by this process from this store. Read it
    // directly, exactly like cwdtree/titles do; never search PATH or run a
    // possibly stale binary as a child.
    let snapshot_json = snapshot(&crate::server_cli::cli_server_endpoint(yggterm_home))
        .ok()
        .and_then(|(snapshot, _)| serde_json::to_value(snapshot).ok());
    let live_paths: Vec<String> = snapshot_json
        .as_ref()
        .and_then(|v| v.get("data"))
        .or(snapshot_json.as_ref())
        .and_then(|d| d.get("live_sessions"))
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    v.get("session_path")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_else(|| {
            browser_rows
                .iter()
                .filter_map(|r| {
                    if r.get("presence").and_then(|p| p.as_str()) == Some("live_rail") {
                        r.get("full_path")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        });
    let live_set: std::collections::HashSet<String> = live_paths.iter().cloned().collect();
    // Remote epoch index: session_id -> modified_epoch (from remote_machines)
    let mut remote_epoch_by_id: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    if let Some(machines) = snapshot_json
        .as_ref()
        .and_then(|v| v.get("data"))
        .or(snapshot_json.as_ref())
        .and_then(|d| d.get("remote_machines"))
        .and_then(|v| v.as_array())
    {
        for m in machines {
            if let Some(sessions) = m.get("sessions").and_then(|s| s.as_array()) {
                for s in sessions {
                    if let (Some(id), Some(epoch)) = (
                        s.get("session_id").and_then(|v| v.as_str()),
                        s.get("modified_epoch").and_then(|v| v.as_i64()),
                    ) {
                        remote_epoch_by_id.insert(id.to_string(), epoch);
                    }
                }
            }
        }
    }
    // Also fetch app state for scope (selected row)
    let app_state = crate::request_app_control(
        yggterm_home,
        crate::AppControlCommand::DescribeState,
        STARTPAGE_APP_OBSERVER_TIMEOUT_MS,
    )
    .ok()
    .filter(|response| response.error.is_none())
    .and_then(|response| response.data);
    // Faithfulness: browser rows prove daemon liveness (don't kill daemon, use telemetry).
    let browser_old = browser_rows
        .iter()
        .filter(|r| {
            r.get("kind")
                .and_then(|k| k.as_str())
                .map(|k| k == "Session")
                .unwrap_or(false)
        })
        .count();
    let browser_new = browser_rows
        .iter()
        .filter(|r| {
            if r.get("kind").and_then(|k| k.as_str()) != Some("Session") {
                return false;
            }
            if r.get("document_kind").and_then(|d| d.as_str()).is_some() {
                return false;
            }
            let host = r.get("host_label").and_then(|s| s.as_str()).unwrap_or("");
            let icon = r.get("icon_kind").and_then(|s| s.as_str()).unwrap_or("");
            if host == "local-shell" {
                return false;
            }
            if icon == "terminal" {
                return false;
            }
            true
        })
        .count();
    // Store ground truth — single-sourced via scan_all_durable_sessions + remote
    let mut store_rows = yggterm_core::startpage::scan_all_durable_sessions(system_home);
    // Also compute remote_total and build remote durable rows from snapshot
    let (remote_rows, remote_total) = build_remote_durable_rows(&snapshot_json);
    // Faithful ordering: is_live > in_scope > modified_epoch > started_at > idx
    // Parse selected scope like GUI does (machine_key + cwd)
    let (scope_machine, scope_cwd, scope_is_live_sessions) = {
        let selected_path = app_state
            .as_ref()
            .and_then(|v| v.get("data"))
            .or(app_state.as_ref())
            .and_then(|d| d.get("selected_browser_path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                app_state
                    .as_ref()
                    .and_then(|v| v.get("data"))
                    .or(app_state.as_ref())
                    .and_then(|d| d.get("selected_row"))
                    .and_then(|r| r.get("full_path"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        if let Some(path) = selected_path {
            if path == "__live_sessions__" {
                (None, None, true)
            } else if let Some(rest) = path.strip_prefix("__remote_folder__/") {
                if let Some((machine, cwd_tail)) = rest.split_once('/') {
                    let cwd = format!("/{}", cwd_tail);
                    (Some(machine.to_string()), Some(cwd), false)
                } else {
                    (Some(rest.to_string()), None, false)
                }
            } else if let Some(machine) = path.strip_prefix("__remote_machine__/") {
                (Some(machine.to_string()), None, false)
            } else if path == "local" || path.starts_with('/') || path.starts_with("local://") {
                // local scope — cwd may be path itself if it's a folder path
                let cwd = if path.starts_with('/') {
                    Some(path)
                } else {
                    None
                };
                (Some("__local__".to_string()), cwd, false)
            } else {
                (None, None, false)
            }
        } else {
            (None, None, false)
        }
    };
    // Merge local + remote, dedup by session_id (same session may appear as both local file and remote scan)
    let mut all_rows_map: std::collections::HashMap<
        String,
        yggterm_core::startpage::StartpageDurableRow,
    > = std::collections::HashMap::new();
    for row in store_rows {
        all_rows_map.insert(row.session_id.clone(), row);
    }
    for row in remote_rows {
        all_rows_map.entry(row.session_id.clone()).or_insert(row);
    }
    // Inject live sessions that have no durable transcript yet (dual presence)
    // Use browser rows' session_cwd as authoritative cwd for live sessions
    {
        use yggterm_core::SessionKind;
        for brow in &browser_rows {
            if brow.get("kind").and_then(|v| v.as_str()) != Some("Session") {
                continue;
            }
            // Skip app/terminal rows
            if brow.get("document_kind").and_then(|v| v.as_str()).is_some() {
                continue;
            }
            let host_label = brow
                .get("host_label")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let icon_kind = brow.get("icon_kind").and_then(|v| v.as_str()).unwrap_or("");
            if host_label == "local-shell" || icon_kind == "terminal" {
                continue;
            }
            let full_path = brow
                .get("full_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Check if this browser row corresponds to a live session
            let is_live_browser = live_set.contains(&full_path)
                || brow
                    .get("live_member")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                || brow
                    .get("live_keep_alive")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
            if !is_live_browser {
                continue;
            }
            let sid = brow
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if sid.is_empty() || all_rows_map.contains_key(&sid) {
                continue;
            }
            let cwd = brow
                .get("session_cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .map(|h| h.display().to_string())
                        .unwrap_or_else(|| "/home/user".to_string())
                });
            let label = brow
                .get("label")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.trim().is_empty());
            // Map icon_kind to SessionKind
            // ONE owner for "which CLI is this row", shared with the cwd tree.
            // The hand-list this replaces spelled three registry slugs wrong and
            // had no arm for the codex family at all, then guessed **Muse** for
            // anything `local://` — the birth scheme every local CLI row uses.
            let kind = yggterm_core::agent_scheme::session_kind_for_row(&full_path, icon_kind)
                .unwrap_or(SessionKind::ClaudeCode);
            let display_path = full_path.clone();
            let storage_path = brow
                .get("session_cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let row = yggterm_core::startpage::StartpageDurableRow {
                session_id: sid.clone(),
                cwd: cwd.clone(),
                title: label.clone(),
                generated_title: None,
                effective_title: label.clone(),
                detail: None,
                kind,
                modified_epoch_ms: now_ms,
                storage_path: if storage_path.is_empty() {
                    display_path.clone()
                } else {
                    storage_path
                },
                display_path,
            };
            all_rows_map.insert(sid, row);
        }
    }
    let mut all_rows: Vec<yggterm_core::startpage::StartpageDurableRow> =
        all_rows_map.into_values().collect();
    // Keep browser counts for warnings (must be computed before draining)
    let browser_old = browser_rows
        .iter()
        .filter(|r| {
            r.get("kind")
                .and_then(|k| k.as_str())
                .map(|k| k == "Session")
                .unwrap_or(false)
        })
        .count();
    let browser_new = browser_rows
        .iter()
        .filter(|r| {
            if r.get("kind").and_then(|k| k.as_str()) != Some("Session") {
                return false;
            }
            if r.get("document_kind").and_then(|d| d.as_str()).is_some() {
                return false;
            }
            let host = r.get("host_label").and_then(|s| s.as_str()).unwrap_or("");
            let icon = r.get("icon_kind").and_then(|s| s.as_str()).unwrap_or("");
            if host == "local-shell" {
                return false;
            }
            if icon == "terminal" {
                return false;
            }
            true
        })
        .count();
    let mut candidates: Vec<(
        yggterm_core::startpage::StartpageDurableRow,
        bool,
        bool,
        i64,
        String,
        usize,
    )> = Vec::new();
    for (idx, row) in all_rows.drain(..).enumerate() {
        let is_live = live_set.contains(&row.display_path)
            || live_set.contains(&row.storage_path)
            || live_set.contains(&row.session_id)
            || live_set.contains(&format!("remote-cc://{}", row.session_id))
            || live_set.contains(&format!("remote-session://{}", row.session_id))
            || live_set.contains(&format!("local://{}", row.session_id));
        let epoch_ms = remote_epoch_by_id
            .get(&row.session_id)
            .map(|e| (*e as u128) * 1000)
            .unwrap_or(row.modified_epoch_ms);
        let epoch = i64::try_from(epoch_ms / 1000).unwrap_or(0);
        let in_scope = if scope_is_live_sessions {
            is_live
        } else if scope_machine.is_some() && scope_cwd.is_some() {
            let scope_cwd_str = scope_cwd.as_deref().unwrap();
            let scope_machine_str = scope_machine.as_deref().unwrap();
            let row_cwd = row.cwd.trim();
            let scope_cwd_trim = scope_cwd_str.trim();
            let cwd_match = row_cwd == scope_cwd_trim
                || row_cwd.starts_with(&format!("{}/", scope_cwd_trim.trim_end_matches('/')));
            let machine_match = if scope_machine_str == "__local__" {
                !row.display_path.starts_with("remote-")
            } else {
                row.display_path.contains(scope_machine_str)
            };
            cwd_match && machine_match
        } else if let Some(scope_cwd_str) = scope_cwd.as_deref() {
            let row_cwd = row.cwd.trim();
            let scope_cwd_trim = scope_cwd_str.trim();
            row_cwd == scope_cwd_trim
                || row_cwd.starts_with(&format!("{}/", scope_cwd_trim.trim_end_matches('/')))
        } else if let Some(scope_machine_str) = scope_machine.as_deref() {
            if scope_machine_str == "__local__" {
                !row.display_path.starts_with("remote-")
            } else {
                row.display_path.contains(scope_machine_str)
            }
        } else {
            true
        };
        // started_at for remote rows is available via remote map, but we use String::new for now (ordering tie-break)
        candidates.push((row, is_live, in_scope, epoch, String::new(), idx));
    }
    let ordered_rows = yggterm_core::startpage::order_candidates_for_startpage(candidates);
    let warnings = vec![
        format!(
            "faithful daemon browser: old Session={} new_agent={} store_durable={} remote_total={}",
            browser_old,
            browser_new,
            ordered_rows.len(),
            remote_total
        ),
        format!(
            "store ground truth {} local + {} remote = {} fleet; ordering is_live>in_scope>recency",
            ordered_rows.len(),
            remote_total,
            ordered_rows.len() + remote_total
        ),
    ];
    append_trace_event(
        yggterm_home,
        "cli",
        "startpage_observers",
        "faithful_read",
        serde_json::json!({
            "browser_row_count": browser_rows.len(),
            "daemon_snapshot_available": snapshot_json.is_some(),
            "app_state_available": app_state.is_some(),
            "elapsed_ms": observer_started.elapsed().as_millis(),
            "app_control_timeout_ms": STARTPAGE_APP_OBSERVER_TIMEOUT_MS,
        }),
    );
    return Some((ordered_rows, warnings, live_paths));
}

fn build_remote_durable_rows(
    snapshot_json: &Option<serde_json::Value>,
) -> (Vec<yggterm_core::startpage::StartpageDurableRow>, usize) {
    let mut rows = Vec::new();
    let mut total = 0;
    let Some(machines) = snapshot_json
        .as_ref()
        .and_then(|v| v.get("data"))
        .or(snapshot_json.as_ref())
        .and_then(|d| d.get("remote_machines"))
        .and_then(|v| v.as_array())
    else {
        return (rows, total);
    };
    for machine in machines {
        let Some(sessions) = machine.get("sessions").and_then(|v| v.as_array()) else {
            continue;
        };
        total += sessions.len();
        for sess in sessions {
            let session_id = sess
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if session_id.is_empty() {
                continue;
            }
            let cwd = sess
                .get("cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title_hint = sess
                .get("title_hint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let modified_epoch = sess
                .get("modified_epoch")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let storage_path = sess
                .get("storage_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let session_path = sess
                .get("session_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Title filtering: same as local — drop generated fallback/low-signal
            let title = if yggterm_core::looks_like_generated_fallback_title(&title_hint)
                || yggterm_core::looks_like_low_signal_generated_copy(&title_hint)
            {
                None
            } else if title_hint.trim().is_empty() {
                None
            } else {
                Some(title_hint.clone())
            };
            let kind_on_wire = sess
                .get("kind")
                .and_then(|value| serde_json::from_value::<yggterm_core::SessionKind>(value.clone()).ok());
            let kind = yggterm_core::agent_scheme::session_kind_for_scanned_row(&session_path, kind_on_wire);
            let display_path = if session_path.is_empty() {
                format!("remote-session://{}", session_id)
            } else {
                session_path.clone()
            };
            let home_dir = dirs::home_dir().unwrap_or_default();
            let generated_title = yggterm_core::SessionTitleStore::open(&home_dir.join(".yggterm"))
                .ok()
                .or_else(|| yggterm_core::SessionTitleStore::open(&home_dir).ok())
                .and_then(|store| store.get_title(&session_id).ok().flatten())
                .filter(|s| {
                    !yggterm_core::looks_like_generated_fallback_title(s)
                        && !yggterm_core::looks_like_low_signal_generated_copy(s)
                });
            let effective_title = title.clone().or(generated_title.clone());
            rows.push(yggterm_core::startpage::StartpageDurableRow {
                session_id: session_id.clone(),
                cwd: if cwd.is_empty() { "/".to_string() } else { cwd },
                title: title.clone(),
                generated_title,
                effective_title,
                detail: None,
                kind,
                modified_epoch_ms: (modified_epoch as u128) * 1000,
                storage_path: if storage_path.is_empty() {
                    display_path.clone()
                } else {
                    storage_path
                },
                display_path,
            });
        }
    }
    (rows, total)
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname")
        .output()
        .context("hostname")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
