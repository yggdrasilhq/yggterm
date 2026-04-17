use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use yggterm_core::{
    AppSettings, SessionStore, SessionTitleStore, best_effort_context_from_session_path,
    best_effort_precis_from_context, best_effort_summary_from_context,
    best_effort_title_from_context, read_codex_session_identity_fields,
};
use yggterm_server::{
    RemoteDeployState, RemoteMachineHealth, RemoteMachineSnapshot, RemoteScannedSession,
    SessionKind, SshConnectTarget, fetch_remote_generation_context, persist_remote_generated_copy,
    scan_remote_machine_sessions_for_target,
};

#[derive(Debug, Default)]
struct Options {
    machines: Vec<String>,
    skip_local: bool,
    skip_remote: bool,
    skip_precis: bool,
    dry_run: bool,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlEnvelope<T> {
    data: T,
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlStateData {
    remote: AppControlRemoteState,
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlRemoteState {
    machines: Vec<AppControlRemoteMachine>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlRemoteMachine {
    machine_key: String,
    label: String,
    ssh_target: String,
}

#[derive(Debug, Serialize)]
struct LocalRefreshSummary {
    scanned: usize,
    refreshed: usize,
    titles: usize,
    precis: usize,
    summaries: usize,
}

#[derive(Debug, Serialize)]
struct RemoteMachineRefreshSummary {
    machine_key: String,
    ssh_target: String,
    scanned: usize,
    refreshed: usize,
    titles: usize,
    precis: usize,
    summaries: usize,
}

#[derive(Debug, Serialize)]
struct RefreshSummary {
    model: String,
    dry_run: bool,
    elapsed_ms: u128,
    local: Option<LocalRefreshSummary>,
    remote: Vec<RemoteMachineRefreshSummary>,
}

fn parse_options(args: &[String]) -> Result<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--machine" => {
                let value = args
                    .get(index + 1)
                    .context("missing value after --machine")?;
                options.machines.push(value.clone());
                index += 2;
            }
            "--skip-local" => {
                options.skip_local = true;
                index += 1;
            }
            "--skip-remote" => {
                options.skip_remote = true;
                index += 1;
            }
            "--skip-precis" => {
                options.skip_precis = true;
                index += 1;
            }
            "--dry-run" => {
                options.dry_run = true;
                index += 1;
            }
            "--limit" => {
                let value = args.get(index + 1).context("missing value after --limit")?;
                options.limit = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| format!("invalid --limit value {value}"))?,
                );
                index += 2;
            }
            "--help" | "-h" | "help" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    Ok(options)
}

fn print_help() {
    println!(
        "usage:
  yggterm-refresh-copy [--machine <ssh-target>]... [--skip-local] [--skip-remote] [--skip-precis] [--dry-run] [--limit <count>]

defaults:
  - refresh local codex session title/precis/summary copy
  - auto-discover remote machines from the running Yggterm app when available
  - force-refresh existing generated copy"
    );
}

fn collect_local_codex_session_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("reading local codex session dir {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type for {}", path.display()))?;
        if file_type.is_dir() {
            collect_local_codex_session_files(&path, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
    Ok(())
}

fn local_codex_root() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("sessions")
}

fn sibling_yggterm_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating current executable")?;
    Ok(exe.with_file_name("yggterm"))
}

fn discover_remote_machines_from_app_state() -> Result<Vec<RemoteMachineSnapshot>> {
    let binary = sibling_yggterm_binary()?;
    if !binary.exists() {
        return Ok(Vec::new());
    }
    let output = Command::new(binary)
        .args(["server", "app", "state", "--timeout-ms", "5000"])
        .output()
        .context("running `yggterm server app state` for remote machine discovery")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let envelope: AppControlEnvelope<AppControlStateData> = serde_json::from_slice(&output.stdout)
        .context("parsing app-control state response for remote machine discovery")?;
    Ok(envelope
        .data
        .remote
        .machines
        .into_iter()
        .map(|machine| RemoteMachineSnapshot {
            machine_key: machine.machine_key,
            label: machine.label,
            ssh_target: machine.ssh_target,
            prefix: None,
            remote_binary_expr: None,
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: Vec::new(),
        })
        .collect())
}

fn explicit_remote_machines(machine_targets: &[String]) -> Vec<RemoteMachineSnapshot> {
    machine_targets
        .iter()
        .map(|target| RemoteMachineSnapshot {
            machine_key: target.clone(),
            label: target.clone(),
            ssh_target: target.clone(),
            prefix: None,
            remote_binary_expr: None,
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: Vec::new(),
        })
        .collect()
}

fn apply_limit<T>(items: &mut Vec<T>, limit: Option<usize>) {
    if let Some(limit) = limit
        && items.len() > limit
    {
        items.truncate(limit);
    }
}

fn refresh_local_copy(
    store: &SessionStore,
    settings: &AppSettings,
    options: &Options,
) -> Result<LocalRefreshSummary> {
    let title_store = SessionTitleStore::open(store.home_dir())?;
    let mut files = Vec::new();
    collect_local_codex_session_files(&local_codex_root(), &mut files)?;
    files.sort();
    files.dedup();
    apply_limit(&mut files, options.limit);
    let mut summary = LocalRefreshSummary {
        scanned: files.len(),
        refreshed: 0,
        titles: 0,
        precis: 0,
        summaries: 0,
    };
    for path in files {
        let session_path = path.display().to_string();
        let mut fallback_context = None::<String>;
        let identity = read_codex_session_identity_fields(&path)
            .with_context(|| format!("reading local session identity for {}", path.display()))?;
        let title = match store.generate_title_for_session_path(settings, &session_path, true) {
            Ok(Some(value)) => Some(value),
            Ok(None) => {
                let fallback = best_effort_title_from_context(ensure_fallback_context(
                    &mut fallback_context,
                    &path,
                ));
                if let (Some((session_id, cwd)), Some(title)) =
                    (identity.as_ref(), fallback.as_ref())
                {
                    title_store.put_title(session_id, cwd, title, "heuristic", "heuristic")?;
                }
                fallback
            }
            Err(error) => {
                eprintln!("warning: local title generation failed for {session_path}: {error:#}");
                let fallback = best_effort_title_from_context(ensure_fallback_context(
                    &mut fallback_context,
                    &path,
                ));
                if let (Some((session_id, cwd)), Some(title)) =
                    (identity.as_ref(), fallback.as_ref())
                {
                    title_store.put_title(session_id, cwd, title, "heuristic", "heuristic")?;
                }
                fallback
            }
        };
        let precis = if options.skip_precis {
            None
        } else {
            match store.generate_precis_for_session_path(settings, &session_path, true) {
                Ok(Some(value)) => Some(value),
                Ok(None) => {
                    let fallback = best_effort_precis_from_context(ensure_fallback_context(
                        &mut fallback_context,
                        &path,
                    ));
                    if let (Some((session_id, cwd)), Some(precis)) =
                        (identity.as_ref(), fallback.as_ref())
                    {
                        title_store.put_precis(
                            session_id,
                            cwd,
                            precis,
                            "heuristic",
                            "heuristic",
                        )?;
                    }
                    fallback
                }
                Err(error) => {
                    eprintln!(
                        "warning: local precis generation failed for {session_path}: {error:#}"
                    );
                    let fallback = best_effort_precis_from_context(ensure_fallback_context(
                        &mut fallback_context,
                        &path,
                    ));
                    if let (Some((session_id, cwd)), Some(precis)) =
                        (identity.as_ref(), fallback.as_ref())
                    {
                        title_store.put_precis(
                            session_id,
                            cwd,
                            precis,
                            "heuristic",
                            "heuristic",
                        )?;
                    }
                    fallback
                }
            }
        };
        let body_summary =
            match store.generate_summary_for_session_path(settings, &session_path, true) {
                Ok(Some(value)) => Some(value),
                Ok(None) => {
                    let fallback = best_effort_summary_from_context(ensure_fallback_context(
                        &mut fallback_context,
                        &path,
                    ));
                    if let (Some((session_id, cwd)), Some(summary)) =
                        (identity.as_ref(), fallback.as_ref())
                    {
                        title_store.put_summary(
                            session_id,
                            cwd,
                            summary,
                            "heuristic",
                            "heuristic",
                        )?;
                    }
                    fallback
                }
                Err(error) => {
                    eprintln!(
                        "warning: local summary generation failed for {session_path}: {error:#}"
                    );
                    let fallback = best_effort_summary_from_context(ensure_fallback_context(
                        &mut fallback_context,
                        &path,
                    ));
                    if let (Some((session_id, cwd)), Some(summary)) =
                        (identity.as_ref(), fallback.as_ref())
                    {
                        title_store.put_summary(
                            session_id,
                            cwd,
                            summary,
                            "heuristic",
                            "heuristic",
                        )?;
                    }
                    fallback
                }
            };
        summary.refreshed += 1;
        if title.is_some() {
            summary.titles += 1;
        }
        if precis.is_some() {
            summary.precis += 1;
        }
        if body_summary.is_some() {
            summary.summaries += 1;
        }
        println!("local refreshed {}", session_path);
    }
    Ok(summary)
}

fn scan_remote_machine(machine: &RemoteMachineSnapshot) -> Result<Vec<RemoteScannedSession>> {
    let target = SshConnectTarget {
        label: machine.label.clone(),
        kind: SessionKind::SshShell,
        ssh_target: machine.ssh_target.clone(),
        prefix: machine.prefix.clone(),
        cwd: None,
    };
    scan_remote_machine_sessions_for_target(&target)
        .with_context(|| format!("scanning remote machine {}", machine.ssh_target))
}

fn merge_context_fragments(primary: &str, secondary: &str) -> String {
    let primary = primary.trim();
    let secondary = secondary.trim();
    match (primary.is_empty(), secondary.is_empty()) {
        (true, true) => String::new(),
        (false, true) => primary.to_string(),
        (true, false) => secondary.to_string(),
        (false, false) => {
            let primary_lower = primary.to_ascii_lowercase();
            let secondary_lower = secondary.to_ascii_lowercase();
            if primary_lower.contains(&secondary_lower) {
                primary.to_string()
            } else if secondary_lower.contains(&primary_lower) {
                secondary.to_string()
            } else {
                format!("{primary}\n{secondary}")
            }
        }
    }
}

fn ensure_fallback_context<'a>(slot: &'a mut Option<String>, path: &Path) -> &'a str {
    slot.get_or_insert_with(|| best_effort_context_from_session_path(path).unwrap_or_default())
        .as_str()
}

fn refresh_remote_copy(
    store: &SessionStore,
    settings: &AppSettings,
    options: &Options,
    mut machine: RemoteMachineSnapshot,
) -> Result<RemoteMachineRefreshSummary> {
    let target = SshConnectTarget {
        label: machine.label.clone(),
        kind: SessionKind::SshShell,
        ssh_target: machine.ssh_target.clone(),
        prefix: machine.prefix.clone(),
        cwd: None,
    };
    let mut sessions = scan_remote_machine(&machine)?;
    sessions.sort_by(|left, right| left.storage_path.cmp(&right.storage_path));
    apply_limit(&mut sessions, options.limit);
    machine.sessions = sessions.clone();
    let mut summary = RemoteMachineRefreshSummary {
        machine_key: machine.machine_key.clone(),
        ssh_target: machine.ssh_target.clone(),
        scanned: sessions.len(),
        refreshed: 0,
        titles: 0,
        precis: 0,
        summaries: 0,
    };
    let mut remote_llm_enabled = true;
    for scanned in sessions {
        let fetched_context = fetch_remote_generation_context(&target, &scanned.storage_path)
            .with_context(|| {
                format!(
                    "fetching remote generation context for {} on {}",
                    scanned.storage_path, machine.ssh_target
                )
            })?;
        let context = merge_context_fragments(&fetched_context, &scanned.recent_context);
        let mut disable_remote_llm = false;
        let title = if remote_llm_enabled {
            match store.generate_title_for_context(
                settings,
                &scanned.session_id,
                &scanned.cwd,
                &context,
                true,
            ) {
                Ok(Some(value)) => Some(value),
                Ok(None) => best_effort_title_from_context(&context),
                Err(error) => {
                    disable_remote_llm = true;
                    eprintln!(
                        "warning: remote title generation failed for {} on {}: {error:#}; disabling remote llm generation for remaining sessions on this machine",
                        scanned.storage_path, machine.ssh_target
                    );
                    best_effort_title_from_context(&context)
                }
            }
        } else {
            best_effort_title_from_context(&context)
        };
        let precis = if options.skip_precis {
            None
        } else if remote_llm_enabled && !disable_remote_llm {
            match store.generate_precis_for_context(
                settings,
                &scanned.session_id,
                &scanned.cwd,
                &context,
                true,
            ) {
                Ok(Some(value)) => Some(value),
                Ok(None) => best_effort_precis_from_context(&context),
                Err(error) => {
                    disable_remote_llm = true;
                    eprintln!(
                        "warning: remote precis generation failed for {} on {}: {error:#}; disabling remote llm generation for remaining sessions on this machine",
                        scanned.storage_path, machine.ssh_target
                    );
                    best_effort_precis_from_context(&context)
                }
            }
        } else {
            best_effort_precis_from_context(&context)
        };
        let body_summary = if remote_llm_enabled && !disable_remote_llm {
            match store.generate_summary_for_context(
                settings,
                &scanned.session_id,
                &scanned.cwd,
                &context,
                true,
            ) {
                Ok(Some(value)) => Some(value),
                Ok(None) => best_effort_summary_from_context(&context),
                Err(error) => {
                    disable_remote_llm = true;
                    eprintln!(
                        "warning: remote summary generation failed for {} on {}: {error:#}; disabling remote llm generation for remaining sessions on this machine",
                        scanned.storage_path, machine.ssh_target
                    );
                    best_effort_summary_from_context(&context)
                }
            }
        } else {
            best_effort_summary_from_context(&context)
        };
        if disable_remote_llm {
            remote_llm_enabled = false;
        }
        if !options.dry_run {
            persist_remote_generated_copy(
                &machine,
                &scanned.session_id,
                &scanned.cwd,
                title.as_deref(),
                precis.as_deref(),
                body_summary.as_deref(),
                &settings.interface_llm_model,
            )?;
        }
        summary.refreshed += 1;
        if title.is_some() {
            summary.titles += 1;
        }
        if precis.is_some() {
            summary.precis += 1;
        }
        if body_summary.is_some() {
            summary.summaries += 1;
        }
        println!(
            "remote refreshed {} {}",
            machine.ssh_target, scanned.storage_path
        );
    }
    Ok(summary)
}

fn dedupe_machines(machines: Vec<RemoteMachineSnapshot>) -> Vec<RemoteMachineSnapshot> {
    let mut seen = std::collections::BTreeSet::<(String, String)>::new();
    let mut deduped = Vec::new();
    for machine in machines {
        let key = (machine.machine_key.clone(), machine.ssh_target.clone());
        if seen.insert(key) {
            deduped.push(machine);
        }
    }
    deduped
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let options = parse_options(&args)?;
    let started_at = Instant::now();
    let store = SessionStore::open_or_init()?;
    let settings = store.load_settings().unwrap_or_default();

    let local = if options.skip_local {
        None
    } else {
        Some(refresh_local_copy(&store, &settings, &options)?)
    };

    let mut remote_summaries = Vec::new();
    if !options.skip_remote {
        let discovered = if options.machines.is_empty() {
            discover_remote_machines_from_app_state().unwrap_or_default()
        } else {
            explicit_remote_machines(&options.machines)
        };
        for machine in dedupe_machines(discovered) {
            remote_summaries.push(refresh_remote_copy(&store, &settings, &options, machine)?);
        }
    }

    let summary = RefreshSummary {
        model: settings.interface_llm_model.clone(),
        dry_run: options.dry_run,
        elapsed_ms: started_at.elapsed().as_millis(),
        local,
        remote: remote_summaries,
    };
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
