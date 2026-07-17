use crate::codex_cli::{
    TerminalIdentityColorProfile, sync_terminal_identity_appearance_with_profile,
};
use crate::terminal::{TerminalBufferStats, terminal_data_has_scrollback_text};
use crate::{
    CodexRuntimeProcessIdentity, GhosttyHostSupport, ManagedSessionView, PersistedDaemonState,
    PersistedLiveSession, RemoteMachineSnapshot, RemoteRuntimeRegistry, ServerUiSnapshot,
    SessionKind, SessionSource, SnapshotSessionView, SshConnectTarget, TerminalManager,
    WorkspaceViewMode, YggtermServer, active_client_instance_records,
    active_client_instance_records_for_endpoint_scope,
    claude_code_runtime_process_identity_from_root_pid,
    codex_runtime_process_identity_from_root_pid, current_millis, fetch_remote_generation_context,
    local_headless_companion_executable_from_current, overlay_codex_runtime_snapshot_identity,
    persist_remote_generated_copy, poll_remote_local_codex_identities,
    remote_resume_runtime_output_requires_restart, request_remote_codex_session_shutdown,
    spawn_hot_restart_daemon_process, terminate_remote_codex_session,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::MutexGuard;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use time::OffsetDateTime;
use tracing::{info, warn};
use yggterm_core::{
    AppSettings, PerfSpan, SessionNode, SessionNodeKind, SessionStore, append_trace_event,
    local_cc_session_jsonl_path, looks_like_generated_fallback_title, read_cc_session_title,
    resolve_yggterm_home,
};
use yggui_contract::UiTheme;

pub const SERVER_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");
const BACKGROUND_COPY_CHORE_MS: u64 = 12_000;
const BACKGROUND_COPY_MAX_IDLE_CHORE_MS: u64 = 60_000;
const BACKGROUND_COPY_BUDGET_PER_TICK: usize = 23;
// Endpoint pacing ([[spec-title-summary-working-indicator]]): llm.example.com
// 429s under quick successive calls; each generation may be 2 LLM calls.
const BACKGROUND_COPY_LLM_GENERATIONS_PER_TICK: usize = 3;
// A session that produced PTY output within this window counts as "working"
// for the title trigger even if the esc-to-interrupt footer just cleared.
const BACKGROUND_COPY_WORKING_RECENT_MS: u64 = 30_000;
const DAEMON_ACCEPT_POLL_MS: u64 = 1000;
const DEFAULT_DAEMON_IDLE_SHUTDOWN_MS: u64 = 90_000;
const DEFAULT_TERMINAL_IDLE_TRIM_AFTER_MS: u64 = 45_000;
#[cfg(target_os = "linux")]
const DEFAULT_ORPHAN_DAEMON_REAP_AFTER_MS: u64 = 180_000;
#[cfg(target_os = "linux")]
const DUPLICATE_SAME_HOME_GRACE_MS: u64 = 2_000;
const DAEMON_REQUEST_IO_TIMEOUT_MS: u64 = 10_000;
const DAEMON_LONG_REQUEST_IO_TIMEOUT_MS: u64 = 60_000;
const DAEMON_CLIENT_REQUEST_READ_TIMEOUT_MS: u64 = 2_000;
const REMOTE_ATTACH_STARTUP_GRACE_MS: u64 = 900;
// Negative-cache window for dead preserved-owner sockets (post-swap shadow fix).
const PRESERVED_OWNER_UNREACHABLE_CACHE_MS: u64 = 60_000;
const REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS: u64 = 18_000;
const CLIENT_CLOSE_FORCE_SHUTDOWN_AFTER_SECS: u64 = 60 * 60;
const EXPLICIT_REMOTE_SESSION_CLOSE_FORCE_AFTER_SECS: u64 = 2;
const ENV_YGGTERM_ENABLE_BACKGROUND_COPY_CHORE: &str = "YGGTERM_ENABLE_BACKGROUND_COPY_CHORE";
// Remote-Codex identity poll (`[[finding-uuidv4-codex-session-drift]]` Stage 2).
const REMOTE_CODEX_IDENTITY_POLL_MS: u64 = 8_000;
const REMOTE_CODEX_IDENTITY_POLL_MAX_IDLE_MS: u64 = 60_000;
// A remote-Codex row that never matches a running process (codex already
// exited, cwd mismatch) is abandoned after this many SSH polls so the daemon
// does not SSH a machine forever for an un-rebindable row.
const REMOTE_CODEX_IDENTITY_POLL_MAX_ATTEMPTS: u32 = 12;
const ENV_YGGTERM_DISABLE_REMOTE_CODEX_IDENTITY_POLL: &str =
    "YGGTERM_DISABLE_REMOTE_CODEX_IDENTITY_POLL";
// Perf incident monitor: every MONITOR_MS, look at the last WINDOW_MS of perf
// telemetry and, if it looks like a load incident (the random "fan gets angry"),
// append a durable snapshot to perf-incidents.jsonl. See record_perf_incident_if_hot.
const PERF_INCIDENT_MONITOR_MS: u64 = 30_000;
const PERF_INCIDENT_WINDOW_MS: u64 = 60_000;

fn spawn_explicit_remote_session_shutdown(
    home: &Path,
    path: &str,
    machine: RemoteMachineSnapshot,
    session_id: String,
) {
    let home = home.to_path_buf();
    let path = path.to_string();
    let machine_key = machine.machine_key.clone();
    let spawn_failure_home = home.clone();
    let spawn_failure_path = path.clone();
    let spawn_failure_machine_key = machine_key.clone();
    let spawn_failure_session_id = session_id.clone();
    let thread_name = format!("explicit-remote-close-{}", machine.machine_key);
    let spawn_result = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let force_after =
                std::time::Duration::from_secs(EXPLICIT_REMOTE_SESSION_CLOSE_FORCE_AFTER_SECS);
            let result = request_remote_codex_session_shutdown(&machine, &session_id, force_after);
            append_trace_event(
                &home,
                "daemon",
                "session",
                if result.is_ok() {
                    "explicit_remote_session_close_requested"
                } else {
                    "explicit_remote_session_close_warning"
                },
                serde_json::json!({
                    "path": path,
                    "machine_key": machine_key,
                    "ssh_target": machine.ssh_target,
                    "session_id": session_id,
                    "force_after_seconds": EXPLICIT_REMOTE_SESSION_CLOSE_FORCE_AFTER_SECS,
                    "error": result.err().map(|error| error.to_string()),
                }),
            );
        });
    if let Err(error) = spawn_result {
        append_trace_event(
            &spawn_failure_home,
            "daemon",
            "session",
            "explicit_remote_session_close_spawn_failed",
            serde_json::json!({
                "path": spawn_failure_path,
                "machine_key": spawn_failure_machine_key,
                "session_id": spawn_failure_session_id,
                "error": error.to_string(),
            }),
        );
    }
}

fn spawn_remote_generated_copy_persist(
    machine: RemoteMachineSnapshot,
    session_id: String,
    cwd: String,
    title: Option<String>,
    precis: Option<String>,
    summary: Option<String>,
    model: &'static str,
) {
    let thread_name = format!("remote-copy-persist-{}", machine.machine_key);
    if let Err(error) = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            if let Err(error) = persist_remote_generated_copy(
                &machine,
                &session_id,
                &cwd,
                title.as_deref(),
                precis.as_deref(),
                summary.as_deref(),
                model,
            ) {
                warn!(
                    machine_key=%machine.machine_key,
                    session_id=%session_id,
                    error=%error,
                    "failed to persist remote session copy hints"
                );
                if let Ok(home) = resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "copy",
                        "remote_generated_copy_persist_failed",
                        serde_json::json!({
                            "machine_key": machine.machine_key,
                            "session_id": session_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        })
    {
        warn!(error=%error, "failed to spawn remote generated copy persistence worker");
    }
}

fn daemon_env_flag_truthy(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn daemon_env_flag_falsey_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn startup_prewarm_enabled() -> bool {
    if daemon_env_flag_truthy("YGGTERM_DISABLE_STARTUP_TERMINAL_PREWARM") {
        return false;
    }
    std::env::var("YGGTERM_ENABLE_STARTUP_TERMINAL_PREWARM")
        .ok()
        .map(|value| !daemon_env_flag_falsey_value(&value))
        .unwrap_or(false)
}

fn startup_prewarm_skip_reason_with_flag(active_path: &str, enabled: bool) -> Option<&'static str> {
    if !enabled {
        return Some("disabled_by_env");
    }
    if active_path.starts_with("live::") {
        return Some("runtime_owned_live_session");
    }
    None
}

fn startup_prewarm_skip_reason(active_path: &str) -> Option<&'static str> {
    startup_prewarm_skip_reason_with_flag(active_path, startup_prewarm_enabled())
}

fn daemon_request_trace_enabled(request_name: &str) -> bool {
    request_name != "terminal_read" || daemon_env_flag_truthy("YGGTERM_TRACE_TERMINAL_READS")
}

#[cfg(target_os = "linux")]
fn daemon_expected_binary_paths(current_exe: &Path) -> Vec<PathBuf> {
    let mut allowed = vec![current_exe.to_path_buf()];
    if let Some(headless) = local_headless_companion_executable_from_current(current_exe) {
        allowed.push(headless);
    }
    allowed
}

#[cfg(target_os = "linux")]
fn daemon_binary_is_legacy(
    current_exe: &Path,
    argv0: &str,
    proc_exe_target: Option<&Path>,
) -> bool {
    let allowed = daemon_expected_binary_paths(current_exe);
    let argv_allowed = allowed
        .iter()
        .any(|candidate| argv0 == candidate.to_string_lossy());
    if !argv_allowed {
        return true;
    }
    let Some(proc_exe_target) = proc_exe_target else {
        return false;
    };
    let proc_exe_target = daemon_proc_exe_target_without_deleted_suffix(proc_exe_target);
    let target = proc_exe_target.to_string_lossy();
    !allowed
        .iter()
        .any(|candidate| target == candidate.to_string_lossy())
}

#[cfg(target_os = "linux")]
fn daemon_proc_exe_target_without_deleted_suffix(proc_exe_target: &Path) -> PathBuf {
    let target = proc_exe_target.to_string_lossy();
    target
        .strip_suffix(" (deleted)")
        .map(PathBuf::from)
        .unwrap_or_else(|| proc_exe_target.to_path_buf())
}

#[cfg(target_os = "linux")]
fn legacy_daemon_reap_applies_to_home(
    current_home: Option<&Path>,
    daemon_home: Option<&Path>,
) -> bool {
    current_home.is_some() && daemon_home == current_home
}

#[cfg(target_os = "linux")]
fn orphan_daemon_reap_applies_to_home(
    current_home: Option<&Path>,
    daemon_home: Option<&Path>,
) -> bool {
    current_home.is_some() && daemon_home == current_home
}

fn terminal_sidebar_snapshot_from_screen(text: &str) -> Option<(String, Vec<String>)> {
    let lines = text
        .replace('\r', "")
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let tail = lines
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let status_line = tail.last().cloned().unwrap_or_default();
    Some((status_line, tail))
}

/// Parse an "X.Y.Z" (semver-ish) version string into a comparable tuple,
/// tolerating a pre-release/build suffix. Returns None if unparseable.
fn parse_semverish_version(value: &str) -> Option<(u64, u64, u64)> {
    let core = value.split(['-', '+']).next().unwrap_or(value).trim();
    let mut parts = core.split('.');
    let major = parts.next()?.trim().parse::<u64>().ok()?;
    let minor = parts.next()?.trim().parse::<u64>().ok()?;
    let patch = parts
        .next()
        .map(|raw| raw.trim().parse::<u64>().unwrap_or(0))
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// True iff `candidate` is a strictly newer server version than `mine`. Used to
/// retire a stale OLD daemon once an update brought up a newer successor — even
/// when this daemon's versioned binary still exists on disk (so the
/// "/proc/self/exe (deleted)" trigger never fires). That stale-daemon split-brain
/// stranded remote-session terminal streams on the seed placeholder = blank
/// viewport on every update. Unparseable versions return false (never retire —
/// safe default). See [[finding-blank-on-restart-split-brain-daemon]].
fn server_version_is_strictly_newer(candidate: &str, mine: &str) -> bool {
    match (
        parse_semverish_version(candidate),
        parse_semverish_version(mine),
    ) {
        (Some(other), Some(own)) => other > own,
        _ => false,
    }
}

#[cfg(unix)]
fn parse_versioned_server_socket_name(path: &Path) -> Option<(u64, u64, u64)> {
    let file_name = path.file_name()?.to_str()?;
    let version = file_name.strip_prefix("server-")?.strip_suffix(".sock")?;
    let mut parts = version.split('-');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(unix)]
fn versioned_server_socket_alias_candidates(current: &Path) -> Vec<PathBuf> {
    let Some(parent) = current.parent() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let candidate = entry.path();
            if candidate != current
                && parse_versioned_server_socket_name(&candidate).is_some()
                && seen.insert(candidate.clone())
            {
                candidates.push(candidate);
            }
        }
    }
    let client_instances_dir = parent.join("client-instances");
    if let Ok(entries) = fs::read_dir(client_instances_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(socket_name) = name.rsplit("--").next() else {
                continue;
            };
            let Some(socket_name) = socket_name
                .find("server-")
                .map(|offset| &socket_name[offset..])
            else {
                continue;
            };
            let Some(socket_stem) = socket_name.strip_suffix("-sock") else {
                continue;
            };
            let candidate = parent.join(format!("{socket_stem}.sock"));
            if candidate != current
                && parse_versioned_server_socket_name(&candidate).is_some()
                && seen.insert(candidate.clone())
            {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort();
    candidates
}

#[cfg(unix)]
fn refresh_legacy_server_socket_aliases(current: &Path) {
    let Some(current_version) = parse_versioned_server_socket_name(current) else {
        return;
    };
    for candidate in versioned_server_socket_alias_candidates(current) {
        let Some(candidate_version) = parse_versioned_server_socket_name(&candidate) else {
            continue;
        };
        if !versioned_socket_alias_is_legacy(current_version, candidate_version) {
            continue;
        }
        if versioned_socket_alias_points_to_current(&candidate, current) {
            continue;
        }
        if versioned_socket_candidate_is_symlink(&candidate) {
            let _ = fs::remove_file(&candidate);
            let _ = std::os::unix::fs::symlink(current, &candidate);
            continue;
        }
        if ping(&ServerEndpoint::UnixSocket(candidate.clone())).is_ok() {
            continue;
        }
        let _ = fs::remove_file(&candidate);
        let _ = std::os::unix::fs::symlink(current, &candidate);
    }
}

#[cfg(unix)]
fn versioned_socket_alias_points_to_current(candidate: &Path, current: &Path) -> bool {
    let Ok(candidate_target) = candidate.canonicalize() else {
        return false;
    };
    let Ok(current_target) = current.canonicalize() else {
        return false;
    };
    candidate_target == current_target
}

#[cfg(unix)]
fn versioned_socket_candidate_is_symlink(candidate: &Path) -> bool {
    fs::symlink_metadata(candidate)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(unix)]
fn versioned_socket_alias_is_legacy(
    current_version: (u64, u64, u64),
    candidate_version: (u64, u64, u64),
) -> bool {
    candidate_version < current_version
}

fn parse_protocol_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.trim().split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn hot_update_target_regression(
    existing_expected: Option<&str>,
    requested_expected: Option<&str>,
) -> Option<(String, String)> {
    if std::env::var("YGGTERM_ALLOW_HOT_UPDATE_TARGET_DOWNGRADE")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
    {
        return None;
    }
    let existing = existing_expected?;
    let requested = requested_expected?;
    let existing_version = parse_protocol_version(existing)?;
    let requested_version = parse_protocol_version(requested)?;
    (requested_version < existing_version).then(|| (existing.to_string(), requested.to_string()))
}

/// Default idle window an agent CLI session must be quiet for before a
/// hot-update is allowed to proceed. Tuned around the Anthropic prompt-cache
/// window: a `claude --resume` within a few minutes of the last turn still
/// re-hits the cache, so deferring updates while a session was recently active
/// (or is mid-turn) preserves that cache and avoids interrupting work. The user
/// can shorten/lengthen via `YGGTERM_HOT_UPDATE_IDLE_THRESHOLD_MS`.
/// See [[finding-hot-update-interrupts-remote-sessions]].
const HOT_UPDATE_IDLE_THRESHOLD_MS_DEFAULT: u64 = 300_000;

fn hot_update_idle_threshold_ms() -> u64 {
    std::env::var("YGGTERM_HOT_UPDATE_IDLE_THRESHOLD_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(HOT_UPDATE_IDLE_THRESHOLD_MS_DEFAULT)
}

/// `true` when the operator has explicitly opted out of the idle gate (e.g. an
/// agent/dev deploy that must land now). Cache preservation is the default
/// priority, so this is an explicit override rather than something `--force`
/// implies (`--force` only bypasses the same-version target check).
fn hot_update_idle_gate_overridden() -> bool {
    std::env::var("YGGTERM_HOT_UPDATE_IGNORE_IDLE_GATE")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

/// How long a session must have been quiet (no input AND no output — both bump
/// `last_activity_ms`) before progressive migration may release it. Generous on
/// purpose: a working codex/CC turn drives an animated spinner ~10fps the whole
/// time, so output-idle stays ~0 while busy and only climbs at a true prompt;
/// 45s clears the spinner's frame cadence with headroom while still letting an
/// idle session migrate promptly. Tunable via `YGGTERM_MIGRATION_IDLE_MS`.
const MIGRATION_IDLE_THRESHOLD_MS_DEFAULT: u64 = 45_000;

fn migration_idle_threshold_ms() -> u64 {
    std::env::var("YGGTERM_MIGRATION_IDLE_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(MIGRATION_IDLE_THRESHOLD_MS_DEFAULT)
}

/// The tangible, daemon-side OS signals a progressive session migration weighs.
/// Every field is an OS fact the PTY owner already has (it holds the master fd
/// and forwards every byte), uniform across codex/CC/shell — NOT a screen-string
/// scrape. See [[finding-daemon-authoritative-working-state-2945]] for why the
/// `esc to interrupt` footer was demoted from a primary signal to the optional
/// `screen_shows_working` guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MigratableSignals {
    /// Milliseconds since the session last had ANY input or output activity.
    /// `None` = this daemon doesn't own a live runtime for the key.
    activity_idle_ms: Option<u64>,
    /// Sticky "typed-but-unsent draft on the current line". `Some(true)` =
    /// protected draft present; `None` = not owned.
    has_pending_draft: Option<bool>,
    /// `Some(true)` when a foreground command is running in the tty (the PTY's
    /// foreground pgrp differs from the session leader) — i.e. a shell command
    /// or an agent tool-execution child, including SILENT ones that emit no
    /// output (`sleep 300`). `None` = not owned / not a unix runtime.
    foreground_command_running: Option<bool>,
    /// Optional belt-and-suspenders guard: the live screen footer still shows a
    /// recognized agent working indicator. Only ever ADDS safety (an agent
    /// genuinely mid-turn but momentarily silent); shells never depend on it.
    screen_shows_working: bool,
}

/// Pure predicate: is this session SAFE to migrate (release + re-resume on the
/// newest daemon) right now? Biased entirely to safety — a lingering old daemon
/// is harmless, but losing a user's unsent work is the cardinal sin — so ANY
/// unavailable/ambiguous signal (`None`) means NOT migratable.
fn session_is_migratable(signals: &MigratableSignals, idle_threshold_ms: u64) -> bool {
    // (a) output/input-idle past the generous threshold.
    let Some(idle_ms) = signals.activity_idle_ms else {
        return false;
    };
    if idle_ms < idle_threshold_ms {
        return false;
    }
    // (b) no protected draft on the current line.
    if signals.has_pending_draft != Some(false) {
        return false;
    }
    // (c) no foreground command running in the tty.
    if signals.foreground_command_running != Some(false) {
        return false;
    }
    // (d) optional working-footer guard.
    if signals.screen_shows_working {
        return false;
    }
    true
}

#[cfg(unix)]
fn server_socket_path_lexists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(unix)]
fn drain_unix_client_outcomes(
    receiver: &std::sync::mpsc::Receiver<Result<DaemonRequestOutcome>>,
    restart_after_exit: &mut Option<PathBuf>,
) -> bool {
    let mut should_shutdown = false;
    while let Ok(result) = receiver.try_recv() {
        match result {
            Ok(outcome) => {
                if let Some(executable) = outcome.restart_executable {
                    *restart_after_exit = Some(executable);
                }
                if outcome.should_shutdown {
                    should_shutdown = true;
                }
            }
            Err(error) => warn!(error=%format!("{error:#}"), "daemon request failed"),
        }
    }
    should_shutdown
}

#[cfg(unix)]
fn spawn_unix_client_handler(
    stream: std::os::unix::net::UnixStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
    outcomes: std::sync::mpsc::Sender<Result<DaemonRequestOutcome>>,
) {
    let thread_name = format!("yggterm-daemon-client-{}", current_millis());
    if let Err(error) = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let result = handle_unix_stream(stream, runtime, last_activity_ms);
            let _ = outcomes.send(result);
        })
    {
        warn!(error=%error, "failed to spawn daemon client handler");
    }
}

#[cfg(unix)]
const MAX_UNIX_SOCKET_PATH_BYTES: usize = 100;

#[cfg(unix)]
fn unix_socket_path_fits_platform(path: &Path) -> bool {
    path.as_os_str().as_bytes().len() < MAX_UNIX_SOCKET_PATH_BYTES
}

#[cfg(unix)]
fn stable_socket_home_hash(home_dir: &Path) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in home_dir.as_os_str().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(unix)]
fn runtime_socket_base_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::geteuid() };
            std::env::temp_dir().join(format!("yggterm-{uid}"))
        })
        .join("yggterm")
}

#[cfg(unix)]
fn short_unix_socket_path_for_home(home_dir: &Path, socket_name: &str) -> PathBuf {
    let hash = stable_socket_home_hash(home_dir);
    let scoped = PathBuf::from(format!("h-{hash:016x}")).join(socket_name);
    let runtime_candidate = runtime_socket_base_dir().join(&scoped);
    if unix_socket_path_fits_platform(&runtime_candidate) {
        return runtime_candidate;
    }
    let temp_candidate = std::env::temp_dir()
        .join(format!("yggterm-{}", unsafe { libc::geteuid() }))
        .join(scoped);
    if unix_socket_path_fits_platform(&temp_candidate) {
        return temp_candidate;
    }
    std::env::temp_dir().join(format!(
        "yg-{hash:016x}-{}.sock",
        SERVER_PROTOCOL_VERSION.replace('.', "-")
    ))
}

#[cfg(unix)]
fn unix_socket_path_uses_runtime_fallback(path: &Path) -> bool {
    path.parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("yggterm")
}

#[cfg(unix)]
fn daemon_logical_home_for_endpoint(endpoint: &ServerEndpoint) -> Option<PathBuf> {
    match endpoint {
        ServerEndpoint::UnixSocket(path) if unix_socket_path_uses_runtime_fallback(path) => {
            resolve_yggterm_home()
                .ok()
                .or_else(|| path.parent().map(Path::to_path_buf))
        }
        ServerEndpoint::UnixSocket(path) => path.parent().map(Path::to_path_buf),
        ServerEndpoint::Tcp { .. } => resolve_yggterm_home().ok(),
    }
}

fn uses_runtime_owned_terminal_path(path: &str) -> bool {
    path.starts_with("remote-session://") || path.starts_with("codex-runtime://")
}

fn terminal_launch_command_for_path(
    _path: &str,
    stored_launch_command: String,
    _legacy_direct_attach_launch_command: Option<String>,
) -> String {
    stored_launch_command
}

fn remote_resume_stale_attach(
    has_runtime_output: bool,
    runtime_age_ms: u64,
    runtime_output_requires_restart: bool,
    startup_grace_ms: u64,
) -> bool {
    runtime_output_requires_restart || (!has_runtime_output && runtime_age_ms >= startup_grace_ms)
}

fn remote_resume_saved_session_mismatch_requires_restart(
    remote_resume_path: bool,
    has_runtime_output: bool,
    runtime_age_ms: u64,
    runtime_saved_session_mismatch: bool,
    startup_grace_ms: u64,
) -> bool {
    let _ = (
        remote_resume_path,
        has_runtime_output,
        runtime_age_ms,
        runtime_saved_session_mismatch,
        startup_grace_ms,
    );
    false
}

fn terminal_reuse_needs_restart(
    still_running: bool,
    remote_resume_path: bool,
    restart_protected_runtime: bool,
    stale_remote_attach: bool,
    blank_remote_attach: bool,
    remote_saved_session_mismatch_requires_restart: bool,
    spec_matches: bool,
) -> (bool, bool) {
    if !still_running {
        return (true, false);
    }
    let restart_blocked_by_protected_runtime = remote_resume_path && restart_protected_runtime;
    let restart_requested = stale_remote_attach
        || blank_remote_attach
        || remote_saved_session_mismatch_requires_restart
        || !spec_matches;
    (
        restart_requested && !restart_blocked_by_protected_runtime,
        restart_requested && restart_blocked_by_protected_runtime,
    )
}

fn terminal_read_runtime_recovery_reason(
    path: &str,
    runtime_path: &str,
    terminals: &TerminalManager,
) -> Option<&'static str> {
    if !uses_runtime_owned_terminal_path(path) {
        return None;
    }
    if terminals.session_is_running(runtime_path) {
        if terminals.session_hit_eof_without_output(runtime_path)
            && !terminals.session_has_runtime_output(runtime_path)
        {
            return Some("eof_without_output");
        }
        return None;
    }
    let Some(snapshot) = terminals.session_snapshot(runtime_path) else {
        return Some("missing_runtime_before_read");
    };
    if snapshot.trim().is_empty() {
        return Some("exited_empty_runtime_before_read");
    }
    if crate::remote_resume_runtime_output_requires_restart(snapshot.as_bytes()) {
        return Some("exited_restartable_runtime_output_before_read");
    }
    None
}

fn preserved_owner_saved_session_mismatch_should_detach(
    path: &str,
    keep_alive_runtime: bool,
    temporary_update_restore: bool,
    runtime_output_mismatches_path: bool,
) -> bool {
    runtime_output_mismatches_path
        && !temporary_update_restore
        && !(path.starts_with("remote-session://") && keep_alive_runtime)
}

fn remove_session_should_detach_keep_alive_runtime(keep_alive_runtime: bool) -> bool {
    let _ = keep_alive_runtime;
    false
}

fn valid_initial_terminal_size(cols: Option<u16>, rows: Option<u16>) -> Option<(u16, u16)> {
    match (cols, rows) {
        (Some(cols), Some(rows)) if cols > 0 && rows > 0 => Some((cols, rows)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEndpoint {
    #[cfg(unix)]
    UnixSocket(PathBuf),
    Tcp {
        host: String,
        port: u16,
    },
}

const HOT_UPDATE_OWNERS_FILE: &str = "hot-update-terminal-owners.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "snake_case")]
enum PreservedOwnerEndpoint {
    #[cfg(unix)]
    Unix {
        path: String,
    },
    Tcp {
        host: String,
        port: u16,
    },
}

impl PreservedOwnerEndpoint {
    fn from_endpoint(endpoint: &ServerEndpoint) -> Self {
        match endpoint {
            #[cfg(unix)]
            ServerEndpoint::UnixSocket(path) => Self::Unix {
                path: path.display().to_string(),
            },
            ServerEndpoint::Tcp { host, port } => Self::Tcp {
                host: host.clone(),
                port: *port,
            },
        }
    }

    fn to_endpoint(&self) -> ServerEndpoint {
        match self {
            #[cfg(unix)]
            Self::Unix { path } => ServerEndpoint::UnixSocket(PathBuf::from(path)),
            Self::Tcp { host, port } => ServerEndpoint::Tcp {
                host: host.clone(),
                port: *port,
            },
        }
    }

    fn label(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix { path } => path.clone(),
            Self::Tcp { host, port } => format!("{host}:{port}"),
        }
    }
}

/// Transport-shaped owner-probe failures (timeout / dead socket): the owner
/// is unreachable; a status() re-probe would burn another full request
/// timeout against the same dead socket (the post-swap 30s deaf window).
fn preserved_owner_error_is_transport_shaped(error: &anyhow::Error) -> bool {
    let error_text = format!("{error:#}").to_ascii_lowercase();
    error_text.contains("reading daemon response")
        || error_text.contains("connecting to")
        || error_text.contains("timed out")
}

/// The version named by the direct-install state on disk, if readable.
fn staged_direct_install_version() -> Option<String> {
    let root = yggterm_core::direct_install_root().ok()?;
    let raw = fs::read_to_string(root.join("install-state.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value
        .get("active_version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// A client close that happens while an update is staged is part of an update
/// restart, not a user abandoning their sessions — preserve non-keep-alive
/// live sessions (gate #5 of the persistence saga).
fn client_close_should_preserve_for_update(
    update_restart_state_written: bool,
    staged_version: Option<&str>,
    running_version: &str,
) -> bool {
    update_restart_state_written
        || staged_version
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .is_some_and(|version| version != running_version)
}

fn owner_endpoint_label(endpoint: &ServerEndpoint) -> String {
    PreservedOwnerEndpoint::from_endpoint(endpoint).label()
}

fn server_endpoints_same_target(left: &ServerEndpoint, right: &ServerEndpoint) -> bool {
    match (left, right) {
        #[cfg(unix)]
        (ServerEndpoint::UnixSocket(left_path), ServerEndpoint::UnixSocket(right_path)) => {
            if left_path == right_path {
                return true;
            }
            let left_identity = fs::canonicalize(left_path).unwrap_or_else(|_| left_path.clone());
            let right_identity =
                fs::canonicalize(right_path).unwrap_or_else(|_| right_path.clone());
            left_identity == right_identity
        }
        (
            ServerEndpoint::Tcp {
                host: left_host,
                port: left_port,
            },
            ServerEndpoint::Tcp {
                host: right_host,
                port: right_port,
            },
        ) => left_host == right_host && left_port == right_port,
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreservedTerminalOwnerEntry {
    runtime_key: String,
    endpoint: PreservedOwnerEndpoint,
    owner_server_version: String,
    owner_server_build_id: u64,
    owner_server_pid: u32,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreservedTerminalOwnerRegistry {
    #[serde(default = "preserved_terminal_owner_schema_version")]
    schema_version: u32,
    #[serde(default)]
    expected_server_version: Option<String>,
    #[serde(default)]
    entries: Vec<PreservedTerminalOwnerEntry>,
}

fn preserved_terminal_owner_schema_version() -> u32 {
    1
}

impl Default for PreservedTerminalOwnerRegistry {
    fn default() -> Self {
        Self {
            schema_version: preserved_terminal_owner_schema_version(),
            expected_server_version: None,
            entries: Vec::new(),
        }
    }
}

impl PreservedTerminalOwnerRegistry {
    fn path(home_dir: &Path) -> PathBuf {
        home_dir.join(HOT_UPDATE_OWNERS_FILE)
    }

    fn load(home_dir: &Path) -> Self {
        let path = Self::path(home_dir);
        let Ok(bytes) = fs::read(&path) else {
            return Self {
                schema_version: preserved_terminal_owner_schema_version(),
                expected_server_version: None,
                entries: Vec::new(),
            };
        };
        serde_json::from_slice::<Self>(&bytes).unwrap_or_default()
    }

    fn save(&self, home_dir: &Path) -> Result<()> {
        let path = Self::path(home_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating hot-update owner dir {}", parent.display()))?;
        }
        let temp_path = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).context("serializing hot-update owners")?;
        fs::write(&temp_path, json)
            .with_context(|| format!("writing hot-update owners temp {}", temp_path.display()))?;
        fs::rename(&temp_path, &path)
            .or_else(|_| {
                fs::copy(&temp_path, &path)?;
                fs::remove_file(&temp_path)?;
                Ok::<(), std::io::Error>(())
            })
            .with_context(|| format!("writing hot-update owners {}", path.display()))
    }

    fn write_handoff(
        home_dir: &Path,
        owner_endpoint: &ServerEndpoint,
        owner_status: &ServerRuntimeStatus,
        expected_server_version: Option<String>,
        runtime_keys: Vec<String>,
        existing_entries: Vec<PreservedTerminalOwnerEntry>,
    ) -> Result<Self> {
        let existing_registry = Self::load(home_dir);
        if !existing_registry.entries.is_empty()
            && let Some((existing_expected, requested_expected)) = hot_update_target_regression(
                existing_registry.expected_server_version.as_deref(),
                expected_server_version.as_deref(),
            )
        {
            append_trace_event(
                home_dir,
                "daemon",
                "hot_update",
                "handoff_target_regression_refused",
                serde_json::json!({
                    "existing_expected_server_version": existing_expected,
                    "requested_expected_server_version": requested_expected,
                    "owner_endpoint": owner_endpoint_label(owner_endpoint),
                    "owner_server_version": &owner_status.server_version,
                    "owner_server_pid": owner_status.server_pid,
                    "runtime_keys": runtime_keys,
                    "reason": "newer_handoff_target_already_registered",
                }),
            );
            bail!(
                "refusing hot-update handoff target regression from {existing_expected} to {requested_expected}; preserving the newer session-survival target"
            );
        }
        let owner_endpoint = PreservedOwnerEndpoint::from_endpoint(owner_endpoint);
        let now_ms = current_millis_u64();
        let local_keys = runtime_keys.into_iter().collect::<HashSet<_>>();
        let mut entries = existing_entries
            .into_iter()
            .filter(|entry| !local_keys.contains(&entry.runtime_key))
            .collect::<Vec<_>>();
        entries.extend(
            local_keys
                .into_iter()
                .map(|runtime_key| PreservedTerminalOwnerEntry {
                    runtime_key,
                    endpoint: owner_endpoint.clone(),
                    owner_server_version: owner_status.server_version.clone(),
                    owner_server_build_id: owner_status.server_build_id,
                    owner_server_pid: owner_status.server_pid,
                    created_at_ms: now_ms,
                }),
        );
        let mut entries = entries;
        entries.sort_by(|left, right| left.runtime_key.cmp(&right.runtime_key));
        entries.dedup_by(|left, right| left.runtime_key == right.runtime_key);
        let registry = Self {
            schema_version: preserved_terminal_owner_schema_version(),
            expected_server_version,
            entries,
        };
        registry.save(home_dir)?;
        Ok(registry)
    }

    fn retarget_expected_server_version(
        &mut self,
        home_dir: &Path,
        expected_server_version: Option<String>,
    ) -> Result<bool> {
        if self.entries.is_empty() || self.expected_server_version == expected_server_version {
            return Ok(false);
        }
        if let Some((existing_expected, requested_expected)) = hot_update_target_regression(
            self.expected_server_version.as_deref(),
            expected_server_version.as_deref(),
        ) {
            append_trace_event(
                home_dir,
                "daemon",
                "hot_update",
                "handoff_target_regression_refused",
                serde_json::json!({
                    "existing_expected_server_version": existing_expected,
                    "requested_expected_server_version": requested_expected,
                    "runtime_keys": self.keys(),
                    "reason": "retarget_would_downgrade_existing_handoff",
                }),
            );
            bail!(
                "refusing hot-update handoff target regression from {existing_expected} to {requested_expected}; preserving the newer session-survival target"
            );
        }
        self.expected_server_version = expected_server_version;
        self.save(home_dir)?;
        Ok(true)
    }

    fn retarget_loaded_registry_for_current_version(&mut self, home_dir: &Path) -> Result<bool> {
        if self.expected_server_version.as_deref() == Some(SERVER_PROTOCOL_VERSION) {
            return Ok(false);
        }
        self.retarget_expected_server_version(home_dir, Some(SERVER_PROTOCOL_VERSION.to_string()))
    }

    fn retarget_current_alias_entries(
        &mut self,
        home_dir: &Path,
        current_endpoint: &ServerEndpoint,
        reachable_owner_statuses: &[(ServerEndpoint, ServerRuntimeStatus)],
        current_pid: u32,
        reason: &'static str,
    ) -> Result<bool> {
        if self.entries.is_empty() {
            return Ok(false);
        }
        let before = self.entries.clone();
        let mut retargeted = Vec::new();
        let mut unresolved = Vec::new();
        for entry in &mut self.entries {
            let entry_endpoint = entry.endpoint.to_endpoint();
            if !server_endpoints_same_target(&entry_endpoint, current_endpoint) {
                continue;
            }
            let candidate = preserved_owner_candidate_for_runtime_key(
                reachable_owner_statuses.to_vec(),
                &entry.runtime_key,
                current_pid,
            );
            let Some((owner_endpoint, owner_status)) = candidate else {
                unresolved.push(serde_json::json!({
                    "runtime_key": entry.runtime_key,
                    "old_endpoint": owner_endpoint_label(&entry_endpoint),
                    "old_owner_server_version": entry.owner_server_version,
                    "old_owner_server_pid": entry.owner_server_pid,
                }));
                continue;
            };
            if server_endpoints_same_target(&owner_endpoint, current_endpoint) {
                unresolved.push(serde_json::json!({
                    "runtime_key": entry.runtime_key,
                    "old_endpoint": owner_endpoint_label(&entry_endpoint),
                    "candidate_endpoint": owner_endpoint_label(&owner_endpoint),
                    "candidate_owner_server_version": owner_status.server_version,
                    "candidate_owner_server_pid": owner_status.server_pid,
                    "reason": "candidate_is_current_endpoint",
                }));
                continue;
            }
            retargeted.push(serde_json::json!({
                "runtime_key": entry.runtime_key,
                "old_endpoint": owner_endpoint_label(&entry_endpoint),
                "new_endpoint": owner_endpoint_label(&owner_endpoint),
                "old_owner_server_version": entry.owner_server_version,
                "new_owner_server_version": owner_status.server_version,
                "old_owner_server_pid": entry.owner_server_pid,
                "new_owner_server_pid": owner_status.server_pid,
            }));
            entry.endpoint = PreservedOwnerEndpoint::from_endpoint(&owner_endpoint);
            entry.owner_server_version = owner_status.server_version;
            entry.owner_server_build_id = owner_status.server_build_id;
            entry.owner_server_pid = owner_status.server_pid;
            entry.created_at_ms = current_millis_u64();
        }
        if self.entries == before {
            if !unresolved.is_empty() {
                append_trace_event(
                    home_dir,
                    "daemon",
                    "hot_update",
                    "preserved_owner_current_alias_unresolved",
                    serde_json::json!({
                        "reason": reason,
                        "current_endpoint": owner_endpoint_label(current_endpoint),
                        "unresolved": unresolved,
                    }),
                );
            }
            return Ok(false);
        }
        self.entries
            .sort_by(|left, right| left.runtime_key.cmp(&right.runtime_key));
        self.entries
            .dedup_by(|left, right| left.runtime_key == right.runtime_key);
        self.save(home_dir)?;
        append_trace_event(
            home_dir,
            "daemon",
            "hot_update",
            "preserved_owner_current_alias_retargeted",
            serde_json::json!({
                "reason": reason,
                "current_endpoint": owner_endpoint_label(current_endpoint),
                "retargeted": retargeted,
                "unresolved": unresolved,
                "remaining_runtime_keys": self.keys(),
            }),
        );
        Ok(true)
    }

    fn keys(&self) -> Vec<String> {
        let mut keys = self
            .entries
            .iter()
            .map(|entry| entry.runtime_key.clone())
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        keys
    }

    fn endpoint_groups(&self) -> Vec<(ServerEndpoint, Vec<String>)> {
        let mut groups = BTreeMap::<String, (ServerEndpoint, Vec<String>)>::new();
        for entry in &self.entries {
            let label = entry.endpoint.label();
            let endpoint = entry.endpoint.to_endpoint();
            groups
                .entry(label)
                .or_insert_with(|| (endpoint, Vec::new()))
                .1
                .push(entry.runtime_key.clone());
        }
        groups
            .into_values()
            .map(|(endpoint, mut runtime_keys)| {
                runtime_keys.sort();
                runtime_keys.dedup();
                (endpoint, runtime_keys)
            })
            .collect()
    }

    fn retain_represented_keys<F>(&mut self, mut is_represented: F) -> Vec<String>
    where
        F: FnMut(&str) -> bool,
    {
        let mut removed = Vec::new();
        self.entries.retain(|entry| {
            if is_represented(&entry.runtime_key) {
                true
            } else {
                removed.push(entry.runtime_key.clone());
                false
            }
        });
        removed.sort();
        removed.dedup();
        removed
    }

    fn owner_for_key(&self, runtime_key: &str) -> Option<&PreservedTerminalOwnerEntry> {
        self.entries
            .iter()
            .find(|entry| entry.runtime_key == runtime_key)
    }

    fn upsert_runtime_owner(
        &mut self,
        home_dir: &Path,
        runtime_key: &str,
        owner_endpoint: &ServerEndpoint,
        owner_status: &ServerRuntimeStatus,
        expected_server_version: Option<String>,
    ) -> Result<bool> {
        if let Some((existing_expected, requested_expected)) = hot_update_target_regression(
            self.expected_server_version.as_deref(),
            expected_server_version.as_deref(),
        ) {
            append_trace_event(
                home_dir,
                "daemon",
                "hot_update",
                "handoff_target_regression_refused",
                serde_json::json!({
                    "existing_expected_server_version": existing_expected,
                    "requested_expected_server_version": requested_expected,
                    "runtime_key": runtime_key,
                    "owner_endpoint": owner_endpoint_label(owner_endpoint),
                    "owner_server_version": &owner_status.server_version,
                    "owner_server_pid": owner_status.server_pid,
                    "reason": "discovered_owner_would_downgrade_existing_handoff",
                }),
            );
            bail!(
                "refusing hot-update handoff target regression from {existing_expected} to {requested_expected}; preserving the newer session-survival target"
            );
        }

        let owner_endpoint = PreservedOwnerEndpoint::from_endpoint(owner_endpoint);
        if self.expected_server_version != expected_server_version {
            self.expected_server_version = expected_server_version;
        }
        let before = self.entries.clone();
        self.entries
            .retain(|entry| entry.runtime_key != runtime_key);
        self.entries.push(PreservedTerminalOwnerEntry {
            runtime_key: runtime_key.to_string(),
            endpoint: owner_endpoint,
            owner_server_version: owner_status.server_version.clone(),
            owner_server_build_id: owner_status.server_build_id,
            owner_server_pid: owner_status.server_pid,
            created_at_ms: current_millis_u64(),
        });
        self.entries
            .sort_by(|left, right| left.runtime_key.cmp(&right.runtime_key));
        self.entries
            .dedup_by(|left, right| left.runtime_key == right.runtime_key);
        if self.entries == before {
            return Ok(false);
        }
        self.save(home_dir)?;
        Ok(true)
    }

    fn remove_key(&mut self, runtime_key: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.runtime_key != runtime_key);
        before != self.entries.len()
    }
}

fn snapshot_session_metadata_value(session: &SnapshotSessionView, label: &str) -> Option<String> {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == label)
        .map(|entry| entry.value.clone())
}

fn snapshot_session_keep_alive(session: &SnapshotSessionView) -> bool {
    snapshot_session_metadata_value(session, "Runtime Persistence").as_deref() == Some("keep-alive")
}

fn snapshot_session_update_restore(session: &SnapshotSessionView) -> bool {
    snapshot_session_metadata_value(session, "Runtime Restore Reason").as_deref()
        == Some(crate::UPDATE_RESTART_RESTORE_REASON)
}

fn snapshot_session_is_preserved_owner_recoverable(session: &SnapshotSessionView) -> bool {
    snapshot_session_keep_alive(session) || snapshot_session_update_restore(session)
}

fn persisted_live_session_from_preserved_owner_snapshot(
    session: &SnapshotSessionView,
) -> Option<PersistedLiveSession> {
    if !matches!(
        session.source,
        SessionSource::LiveLocal | SessionSource::LiveSsh
    ) {
        return None;
    }
    let ssh_target = session.ssh_target.clone()?;
    Some(PersistedLiveSession {
        key: session.session_path.clone(),
        id: session.id.clone(),
        title: session.title.clone(),
        kind: session.kind,
        keep_alive: snapshot_session_keep_alive(session),
        ssh_target,
        prefix: session.ssh_prefix.clone(),
        cwd: snapshot_session_metadata_value(session, "Cwd"),
        remote_launch_action: snapshot_session_metadata_value(session, "Remote Launch Action"),
        storage_path: snapshot_session_metadata_value(session, "Storage"),
        restore_reason: Some(crate::UPDATE_RESTART_RESTORE_REASON.to_string()),
    })
}

fn restore_preserved_owner_live_sessions_from_snapshot(
    server: &mut YggtermServer,
    owner_snapshot: &ServerUiSnapshot,
    runtime_keys: &HashSet<String>,
) -> Vec<String> {
    restore_preserved_owner_live_sessions_from_snapshot_with_policy(
        server,
        owner_snapshot,
        runtime_keys,
        false,
    )
}

fn restore_preserved_owner_live_sessions_from_snapshot_with_policy(
    server: &mut YggtermServer,
    owner_snapshot: &ServerUiSnapshot,
    runtime_keys: &HashSet<String>,
    require_recoverable_metadata: bool,
) -> Vec<String> {
    let mut restored = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    let owner_sessions = owner_snapshot
        .live_sessions
        .iter()
        .chain(owner_snapshot.active_session.iter());
    for session in owner_sessions {
        if !seen.insert(session.session_path.clone()) {
            continue;
        }
        let runtime_key = server.terminal_runtime_key_for_path(&session.session_path);
        if !runtime_keys.contains(&session.session_path) && !runtime_keys.contains(&runtime_key) {
            continue;
        }
        if server.represents_terminal_runtime_key(&runtime_key) {
            continue;
        }
        if require_recoverable_metadata && !snapshot_session_is_preserved_owner_recoverable(session)
        {
            continue;
        }
        let Some(live) = persisted_live_session_from_preserved_owner_snapshot(session) else {
            continue;
        };
        server.restore_live_session(live);
        restored.push(runtime_key);
    }
    restored.sort();
    restored.dedup();
    restored
}

fn runtime_status_reports_terminal_key(status: &ServerRuntimeStatus, runtime_key: &str) -> bool {
    status
        .owned_terminal_session_keys
        .iter()
        .any(|key| key == runtime_key)
        || status
            .terminal_session_keys
            .iter()
            .any(|key| key == runtime_key)
}

fn runtime_status_direct_terminal_keys(status: &ServerRuntimeStatus) -> Vec<String> {
    let mut keys = if status.owned_terminal_session_keys.is_empty() {
        status.terminal_session_keys.clone()
    } else {
        status.owned_terminal_session_keys.clone()
    };
    keys.sort();
    keys.dedup();
    keys
}

fn preserved_owner_entries_live_for_handoff(
    entries: &[PreservedTerminalOwnerEntry],
    outgoing_owned_runtime_keys: &HashSet<String>,
    represented_preserved_owner_keys: &HashSet<String>,
    owner_status_cache: &BTreeMap<String, Option<ServerRuntimeStatus>>,
) -> Vec<PreservedTerminalOwnerEntry> {
    let mut live_entries = entries
        .iter()
        .filter(|entry| {
            if outgoing_owned_runtime_keys.contains(&entry.runtime_key) {
                return false;
            }
            if represented_preserved_owner_keys.contains(&entry.runtime_key) {
                return true;
            }
            owner_status_cache
                .get(&entry.endpoint.label())
                .and_then(|status| status.as_ref())
                .is_some_and(|status| {
                    runtime_status_reports_terminal_key(status, &entry.runtime_key)
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    live_entries.sort_by(|left, right| left.runtime_key.cmp(&right.runtime_key));
    live_entries.dedup_by(|left, right| left.runtime_key == right.runtime_key);
    live_entries
}

fn represented_live_session_needs_preserved_owner(
    server: &YggtermServer,
    terminals: &TerminalManager,
    runtime_key: &str,
) -> bool {
    !terminals.has_session(runtime_key)
        && server.represents_terminal_runtime_key(runtime_key)
        && (server.live_session_keep_alive(runtime_key)
            || server.live_session_is_temporary_update_restore(runtime_key))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRuntimeStatus {
    pub server_version: String,
    #[serde(default)]
    pub server_build_id: u64,
    #[serde(default)]
    pub server_pid: u32,
    pub host_kind: String,
    pub host_detail: String,
    pub embedded_surface_supported: bool,
    pub bridge_enabled: bool,
    #[serde(default)]
    pub restored_from_persisted_state: bool,
    #[serde(default)]
    pub restored_stored_sessions: usize,
    #[serde(default)]
    pub restored_live_sessions: usize,
    #[serde(default)]
    pub restored_remote_machines: usize,
    #[serde(default)]
    pub owned_terminal_session_count: usize,
    #[serde(default)]
    pub owned_terminal_session_keys: Vec<String>,
    #[serde(default)]
    pub terminal_session_count: usize,
    #[serde(default)]
    pub terminal_session_keys: Vec<String>,
    #[serde(default)]
    pub preserved_terminal_owner_count: usize,
    #[serde(default)]
    pub preserved_terminal_owner_keys: Vec<String>,
    #[serde(default)]
    pub terminal_retained_chunks: usize,
    #[serde(default)]
    pub terminal_retained_bytes: usize,
    #[serde(default)]
    pub terminal_session_buffer_limit_bytes: usize,
    #[serde(default)]
    pub terminal_idle_buffer_limit_bytes: usize,
    #[serde(default)]
    pub managed_session_count: usize,
    #[serde(default)]
    pub session_metadata_entries: usize,
    #[serde(default)]
    pub session_metadata_bytes: usize,
    #[serde(default)]
    pub session_preview_block_count: usize,
    #[serde(default)]
    pub session_preview_line_count: usize,
    #[serde(default)]
    pub session_preview_bytes: usize,
    #[serde(default)]
    pub session_rendered_section_count: usize,
    #[serde(default)]
    pub session_rendered_line_count: usize,
    #[serde(default)]
    pub session_rendered_bytes: usize,
    #[serde(default)]
    pub session_terminal_line_count: usize,
    #[serde(default)]
    pub session_terminal_bytes: usize,
    #[serde(default)]
    pub session_payload_total_bytes: usize,
    /// Epoch-ms this daemon process started, and how long it has been up. The user
    /// cannot otherwise tell a daemon that swapped 30s ago from one that has been
    /// pinned for 19 hours — and that difference is usually the whole story.
    #[serde(default)]
    pub daemon_started_at_ms: u64,
    #[serde(default)]
    pub daemon_uptime_ms: u64,
    /// Build id of the binary this process is RUNNING vs the one on DISK. When they
    /// differ, a deploy has landed a newer build that this daemon has not picked up.
    #[serde(default)]
    pub running_build_id: u64,
    #[serde(default)]
    pub on_disk_build_id: u64,
    #[serde(default)]
    pub hot_restart_pending: bool,
    /// Why a hot-restart is being DEFERRED right now, in the daemon's own words —
    /// `None` means nothing is blocking it. The daemon has always computed this to
    /// decide whether to retire; it just never told anyone, so a pinned daemon looked
    /// identical to a healthy one.
    #[serde(default)]
    pub hot_restart_block_reason: Option<String>,
    /// EVERY session currently deferring the restart, not just the first. The one-line
    /// `hot_restart_block_reason` above is a summary of exactly this list; a client that
    /// wants to show the user what to clear (or an agent mining why a daemon went stale)
    /// reads the structured form. `#[serde(default)]` so an older daemon's status still
    /// parses — it simply reports no blockers.
    #[serde(default)]
    pub hot_restart_blockers: Vec<HotRestartBlocker>,
}

/// A session is working right now (`esc to interrupt` on its screen).
pub const HOT_RESTART_BLOCKER_WORKING: &str = "working";
/// A session was active inside the idle window, so a swap could still eat a turn.
pub const HOT_RESTART_BLOCKER_RECENTLY_ACTIVE: &str = "recently_active";

/// One session holding a hot-restart open, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotRestartBlocker {
    pub session_key: String,
    /// [`HOT_RESTART_BLOCKER_WORKING`] or [`HOT_RESTART_BLOCKER_RECENTLY_ACTIVE`].
    pub kind: String,
    /// How long since this session last produced output. `None` when unreadable.
    #[serde(default)]
    pub idle_ms: Option<u64>,
    /// The idle window the session is being measured against.
    #[serde(default)]
    pub threshold_ms: u64,
}

/// Collapse the blocker list into the single line the metadata panel prints. Pure, so the
/// wording is unit-testable and cannot drift from the predicate the daemon acts on.
pub fn hot_restart_block_reason_summary(blockers: &[HotRestartBlocker]) -> Option<String> {
    let first = blockers.first()?;
    let head = match first.kind.as_str() {
        HOT_RESTART_BLOCKER_WORKING => {
            format!("{} is working (esc to interrupt)", first.session_key)
        }
        _ => match first.idle_ms {
            Some(idle_ms) => format!(
                "{} was active {}s ago (idle window {}s)",
                first.session_key,
                idle_ms / 1_000,
                first.threshold_ms / 1_000
            ),
            None => format!("{} was recently active", first.session_key),
        },
    };
    // Naming only the first blocker is what made this opaque: clear that session and the
    // restart still defers, on a session the panel never mentioned. Say how many more.
    match blockers.len() {
        1 => Some(head),
        n => Some(format!("{head} (+{} more session(s))", n - 1)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalStreamChunk {
    pub seq: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerRequest {
    Ping,
    Status,
    WorkingFlags,
    Snapshot,
    PrepareUpdateRestart,
    PrepareClientClose,
    HotRestart {
        daemon_executable: String,
        expected_version: Option<String>,
        expected_build_id: Option<u64>,
        reason: Option<String>,
        /// When `true`, the daemon bypasses the same-version refusal check
        /// and proceeds with the handoff even when the target version
        /// equals the current `SERVER_PROTOCOL_VERSION`. Used by dev/agent
        /// deploys where the build_id changed but the version_string didn't.
        /// See [[bug-class-auto-hot-restart-version-gated]].
        #[serde(default)]
        force: bool,
    },
    RetireDaemon {
        reason: Option<String>,
    },
    OpenStoredSession {
        session_kind: SessionKind,
        path: String,
        session_id: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
        view_mode: Option<WorkspaceViewMode>,
    },
    ConnectSsh {
        target_ix: usize,
    },
    ConnectSshCustom {
        target: String,
        prefix: Option<String>,
    },
    StartSshSession {
        target: String,
        prefix: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
        /// Live-order anchor: place the new session directly below this row.
        #[serde(default)]
        insert_after: Option<String>,
    },
    StartRemoteCodexSession {
        target: String,
        prefix: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        insert_after: Option<String>,
    },
    StartRemoteClaudeSession {
        target: String,
        prefix: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        insert_after: Option<String>,
    },
    OpenRemoteSession {
        machine_key: String,
        session_id: String,
        cwd: Option<String>,
        title_hint: Option<String>,
        view_mode: Option<WorkspaceViewMode>,
    },
    RefreshRemoteMachine {
        machine_key: String,
    },
    RefreshManagedCli {
        machine_key: Option<String>,
        background: bool,
    },
    RefreshPreview {
        path: String,
        #[serde(default)]
        full_remote_payload: bool,
    },
    UpdateSessionCopy {
        path: String,
        title: Option<String>,
        precis: Option<String>,
        summary: Option<String>,
    },
    RemoveSshTarget {
        machine_key: String,
    },
    RemoveSession {
        path: String,
    },
    DropTerminalRuntime {
        runtime_key: String,
        reason: Option<String>,
    },
    SetSessionKeepAlive {
        path: String,
        keep_alive: bool,
    },
    ReorderLiveSessions {
        ordered_paths: Vec<String>,
        /// Stable client identity (e.g. `gui:<host>`): the reorder is also
        /// recorded into this client's row-order ledger scope, so multiple
        /// GUIs attached to the same daemon can keep independent arrangements.
        #[serde(default)]
        client_scope: Option<String>,
    },
    /// Read-only report of the row-order ledger (all scopes, or one).
    RowOrderLedgerReport {
        #[serde(default)]
        scope: Option<String>,
    },
    StartLocalSession {
        session_kind: SessionKind,
        cwd: Option<String>,
        title_hint: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        insert_after: Option<String>,
    },
    SwitchAgentSessionMode {
        path: String,
        session_kind: SessionKind,
    },
    StartCommandSession {
        cwd: Option<String>,
        title_hint: Option<String>,
        launch_command: String,
        source_label: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
    },
    EnsureRemoteRuntimeCodexSession {
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    StartRemoteRuntimeCodexSession {
        session_id: String,
        cwd: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    /// Claude Code twins of the codex daemon-runtime requests — same lane,
    /// CC PTY owned by this host's daemon ([[spec-unify-local-remote]]).
    /// Separate variants (not a kind field on the codex ones) so an OLD
    /// daemon fails LOUDLY on version skew instead of resuming the wrong CLI.
    EnsureRemoteRuntimeCcSession {
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
        /// Client-side configured claude extra CLI args (raw settings string)
        /// forwarded so the daemon-spawned `claude` carries them — the host's
        /// own settings store does not have the client's configuration.
        #[serde(default)]
        claude_extra_args: Option<String>,
    },
    StartRemoteRuntimeCcSession {
        session_id: String,
        cwd: Option<String>,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
        #[serde(default)]
        claude_extra_args: Option<String>,
    },
    /// Daemon-owned resumable plain-shell session (tmux replacement for
    /// `server attach`). The host daemon owns/persists the shell PTY.
    EnsureShellSession {
        session_id: String,
        cwd: Option<String>,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    FocusLive {
        key: String,
        view_mode: Option<WorkspaceViewMode>,
    },
    SetViewMode {
        mode: WorkspaceViewMode,
    },
    TogglePreviewBlock {
        block_ix: usize,
    },
    SetAllPreviewBlocksFolded {
        folded: bool,
    },
    RequestTerminalLaunch,
    RequestTerminalLaunchForPath {
        path: String,
    },
    TerminalEnsure {
        path: String,
    },
    TerminalRead {
        path: String,
        cursor: u64,
    },
    TerminalSnapshot {
        path: String,
    },
    TerminalRetainedSnapshot {
        path: String,
    },
    /// Diagnostic: the daemon's CLEAN scrolled-off vt100 scrollback rows for a
    /// session (the history that CAN load into xterm scrollback). Used to confirm
    /// whether the daemon actually holds a codex session's transcript (proving the
    /// scroll-lock is a client reveal/load gap) vs the parser never capturing it.
    TerminalHistory {
        path: String,
    },
    TerminalWrite {
        path: String,
        data: String,
    },
    TerminalResize {
        path: String,
        cols: u16,
        rows: u16,
    },
    TerminalRestart {
        path: String,
        #[serde(default)]
        terminal_appearance: Option<String>,
        #[serde(default)]
        force_remote: bool,
        #[serde(default)]
        initial_cols: Option<u16>,
        #[serde(default)]
        initial_rows: Option<u16>,
    },
    SyncExternalWindow,
    RaiseExternalWindow,
    SyncTheme {
        theme: UiTheme,
    },
    SyncTerminalIdentity {
        terminal_appearance: String,
        #[serde(default)]
        terminal_profile: Option<TerminalIdentityColorProfile>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerResponse {
    Pong,
    Status(ServerRuntimeStatus),
    WorkingFlags {
        flags: Vec<(String, bool)>,
    },
    Snapshot {
        snapshot: ServerUiSnapshot,
        message: Option<String>,
    },
    TerminalStream {
        cursor: u64,
        chunks: Vec<TerminalStreamChunk>,
        running: bool,
        runtime_output_seen: bool,
        eof_without_output: bool,
        #[serde(default)]
        post_resize_output_seen: bool,
        #[serde(default)]
        last_resize_seq: u64,
        // The live chunk ring trimmed below this read's cursor — the surviving
        // chunks skip a contiguous middle, so the client must re-attach to recover
        // it (docs/xterm-bugs.md#chunk-ring-trim-drops-mid-stream). `serde(default)`
        // keeps it cross-version safe: an older daemon that doesn't send the field
        // deserializes as `false`, and older clients ignore it.
        #[serde(default)]
        resync_required: bool,
    },
    TerminalSnapshot {
        text: String,
        running: bool,
        runtime_output_seen: bool,
        #[serde(default)]
        post_resize_output_seen: bool,
        #[serde(default)]
        last_resize_seq: u64,
        // Unique id of the PTY spawn this snapshot was read from (vacuum-guard
        // cold-re-resume signal). `serde(default)` keeps it cross-version safe:
        // an older daemon deserializes as 0 = unknown, and the client guard
        // fails OPEN (no guard) on 0.
        #[serde(default)]
        runtime_spawn_id: u64,
    },
    TerminalRetainedSnapshot {
        text: String,
        running: bool,
        runtime_output_seen: bool,
        #[serde(default)]
        post_resize_output_seen: bool,
        #[serde(default)]
        last_resize_seq: u64,
        #[serde(default)]
        runtime_spawn_id: u64,
    },
    TerminalHistory {
        #[serde(default)]
        rows: Vec<String>,
        running: bool,
    },
    Ack {
        message: Option<String>,
    },
    HotUpdateHandoff {
        message: Option<String>,
        owner_endpoint: String,
        owner_server_version: String,
        owner_server_pid: u32,
        target_server_version: Option<String>,
        runtime_keys: Vec<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotRestartResult {
    Restarting {
        message: Option<String>,
    },
    Handoff {
        message: Option<String>,
        owner_endpoint: String,
        owner_server_version: String,
        owner_server_pid: u32,
        target_server_version: Option<String>,
        runtime_keys: Vec<String>,
    },
}

impl HotRestartResult {
    pub fn message(&self) -> Option<String> {
        match self {
            HotRestartResult::Restarting { message } => message.clone(),
            HotRestartResult::Handoff { message, .. } => message.clone(),
        }
    }
}

struct DaemonRuntime {
    support: GhosttyHostSupport,
    state_path: PathBuf,
    store: SessionStore,
    server: YggtermServer,
    terminals: TerminalManager,
    preserved_terminal_owners: PreservedTerminalOwnerRegistry,
    /// Durable per-client-scope memory of Live Sessions row slots. Observes
    /// every order change through the persist chokepoint and answers "where
    /// does this row go when it comes back?" — see `row_order_ledger.rs`.
    row_order_ledger: crate::row_order_ledger::RowOrderLedger,
    remote_machine_refreshes_in_flight: HashSet<String>,
    codex_process_identity_cache: Mutex<BTreeMap<u32, CodexRuntimeProcessIdentity>>,
    restored_from_persisted_state: bool,
    restored_stored_sessions: usize,
    restored_live_sessions: usize,
    restored_remote_machines: usize,
    /// Negative cache for preserved-owner probes (post-swap shadow incident
    /// 2026-06-11): probing a dead-but-lingering prior daemon's socket costs
    /// the full 10s request timeout, and the ensure path re-probed the SAME
    /// dead socket repeatedly — one terminal_ensure held the request loop
    /// 30.7s, deafening the daemon while the user stared at the shadow.
    /// endpoint label → epoch-ms until which it is treated unreachable.
    preserved_owner_unreachable_until_ms: HashMap<String, u64>,
    // Last relaunch-recovery attempt per runtime key (terminal-write path);
    // bounds ensure_terminal_for_path to one attempt per cooldown window.
    terminal_write_relaunch_attempted_at_ms: HashMap<String, u64>,
    /// Set once the update-restart snapshot has been written during retire.
    /// LIVE INCIDENT (2026-06-11, every swap): the retiring daemon keeps
    /// serving briefly after writing the protected update-restart state, and
    /// any routine persist() then OVERWRITES it with the keep-alive-only view
    /// before the successor reads it — silently dropping all protected
    /// non-keep-alive sessions (the recurring local-row loss; the persist
    /// drop-telemetry showed ZERO drops because the update snapshot itself
    /// was correct). After this is set, routine persists are no-ops.
    update_restart_state_written: bool,
    /// Run #17 split-brain lesson (structural): cross-daemon duplicate-runtime
    /// pruning probes/drops/retires against OTHER daemons' sockets — against a
    /// live-but-busy duplicate each round-trip can take seconds, and running
    /// that inside a request handler serialized multi-second stalls onto the
    /// request loop (5 consecutive 10s ensure failures; one ensure took 8.8s).
    /// The prune now runs on a background thread; this flag keeps it to one
    /// prune at a time.
    duplicate_runtime_prune_in_flight: Arc<AtomicBool>,
    /// Preserved-owner registry removals discovered by the background prune,
    /// applied on the request loop (the registry is not thread-shared).
    pending_preserved_owner_removals: Arc<Mutex<Vec<(String, &'static str)>>>,
    /// GATE #8 (persistence saga root): multiple daemons share ONE
    /// server-state.json, and a superseded daemon's routine persist clobbers
    /// the successor's file with its stale in-memory view (live-caught: a
    /// lingering 2.8.92 daemon kept erasing the locals the 2.8.93 file held).
    /// Set by the disk-binary poll when a strictly newer daemon is live;
    /// routine persists become no-ops — the successor owns the file now. The
    /// final update-restart snapshot at retire still writes (it carries this
    /// daemon's rows; the successor's takeover import reads it).
    superseded_routine_persist_muted: bool,
    /// Run #19 permanent squish fix: pending remote-PTY resize forwards
    /// (latest-wins per session path) + the in-flight session set. The SSH
    /// round-trip to the remote daemon runs on a background thread, never on
    /// the request loop.
    pending_remote_pty_resizes:
        Arc<Mutex<HashMap<String, (RemoteMachineSnapshot, String, SessionKind, u16, u16)>>>,
    remote_pty_resize_in_flight: Arc<Mutex<HashSet<String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteMachineRefreshQueueStatus {
    Spawn,
    AlreadyInFlight,
    Loopback,
}

#[derive(Debug, Clone)]
struct BackgroundCopyCandidate {
    session_path: String,
    session_id: String,
    cwd: String,
    title: String,
    source_updated_at: Option<OffsetDateTime>,
    remote_machine: Option<RemoteMachineSnapshot>,
    generation_context: Option<String>,
    storage_path: Option<String>,
    cached_summary: Option<String>,
    /// A LIVE local agent session (yggterm-spawned codex/CC). Its live title
    /// is a cwd-derived launch hint (e.g. "home/pi codex"), which the
    /// fallback-title recognizer can't enumerate — so title freshness is
    /// gated on the resolver DB instead (user bug 5: "new codex opens as
    /// cwd-derived title and never updates"). Manual renames live in the
    /// resolver DB and therefore stay protected.
    live_local_agent: bool,
}

#[derive(Debug, Clone)]
struct BackgroundCopyUpdate {
    session_path: String,
    title: Option<String>,
    summary: Option<String>,
}

impl DaemonRuntime {
    fn load(support: GhosttyHostSupport) -> Result<Self> {
        let store = SessionStore::open_or_init()?;
        let perf = PerfSpan::start(store.home_dir(), "daemon", "runtime_load");
        let settings = store.load_settings().unwrap_or_default();
        // Push the profiling toggle into the process-global gate at startup. The chore
        // re-reads settings each tick, so a GUI toggle propagates to the daemon there.
        yggterm_core::set_perf_profiling_enabled(settings.perf_profiling_enabled);
        let tree = store
            .load_codex_tree(&settings)
            .or_else(|_| store.load_tree())?;
        let mut server = YggtermServer::new(
            &tree,
            settings.prefer_ghostty_backend,
            support.clone(),
            settings.theme,
        );
        let state_path = store.home_dir().join("server-state.json");
        let preserved_terminal_owners = PreservedTerminalOwnerRegistry::load(store.home_dir());
        let mut restored_from_persisted_state = false;
        let mut restored_stored_sessions = 0usize;
        let mut restored_live_sessions = 0usize;
        let mut restored_remote_machines = 0usize;
        if let Some(mut saved) = load_persisted_state(&state_path)? {
            // Never resurrect a predecessor's sessions from disk while it is
            // alive and owns their runtimes; it, not this file, is the truth.
            let our_pid = std::process::id();
            let other_daemons: Vec<(u32, usize)> = reachable_versioned_daemon_statuses(store.home_dir())
                .into_iter()
                .filter(|(_, runtime)| runtime.server_pid != our_pid)
                .map(|(_, runtime)| (runtime.server_pid, runtime.owned_terminal_session_count))
                .collect();
            if !may_cold_restore_live_sessions(
                preserved_terminal_owners.expected_server_version.as_deref(),
                SERVER_PROTOCOL_VERSION,
                &other_daemons,
            ) {
                append_trace_event(
                    store.home_dir(),
                    "daemon",
                    "lifecycle",
                    "cold_restore_of_live_sessions_refused",
                    serde_json::json!({
                        "current_version": SERVER_PROTOCOL_VERSION,
                        "current_pid": our_pid,
                        "skipped_live_sessions": saved.live_sessions.len(),
                        "handoff_target_version": preserved_terminal_owners.expected_server_version,
                        "other_daemons": other_daemons,
                    }),
                );
                saved.live_sessions.clear();
            }
            restored_from_persisted_state = true;
            restored_stored_sessions = saved.stored_sessions.len();
            restored_live_sessions = saved.live_sessions.len();
            restored_remote_machines = saved.remote_machines.len();
            // Do not block daemon boot on active terminal launch. Bind the control socket first,
            // then let explicit terminal-open requests or later recovery drive runtime startup.
            server.restore_persisted_state_with_launch_policy(saved, Some(&store), false);
        }
        let store_home_dir_for_ledger = store.home_dir().to_path_buf();
        let mut runtime = Self {
            support,
            state_path,
            store,
            server,
            terminals: TerminalManager::new(),
            preserved_terminal_owners,
            row_order_ledger: crate::row_order_ledger::RowOrderLedger::load(
                store_home_dir_for_ledger.as_path(),
            ),
            remote_machine_refreshes_in_flight: HashSet::new(),
            codex_process_identity_cache: Mutex::new(BTreeMap::new()),
            restored_from_persisted_state,
            restored_stored_sessions,
            restored_live_sessions,
            restored_remote_machines,
            preserved_owner_unreachable_until_ms: HashMap::new(),
            terminal_write_relaunch_attempted_at_ms: HashMap::new(),
            update_restart_state_written: false,
            duplicate_runtime_prune_in_flight: Arc::new(AtomicBool::new(false)),
            pending_preserved_owner_removals: Arc::new(Mutex::new(Vec::new())),
            superseded_routine_persist_muted: false,
            pending_remote_pty_resizes: Arc::new(Mutex::new(HashMap::new())),
            remote_pty_resize_in_flight: Arc::new(Mutex::new(HashSet::new())),
        };
        let preserved_owner_registry_retargeted = runtime
            .preserved_terminal_owners
            .retarget_loaded_registry_for_current_version(runtime.store.home_dir())?;
        if preserved_owner_registry_retargeted {
            append_trace_event(
                runtime.store.home_dir(),
                "daemon",
                "hot_update",
                "preserved_owner_registry_retargeted_on_load",
                serde_json::json!({
                    "expected_server_version": SERVER_PROTOCOL_VERSION,
                    "remaining_runtime_keys": runtime.preserved_terminal_owners.keys(),
                    "reason": "runtime_load",
                }),
            );
        }
        append_trace_event(
            runtime.store.home_dir(),
            "daemon",
            "hot_update",
            "preserved_owner_deep_reconcile_deferred_on_load",
            serde_json::json!({
                "reason": "current daemon socket must bind before cross-daemon preserved-owner inspection",
                "remaining_runtime_keys": runtime.preserved_terminal_owners.keys(),
            }),
        );
        perf.finish(serde_json::json!({
            "prefer_ghostty_backend": settings.prefer_ghostty_backend,
            "theme": format!("{:?}", settings.theme),
        }));
        Ok(runtime)
    }

    fn preserved_terminal_owner_keys(&self) -> Vec<String> {
        self.preserved_terminal_owners
            .keys()
            .into_iter()
            .filter(|key| self.server.represents_terminal_runtime_key(key))
            .filter(|key| !self.terminals.has_session(key))
            .collect()
    }

    fn terminal_runtime_keys_including_preserved(&self) -> Vec<String> {
        let mut keys = self.terminals.session_keys();
        keys.extend(self.preserved_terminal_owner_keys());
        keys.sort();
        keys.dedup();
        keys
    }

    /// Lightweight per-session working verdicts for the GUI's fast dot poll
    /// (the working-dot lag fix): EXACTLY the same SSOT the snapshot path
    /// assigns to `session.working` — agent kinds scrape the live vt100 footer,
    /// shells use the OS foreground-process fact. Sessions with no definite
    /// verdict (`None`: no live screen / not owned) are simply absent, so the
    /// GUI can never blink a frozen last frame. Orders of magnitude cheaper
    /// than a full snapshot, so the GUI may poll it even while a focused
    /// terminal defers background refreshes.
    fn working_flags(&self) -> Vec<(String, bool)> {
        self.server
            .live_sessions()
            .iter()
            .filter_map(|session| {
                let runtime_path = self.terminal_runtime_key_for_path(&session.session_path);
                let working = match session.kind {
                    SessionKind::Codex | SessionKind::CodexLiteLlm | SessionKind::ClaudeCode => {
                        self.terminals
                            .session_screen_snapshot(&runtime_path)
                            .as_deref()
                            .map(yggterm_core::screen_text_shows_agent_working)
                    }
                    SessionKind::Shell => self
                        .terminals
                        .session_foreground_process_active(&runtime_path),
                    _ => None,
                }?;
                Some((session.session_path.clone(), working))
            })
            .collect()
    }

    fn status(&self) -> ServerRuntimeStatus {
        let terminal_stats = self.terminals.stats();
        let payload_stats = self.server.payload_stats();
        let preserved_owner_keys = self.preserved_terminal_owner_keys();
        let owned_terminal_session_keys = self.terminals.session_keys();
        let mut terminal_session_keys = owned_terminal_session_keys.clone();
        terminal_session_keys.extend(preserved_owner_keys.iter().cloned());
        terminal_session_keys.sort();
        terminal_session_keys.dedup();
        // The SAME predicate the cold-retire loop consults, reported verbatim — so what
        // the user reads is the reason the daemon is actually acting on, not a parallel
        // explanation that can drift from it.
        let hot_restart_blockers =
            self.hot_update_idle_gate_blockers(&owned_terminal_session_keys);
        let hot_restart_block_reason = hot_restart_block_reason_summary(&hot_restart_blockers);
        let running_build_id = *DAEMON_RUNNING_BUILD_ID;
        let on_disk_build_id = current_build_id();
        let started_at_ms = *DAEMON_STARTED_AT_MS;
        ServerRuntimeStatus {
            server_version: SERVER_PROTOCOL_VERSION.to_string(),
            server_build_id: current_build_id(),
            server_pid: std::process::id(),
            host_kind: self.support.kind.as_str().to_string(),
            host_detail: self.support.detail.clone(),
            embedded_surface_supported: self.support.embedded_surface_supported,
            bridge_enabled: self.support.bridge_enabled,
            restored_from_persisted_state: self.restored_from_persisted_state,
            restored_stored_sessions: self.restored_stored_sessions,
            restored_live_sessions: self.restored_live_sessions,
            restored_remote_machines: self.restored_remote_machines,
            owned_terminal_session_count: owned_terminal_session_keys.len(),
            owned_terminal_session_keys,
            terminal_session_count: terminal_session_keys.len(),
            terminal_session_keys,
            preserved_terminal_owner_count: preserved_owner_keys.len(),
            preserved_terminal_owner_keys: preserved_owner_keys,
            terminal_retained_chunks: terminal_stats.retained_chunks,
            terminal_retained_bytes: terminal_stats.retained_bytes,
            terminal_session_buffer_limit_bytes: crate::terminal::MAX_BUFFER_BYTES,
            terminal_idle_buffer_limit_bytes: crate::terminal::IDLE_TRIM_MAX_BYTES,
            managed_session_count: payload_stats.session_count,
            session_metadata_entries: payload_stats.metadata_entries,
            session_metadata_bytes: payload_stats.metadata_bytes,
            session_preview_block_count: payload_stats.preview_blocks,
            session_preview_line_count: payload_stats.preview_lines,
            session_preview_bytes: payload_stats.preview_bytes,
            session_rendered_section_count: payload_stats.rendered_sections,
            session_rendered_line_count: payload_stats.rendered_lines,
            session_rendered_bytes: payload_stats.rendered_bytes,
            session_terminal_line_count: payload_stats.terminal_lines,
            session_terminal_bytes: payload_stats.terminal_bytes,
            session_payload_total_bytes: payload_stats.total_bytes,
            daemon_started_at_ms: started_at_ms,
            daemon_uptime_ms: current_millis_u64().saturating_sub(started_at_ms),
            running_build_id,
            on_disk_build_id,
            // A build id of 0 means the exe mtime was unreadable; "pending" would then be
            // a coin flip, so report no pending update rather than a confident wrong one.
            hot_restart_pending: running_build_id != 0
                && on_disk_build_id != 0
                && running_build_id != on_disk_build_id,
            hot_restart_block_reason,
            hot_restart_blockers,
        }
    }

    /// Idle gate for hot-update (#19): every owned session currently DEFERRING a
    /// hot-restart, in the daemon's own words.
    ///
    /// Blocks when a session is working (`esc to interrupt`, via the shared
    /// [`yggterm_core::screen_text_shows_agent_working`] SSOT) or produced output more
    /// recently than the idle threshold. Fail-safe: unknown/unreadable sessions do not
    /// block. Overridable via `YGGTERM_HOT_UPDATE_IGNORE_IDLE_GATE`.
    ///
    /// Returns ALL blockers, not just the first. Reporting only the first made the
    /// deferral opaque: the user clears the named session, the restart still defers, and
    /// the panel names a DIFFERENT session — so a swap pinned by three agents read as an
    /// endless unexplained wait. On jojo (2026-07-11) that opacity let a 2.10.3 daemon run
    /// for 19h44m with 2.10.13 on disk. See [[finding-stale-daemon-trap]].
    fn hot_update_idle_gate_blockers(&self, owned_runtime_keys: &[String]) -> Vec<HotRestartBlocker> {
        if hot_update_idle_gate_overridden() {
            return Vec::new();
        }
        let threshold_ms = hot_update_idle_threshold_ms();
        let mut blockers = Vec::new();
        for key in owned_runtime_keys {
            let runtime_path = self.terminal_runtime_key_for_path(key);
            if let Some(screen) = self.terminals.session_screen_snapshot(&runtime_path)
                && yggterm_core::screen_text_shows_agent_working(&screen)
            {
                blockers.push(HotRestartBlocker {
                    session_key: key.clone(),
                    kind: HOT_RESTART_BLOCKER_WORKING.to_string(),
                    idle_ms: self.terminals.session_idle_for_ms(&runtime_path),
                    threshold_ms,
                });
                continue;
            }
            if let Some(idle_ms) = self.terminals.session_idle_for_ms(&runtime_path)
                && idle_ms < threshold_ms
            {
                blockers.push(HotRestartBlocker {
                    session_key: key.clone(),
                    kind: HOT_RESTART_BLOCKER_RECENTLY_ACTIVE.to_string(),
                    idle_ms: Some(idle_ms),
                    threshold_ms,
                });
            }
        }
        blockers
    }

    /// The same predicate the retire loop acts on, collapsed to one line for the panel.
    fn hot_update_idle_gate_block_reason(&self, owned_runtime_keys: &[String]) -> Option<String> {
        let blockers = self.hot_update_idle_gate_blockers(owned_runtime_keys);
        hot_restart_block_reason_summary(&blockers)
    }

    /// Gather the live OS migration signals for a runtime key we own. Reads the
    /// PTY owner's own facts (activity idle, sticky draft, foreground pgrp) plus
    /// the optional screen-footer guard. A key we don't own yields all-`None`
    /// signals → never migratable (the safety bias lives in the pure predicate).
    fn session_migratable_signals(&self, runtime_key: &str) -> MigratableSignals {
        MigratableSignals {
            activity_idle_ms: self.terminals.session_idle_for_ms(runtime_key),
            has_pending_draft: self.terminals.session_has_pending_input_draft(runtime_key),
            foreground_command_running: self
                .terminals
                .session_foreground_process_active(runtime_key),
            screen_shows_working: self
                .terminals
                .session_screen_snapshot(runtime_key)
                .as_deref()
                .map(yggterm_core::screen_text_shows_agent_working)
                .unwrap_or(false),
        }
    }

    /// `true` when this daemon may safely release `runtime_key` for the newest
    /// daemon to re-resume and own (progressive migration). Only ever called on
    /// runtimes THIS daemon owns; see `session_is_migratable`.
    fn session_is_migratable_now(&self, runtime_key: &str) -> bool {
        session_is_migratable(
            &self.session_migratable_signals(runtime_key),
            migration_idle_threshold_ms(),
        )
    }

    /// Release an owned agent session so the newest daemon re-resumes and owns
    /// it (progressive migration, mechanism (b)). We KILL our PTY runtime but
    /// keep the logical live session intact — the successor already holds the
    /// session record (it restored it as a preserved-owner bridge) and, once it
    /// sees this daemon no longer reports the runtime, re-resumes it on its own
    /// PTY via the existing owner-unreachable recovery / reveal path. Our own
    /// routine persists are muted while superseded, so dropping the runtime here
    /// can never clobber the successor's authoritative server-state.
    fn release_session_for_migration(&mut self, home_dir: &Path, runtime_key: &str) {
        let idle_ms = self.terminals.session_idle_for_ms(runtime_key);
        let kind = self.server.live_session_kind(runtime_key);
        let removed = match self.terminals.remove_session(runtime_key, None) {
            Ok(removed) => removed,
            Err(error) => {
                append_trace_event(
                    home_dir,
                    "daemon",
                    "lifecycle",
                    "progressive_migration_release_failed",
                    serde_json::json!({
                        "runtime_key": runtime_key,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        append_trace_event(
            home_dir,
            "daemon",
            "lifecycle",
            "progressive_migration_session_released",
            serde_json::json!({
                "runtime_key": runtime_key,
                "kind": kind.map(crate::session_kind_label),
                "idle_ms": idle_ms,
                "runtime_removed": removed,
                "current_pid": std::process::id(),
            }),
        );
    }

    fn overlay_terminal_runtime_snapshot_session(&self, session: &mut SnapshotSessionView) {
        let runtime_path = self.terminal_runtime_key_for_path(&session.session_path);
        session.terminal_process_id = self.terminals.session_process_id(&runtime_path);
        session.terminal_foreground_active = self
            .terminals
            .session_foreground_process_active(&runtime_path);
        // Observability (grid-squish): the PTY grid the running program sees, so a
        // squish (PTY grid < client xterm grid) is directly measurable in `server snapshot`.
        if let Some((cols, rows)) = self.terminals.session_size(&runtime_path) {
            session.pty_cols = Some(cols);
            session.pty_rows = Some(rows);
        }
        if matches!(
            session.kind,
            SessionKind::Shell
                | SessionKind::Codex
                | SessionKind::CodexLiteLlm
                | SessionKind::ClaudeCode
        ) {
            let screen_text = self.terminals.session_screen_snapshot(&runtime_path);
            // Daemon-authoritative working state (SSOT for the sidebar dot + the
            // working→done notification): for agent CLIs, derive it from the
            // LIVE screen via the shared esc-to-interrupt detector. `Some(true)`
            // = working now, `Some(false)` = confirmed idle, `None` = no live
            // screen (preserved/foreign-owned) so the GUI must NOT blink it.
            // This is what stops a session from blinking forever after its turn
            // ended but its last-captured frame froze on the working footer.
            if matches!(
                session.kind,
                SessionKind::Codex | SessionKind::CodexLiteLlm | SessionKind::ClaudeCode
            ) {
                session.working = screen_text
                    .as_deref()
                    .map(yggterm_core::screen_text_shows_agent_working);
            } else if session.kind == SessionKind::Shell {
                // Issue #1 ("shell always working"): a plain shell has no agent
                // footer to scrape — its working state is the OS fact "a
                // foreground command is running in the tty" (foreground pgrp !=
                // session leader). `Some(true)` while a command runs (incl.
                // silent ones), `Some(false)` at a bare prompt, `None` when not
                // owned (so the GUI must NOT blink a bridged/foreign shell).
                session.working = self.terminals.session_foreground_process_active(&runtime_path);
            }
            if let Some(screen_text) = screen_text.as_deref()
                && let Some((status_line, terminal_lines)) =
                    terminal_sidebar_snapshot_from_screen(screen_text)
            {
                // ClaudeCode must be included: the sidebar working-indicator detects
                // CC's "esc to interrupt" status, but it can only do so if CC's live
                // screen text is refreshed here. Omitting it left CC sessions stuck
                // showing idle. See memory finding-hot-update-interrupts-remote-sessions (#21).
                session.status_line = status_line;
                session.terminal_lines = terminal_lines;
            }
        }
    }

    fn overlay_codex_runtime_snapshot_session(
        &self,
        session: &mut SnapshotSessionView,
        yggterm_home: Option<&Path>,
    ) {
        if session.kind != SessionKind::Codex {
            return;
        }
        let Some(pid) = session.terminal_process_id else {
            return;
        };
        let identity = self
            .codex_process_identity_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&pid).cloned())
            .or_else(|| {
                let identity = codex_runtime_process_identity_from_root_pid(pid)?;
                if let Ok(mut cache) = self.codex_process_identity_cache.lock() {
                    cache.insert(pid, identity.clone());
                }
                Some(identity)
            });
        if let Some(identity) = identity {
            overlay_codex_runtime_snapshot_identity(session, &identity, yggterm_home);
        }
    }

    fn codex_runtime_identity_for_pid(&self, pid: u32) -> Option<CodexRuntimeProcessIdentity> {
        self.codex_process_identity_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&pid).cloned())
            .or_else(|| {
                let identity = codex_runtime_process_identity_from_root_pid(pid)?;
                if let Ok(mut cache) = self.codex_process_identity_cache.lock() {
                    cache.insert(pid, identity.clone());
                }
                Some(identity)
            })
    }

    fn refresh_live_codex_runtime_identities_for_persistence(&mut self) -> usize {
        let yggterm_home = resolve_yggterm_home().ok();
        let keys = self.server.live_codex_session_keys_for_runtime_identity();
        let mut refreshed = 0usize;
        for key in keys {
            let runtime_path = self.server.terminal_runtime_key_for_path(&key);
            let Some(pid) = self.terminals.session_process_id(&runtime_path) else {
                continue;
            };
            let Some(identity) = self.codex_runtime_identity_for_pid(pid) else {
                continue;
            };
            if self.server.apply_codex_runtime_identity_to_live_session(
                &key,
                &identity,
                yggterm_home.as_deref(),
            ) {
                refreshed += 1;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "persistence",
                    "codex_runtime_identity_refreshed",
                    serde_json::json!({
                        "session_path": key,
                        "runtime_path": runtime_path,
                        "pid": pid,
                        "codex_session_id": identity.session_id,
                        "storage_path": identity.storage_path.display().to_string(),
                    }),
                );
            }
        }
        refreshed
    }

    /// Mirrors `refresh_live_codex_runtime_identities_for_persistence` for
    /// Claude Code. Walks each live ClaudeCode session's PTY process tree
    /// to find the open `~/.claude/projects/.../<session-id>.jsonl` and
    /// applies the discovered identity to the live row. Closes the same
    /// UUIDv4-vs-real-id drift that the codex rebind handles.
    fn refresh_live_claude_code_runtime_identities_for_persistence(&mut self) -> usize {
        let keys = self.server.live_claude_code_session_keys_for_runtime_identity();
        let mut refreshed = 0usize;
        for key in keys {
            let runtime_path = self.server.terminal_runtime_key_for_path(&key);
            let Some(pid) = self.terminals.session_process_id(&runtime_path) else {
                continue;
            };
            let Some(identity) = claude_code_runtime_process_identity_from_root_pid(pid) else {
                continue;
            };
            if self
                .server
                .apply_claude_code_runtime_identity_to_live_session(&key, &identity)
            {
                refreshed += 1;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "persistence",
                    "claude_code_runtime_identity_refreshed",
                    serde_json::json!({
                        "session_path": key,
                        "runtime_path": runtime_path,
                        "pid": pid,
                        "claude_code_session_id": identity.session_id,
                        "storage_path": identity.storage_path.display().to_string(),
                    }),
                );
            }
        }
        refreshed
    }

    fn snapshot_response(&self, message: Option<String>) -> ServerResponse {
        let _perf = yggterm_core::PerfGuard::new(self.store.home_dir(), "daemon", "snapshot_response");
        let mut snapshot = self.server.snapshot();
        let runtime_keys = self
            .terminal_runtime_keys_including_preserved()
            .into_iter()
            .collect();
        apply_terminal_runtime_truth_to_snapshot(&self.server, &runtime_keys, &mut snapshot);
        let yggterm_home = resolve_yggterm_home().ok();
        if let Some(active_session) = snapshot.active_session.as_mut() {
            self.overlay_terminal_runtime_snapshot_session(active_session);
            self.overlay_codex_runtime_snapshot_session(active_session, yggterm_home.as_deref());
        }
        for session in &mut snapshot.live_sessions {
            self.overlay_terminal_runtime_snapshot_session(session);
            self.overlay_codex_runtime_snapshot_session(session, yggterm_home.as_deref());
        }
        ServerResponse::Snapshot { snapshot, message }
    }

    fn terminal_runtime_key_for_path(&self, path: &str) -> String {
        self.server.terminal_runtime_key_for_path(path)
    }

    fn preserved_owner_for_runtime_key(&self, runtime_key: &str) -> Option<ServerEndpoint> {
        if self.terminals.has_session(runtime_key) {
            return None;
        }
        if !self.server.represents_terminal_runtime_key(runtime_key) {
            return None;
        }
        let entry = self.preserved_terminal_owners.owner_for_key(runtime_key)?;
        let endpoint = entry.endpoint.to_endpoint();
        if server_endpoints_same_target(&endpoint, &default_endpoint(self.store.home_dir())) {
            return None;
        }
        // Negative cache: a recently-timed-out owner socket is not re-probed
        // (each probe costs the full request timeout while holding the loop).
        let label = owner_endpoint_label(&endpoint);
        if let Some(until) = self.preserved_owner_unreachable_until_ms.get(&label)
            && (crate::app_control::current_millis() as u64) < *until
        {
            return None;
        }
        Some(endpoint)
    }

    fn mark_preserved_owner_unreachable(&mut self, owner_endpoint: &ServerEndpoint) {
        self.preserved_owner_unreachable_until_ms.insert(
            owner_endpoint_label(owner_endpoint),
            crate::app_control::current_millis() as u64 + PRESERVED_OWNER_UNREACHABLE_CACHE_MS,
        );
    }

    fn remove_preserved_owner(&mut self, runtime_key: &str, reason: &'static str) {
        if !self.preserved_terminal_owners.remove_key(runtime_key) {
            return;
        }
        let _ = self.preserved_terminal_owners.save(self.store.home_dir());
        append_trace_event(
            self.store.home_dir(),
            "daemon",
            "hot_update",
            "preserved_owner_removed",
            serde_json::json!({
                "runtime_key": runtime_key,
                "reason": reason,
            }),
        );
    }

    fn remove_preserved_owner_runtime(&mut self, runtime_key: &str, reason: &'static str) {
        let current_endpoint = default_endpoint(self.store.home_dir());
        let owner_endpoint = self
            .preserved_terminal_owners
            .owner_for_key(runtime_key)
            .map(|entry| entry.endpoint.to_endpoint())
            .filter(|endpoint| !server_endpoints_same_target(endpoint, &current_endpoint));
        if let Some(owner_endpoint) = owner_endpoint {
            match remove_session(&owner_endpoint, runtime_key) {
                Ok((_snapshot, message)) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_runtime_removed",
                        serde_json::json!({
                            "reason": reason,
                            "runtime_key": runtime_key,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "message": message,
                        }),
                    );
                }
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_runtime_remove_failed",
                        serde_json::json!({
                            "reason": reason,
                            "runtime_key": runtime_key,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
        self.remove_preserved_owner(runtime_key, reason);
    }

    fn restore_missing_preserved_owner_live_sessions(&mut self, reason: &'static str) {
        let endpoint_groups = self.preserved_terminal_owners.endpoint_groups();
        if endpoint_groups.is_empty() {
            return;
        }
        let current_endpoint = default_endpoint(self.store.home_dir());
        for (owner_endpoint, owner_registry_keys) in endpoint_groups {
            if server_endpoints_same_target(&owner_endpoint, &current_endpoint) {
                continue;
            }
            let missing_keys = owner_registry_keys
                .iter()
                .filter(|key| !self.server.represents_terminal_runtime_key(key))
                .cloned()
                .collect::<HashSet<_>>();
            if missing_keys.is_empty() {
                continue;
            }
            let owner_snapshot = match snapshot(&owner_endpoint) {
                Ok((snapshot, _message)) => snapshot,
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_live_session_restore_snapshot_failed",
                        serde_json::json!({
                            "reason": reason,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "runtime_keys": owner_registry_keys,
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
            };
            let restored_runtime_keys = restore_preserved_owner_live_sessions_from_snapshot(
                &mut self.server,
                &owner_snapshot,
                &missing_keys,
            );
            if restored_runtime_keys.is_empty() {
                continue;
            }
            let owner_active_path = owner_snapshot.active_session_path.as_deref();
            let active_restored = owner_active_path
                .filter(|path| restored_runtime_keys.iter().any(|key| key == *path))
                .is_some_and(|path| {
                    self.server
                        .focus_live_session_without_launch_if_active_missing(path)
                });
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "hot_update",
                "preserved_owner_live_sessions_restored",
                serde_json::json!({
                    "reason": reason,
                    "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                    "owner_active_path": owner_snapshot.active_session_path,
                    "active_restored": active_restored,
                    "missing_runtime_keys": missing_keys,
                    "restored_runtime_keys": restored_runtime_keys,
                }),
            );
        }
    }

    fn recover_missing_preserved_owner_live_sessions_from_reachable_daemons(
        &mut self,
        reason: &'static str,
    ) {
        let current_endpoint = default_endpoint(self.store.home_dir());
        let mut registered_keys = self
            .preserved_terminal_owners
            .keys()
            .into_iter()
            .collect::<HashSet<_>>();
        for (owner_endpoint, owner_status) in reachable_versioned_daemon_statuses_excluding_endpoint(
            self.store.home_dir(),
            &current_endpoint,
        ) {
            let direct_keys = runtime_status_direct_terminal_keys(&owner_status);
            let represented_missing_owner_keys = direct_keys
                .iter()
                .filter(|key| !registered_keys.contains(*key))
                .filter(|key| {
                    represented_live_session_needs_preserved_owner(
                        &self.server,
                        &self.terminals,
                        key,
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut represented_registered_runtime_keys = Vec::new();
            let mut represented_registration_errors = Vec::new();
            for runtime_key in &represented_missing_owner_keys {
                match self.preserved_terminal_owners.upsert_runtime_owner(
                    self.store.home_dir(),
                    runtime_key,
                    &owner_endpoint,
                    &owner_status,
                    Some(SERVER_PROTOCOL_VERSION.to_string()),
                ) {
                    Ok(_) => {
                        represented_registered_runtime_keys.push(runtime_key.clone());
                        registered_keys.insert(runtime_key.clone());
                    }
                    Err(error) => represented_registration_errors.push(serde_json::json!({
                        "runtime_key": runtime_key,
                        "error": error.to_string(),
                    })),
                }
            }
            if !represented_registered_runtime_keys.is_empty()
                || !represented_registration_errors.is_empty()
            {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "preserved_owner_represented_live_sessions_registered",
                    serde_json::json!({
                        "reason": reason,
                        "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                        "owner_server_version": owner_status.server_version,
                        "owner_server_pid": owner_status.server_pid,
                        "candidate_runtime_keys": represented_missing_owner_keys,
                        "registered_runtime_keys": represented_registered_runtime_keys,
                        "registration_errors": represented_registration_errors,
                    }),
                );
            }

            let missing_keys = direct_keys
                .into_iter()
                .filter(|key| !registered_keys.contains(key))
                .filter(|key| !self.terminals.has_session(key))
                .filter(|key| !self.server.represents_terminal_runtime_key(key))
                .collect::<HashSet<_>>();
            if missing_keys.is_empty() {
                continue;
            }
            let owner_snapshot = match snapshot(&owner_endpoint) {
                Ok((snapshot, _message)) => snapshot,
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_recovery_snapshot_failed",
                        serde_json::json!({
                            "reason": reason,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "owner_server_version": owner_status.server_version,
                            "owner_server_pid": owner_status.server_pid,
                            "runtime_keys": missing_keys,
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
            };
            let restored_runtime_keys =
                restore_preserved_owner_live_sessions_from_snapshot_with_policy(
                    &mut self.server,
                    &owner_snapshot,
                    &missing_keys,
                    false,
                );
            if restored_runtime_keys.is_empty() {
                continue;
            }
            let mut registered_runtime_keys = Vec::new();
            let mut registration_errors = Vec::new();
            for runtime_key in &restored_runtime_keys {
                match self.preserved_terminal_owners.upsert_runtime_owner(
                    self.store.home_dir(),
                    runtime_key,
                    &owner_endpoint,
                    &owner_status,
                    Some(SERVER_PROTOCOL_VERSION.to_string()),
                ) {
                    Ok(_) => registered_runtime_keys.push(runtime_key.clone()),
                    Err(error) => registration_errors.push(serde_json::json!({
                        "runtime_key": runtime_key,
                        "error": error.to_string(),
                    })),
                }
            }
            let owner_active_path = owner_snapshot.active_session_path.as_deref();
            let active_restored = owner_active_path
                .filter(|path| restored_runtime_keys.iter().any(|key| key == *path))
                .is_some_and(|path| {
                    self.server
                        .focus_live_session_without_launch_if_active_missing(path)
                });
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "hot_update",
                "preserved_owner_live_sessions_recovered_from_reachable_daemon",
                serde_json::json!({
                    "reason": reason,
                    "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                    "owner_server_version": owner_status.server_version,
                    "owner_server_pid": owner_status.server_pid,
                    "owner_active_path": owner_snapshot.active_session_path,
                    "active_restored": active_restored,
                    "missing_runtime_keys": missing_keys,
                    "restored_runtime_keys": restored_runtime_keys,
                    "registered_runtime_keys": registered_runtime_keys,
                    "registration_errors": registration_errors,
                }),
            );
        }
    }

    fn preserved_owner_status_cache(&self) -> BTreeMap<String, Option<ServerRuntimeStatus>> {
        let current_endpoint = default_endpoint(self.store.home_dir());
        let mut status_cache = BTreeMap::<String, Option<ServerRuntimeStatus>>::new();
        for entry in &self.preserved_terminal_owners.entries {
            let owner_endpoint = entry.endpoint.to_endpoint();
            if server_endpoints_same_target(&owner_endpoint, &current_endpoint) {
                continue;
            }
            let label = entry.endpoint.label();
            status_cache
                .entry(label)
                .or_insert_with(|| status(&owner_endpoint).ok());
        }
        status_cache
    }

    fn prune_unrepresented_preserved_owners(&mut self, reason: &'static str) {
        let current_endpoint = default_endpoint(self.store.home_dir());
        let mut owner_status_cache = BTreeMap::<String, Option<ServerRuntimeStatus>>::new();
        for (owner_endpoint, _runtime_keys) in self.preserved_terminal_owners.endpoint_groups() {
            if server_endpoints_same_target(&owner_endpoint, &current_endpoint) {
                continue;
            }
            let label = owner_endpoint_label(&owner_endpoint);
            owner_status_cache
                .entry(label)
                .or_insert_with(|| status(&owner_endpoint).ok());
        }
        let preserved_owner_entries = self.preserved_terminal_owners.entries.clone();
        let removed = self
            .preserved_terminal_owners
            .retain_represented_keys(|key| {
                if self.server.represents_terminal_runtime_key(key) {
                    return true;
                }
                let Some(entry) = preserved_owner_entries
                    .iter()
                    .find(|entry| entry.runtime_key == key)
                else {
                    return false;
                };
                let owner_label = entry.endpoint.label();
                owner_status_cache
                    .get(&owner_label)
                    .and_then(|status| status.as_ref())
                    .is_some_and(|status| {
                        status
                            .terminal_session_keys
                            .iter()
                            .any(|candidate| candidate == key)
                    })
            });
        if removed.is_empty() {
            return;
        }
        let _ = self.preserved_terminal_owners.save(self.store.home_dir());
        append_trace_event(
            self.store.home_dir(),
            "daemon",
            "hot_update",
            "preserved_owner_registry_pruned",
            serde_json::json!({
                "reason": reason,
                "removed_runtime_keys": removed,
                "remaining_runtime_keys": self.preserved_terminal_owners.keys(),
            }),
        );
    }

    fn prune_unrepresented_preserved_owner_runtime_sessions(&mut self, reason: &'static str) {
        let endpoint_groups = self.preserved_terminal_owners.endpoint_groups();
        if endpoint_groups.is_empty() {
            return;
        }
        let current_endpoint = default_endpoint(self.store.home_dir());
        for (owner_endpoint, owner_registry_keys) in endpoint_groups {
            if server_endpoints_same_target(&owner_endpoint, &current_endpoint) {
                continue;
            }
            let owner_status = match status(&owner_endpoint) {
                Ok(status) => status,
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_runtime_prune_status_failed",
                        serde_json::json!({
                            "reason": reason,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
            };
            let mut owner_runtime_keys = if owner_status.owned_terminal_session_keys.is_empty() {
                owner_status.terminal_session_keys.clone()
            } else {
                owner_status.owned_terminal_session_keys.clone()
            };
            owner_runtime_keys.sort();
            owner_runtime_keys.dedup();
            let all_registry_keys = self
                .preserved_terminal_owners
                .keys()
                .into_iter()
                .collect::<HashSet<_>>();
            let current_owned_runtime_keys = self
                .terminals
                .session_keys()
                .into_iter()
                .collect::<HashSet<_>>();
            let owner_registry_key_set =
                owner_registry_keys.iter().cloned().collect::<HashSet<_>>();
            let stale_runtime_keys = unrepresented_preserved_owner_runtime_keys(
                &self.server,
                &owner_registry_key_set,
                &all_registry_keys,
                &current_owned_runtime_keys,
                owner_runtime_keys,
            );
            if stale_runtime_keys.is_empty() {
                continue;
            }
            let mut restored_running_runtime_keys = Vec::new();
            let stale_runtime_key_set = stale_runtime_keys.iter().cloned().collect::<HashSet<_>>();
            match snapshot(&owner_endpoint) {
                Ok((owner_snapshot, _message)) => {
                    restored_running_runtime_keys =
                        restore_preserved_owner_live_sessions_from_snapshot_with_policy(
                            &mut self.server,
                            &owner_snapshot,
                            &stale_runtime_key_set,
                            false,
                        );
                    for runtime_key in &restored_running_runtime_keys {
                        if let Err(error) = self.preserved_terminal_owners.upsert_runtime_owner(
                            self.store.home_dir(),
                            runtime_key,
                            &owner_endpoint,
                            &owner_status,
                            Some(SERVER_PROTOCOL_VERSION.to_string()),
                        ) {
                            append_trace_event(
                                self.store.home_dir(),
                                "daemon",
                                "hot_update",
                                "preserved_owner_running_runtime_register_failed",
                                serde_json::json!({
                                    "reason": reason,
                                    "runtime_key": runtime_key,
                                    "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                                    "owner_server_version": owner_status.server_version,
                                    "owner_server_pid": owner_status.server_pid,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                    if !restored_running_runtime_keys.is_empty() {
                        let _ = self.persist();
                        let _ = self.preserved_terminal_owners.save(self.store.home_dir());
                        append_trace_event(
                            self.store.home_dir(),
                            "daemon",
                            "hot_update",
                            "preserved_owner_running_runtimes_recovered_before_prune",
                            serde_json::json!({
                                "reason": reason,
                                "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                                "owner_server_version": owner_status.server_version,
                                "owner_server_pid": owner_status.server_pid,
                                "restored_runtime_keys": &restored_running_runtime_keys,
                            }),
                        );
                    }
                }
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_running_runtime_snapshot_failed",
                        serde_json::json!({
                            "reason": reason,
                            "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                            "owner_server_version": owner_status.server_version,
                            "owner_server_pid": owner_status.server_pid,
                            "candidate_runtime_keys": &stale_runtime_keys,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            let restored_running_runtime_key_set = restored_running_runtime_keys
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            let mut removed_runtime_keys = Vec::new();
            let mut errors = Vec::new();
            for runtime_key in stale_runtime_keys
                .into_iter()
                .filter(|runtime_key| !restored_running_runtime_key_set.contains(runtime_key))
            {
                match drop_terminal_runtime(
                    &owner_endpoint,
                    &runtime_key,
                    Some("unrepresented_preserved_owner_prune"),
                ) {
                    Ok(_message) => removed_runtime_keys.push(runtime_key),
                    Err(error) => {
                        errors.push(serde_json::json!({
                            "runtime_key": runtime_key,
                            "error": error.to_string(),
                        }));
                    }
                }
            }
            let remaining_stale_runtime_keys = match status(&owner_endpoint) {
                Ok(status_after) => {
                    let after_keys = if status_after.owned_terminal_session_keys.is_empty() {
                        status_after.terminal_session_keys
                    } else {
                        status_after.owned_terminal_session_keys
                    };
                    unrepresented_preserved_owner_runtime_keys(
                        &self.server,
                        &owner_registry_key_set,
                        &all_registry_keys,
                        &current_owned_runtime_keys,
                        after_keys,
                    )
                }
                Err(_) => Vec::new(),
            };
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "hot_update",
                "preserved_owner_unrepresented_runtimes_pruned",
                serde_json::json!({
                    "reason": reason,
                    "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                    "owner_server_version": owner_status.server_version,
                    "owner_server_pid": owner_status.server_pid,
                    "restored_running_runtime_keys": restored_running_runtime_key_set
                        .into_iter()
                        .collect::<Vec<_>>(),
                    "removed_runtime_keys": removed_runtime_keys,
                    "errors": errors,
                    "fallback_prepare_client_close": {
                        "attempted": false,
                        "reason": "broad_owner_close_can_kill_unrelated_running_sessions"
                    },
                    "remaining_stale_runtime_keys": remaining_stale_runtime_keys,
                }),
            );
        }
    }

    /// Run #17 split-brain lesson (structural): this prune talks to OTHER
    /// daemons' sockets (status probes, per-key drops, a retire) — against a
    /// live-but-busy duplicate each round-trip can take seconds. It therefore
    /// runs on a BACKGROUND thread; the only daemon-state mutation (the
    /// preserved-owner registry removal) is queued and applied on the request
    /// loop via [`Self::drain_pending_preserved_owner_removals`].
    fn prune_duplicate_legacy_owned_runtime_sessions(&mut self, reason: &'static str) {
        self.drain_pending_preserved_owner_removals();
        let current_runtime_keys = self
            .terminals
            .session_keys()
            .into_iter()
            .collect::<HashSet<_>>();
        if current_runtime_keys.is_empty() {
            return;
        }
        if self
            .duplicate_runtime_prune_in_flight
            .swap(true, Ordering::SeqCst)
        {
            return;
        }
        // Snapshot the registry's owner endpoint per key — the background
        // thread must never touch the registry itself.
        let registry_owner_endpoints: HashMap<String, ServerEndpoint> = self
            .preserved_terminal_owners
            .keys()
            .into_iter()
            .filter_map(|key| {
                let endpoint = self
                    .preserved_terminal_owners
                    .owner_for_key(&key)?
                    .endpoint
                    .to_endpoint();
                Some((key, endpoint))
            })
            .collect();
        let home_dir = self.store.home_dir().to_path_buf();
        let in_flight = Arc::clone(&self.duplicate_runtime_prune_in_flight);
        let pending_removals = Arc::clone(&self.pending_preserved_owner_removals);
        let spawn_result = std::thread::Builder::new()
            .name("yggterm-duplicate-runtime-prune".to_string())
            .spawn(move || {
                run_duplicate_legacy_owned_runtime_prune(
                    &home_dir,
                    reason,
                    &current_runtime_keys,
                    &registry_owner_endpoints,
                    &pending_removals,
                );
                in_flight.store(false, Ordering::SeqCst);
            });
        if spawn_result.is_err() {
            self.duplicate_runtime_prune_in_flight
                .store(false, Ordering::SeqCst);
        }
    }

    /// GATE #8 takeover (persistence saga root): ask every reachable OLDER
    /// live daemon to PrepareUpdateRestart — the predecessor writes its
    /// protected snapshot (carrying its non-keep-alive rows) AND latches its
    /// routine persists off (the proven gate-#3 machinery) — then re-read the
    /// state file and merge-restore every recoverable live row this daemon
    /// does not already hold, then persist the merged view. This removes the
    /// swap-ORDERING dependence: it no longer matters whether the successor
    /// loaded the file before the predecessor wrote its snapshot.
    /// Runs once at startup, under the runtime lock (cross-daemon calls are
    /// bounded by the now-small daemon census; predecessors are idle).
    fn takeover_superseded_daemon_state(&mut self) {
        let current_endpoint = default_endpoint(self.store.home_dir());
        let current_triple = parse_daemon_version_triple(SERVER_PROTOCOL_VERSION);
        let normalize = |key: &str| -> String {
            if let Some(id) = crate::local_runtime_id_from_key(key) {
                format!("id::{id}")
            } else {
                key.to_string()
            }
        };
        let mut existing: HashSet<String> = self
            .server
            .live_session_order
            .iter()
            .map(|key| normalize(key))
            .collect();
        let mut prepared = Vec::new();
        let mut prepare_errors = Vec::new();
        let mut imported_keys = Vec::new();
        for (endpoint, status) in reachable_versioned_daemon_statuses_excluding_endpoint(
            self.store.home_dir(),
            &current_endpoint,
        ) {
            if status.server_pid == std::process::id() {
                continue;
            }
            let older = match (
                parse_daemon_version_triple(&status.server_version),
                current_triple,
            ) {
                (Some(theirs), Some(ours)) => theirs < ours,
                _ => false,
            };
            if !older {
                continue;
            }
            match prepare_update_restart(&endpoint) {
                Ok(_) => prepared.push(owner_endpoint_label(&endpoint)),
                Err(error) => {
                    prepare_errors.push(serde_json::json!({
                        "endpoint": owner_endpoint_label(&endpoint),
                        "error": error.to_string(),
                    }));
                    continue;
                }
            }
            // Merge after EACH prepare: every PrepareUpdateRestart rewrites
            // the shared state file wholesale, so reading only after the last
            // one would lose rows unique to an intermediate daemon.
            let saved = match load_persisted_state(&self.state_path) {
                Ok(Some(saved)) => saved,
                _ => continue,
            };
            // Order-preserving import: walk the source daemon's live sessions
            // IN ITS ORDER, tracking the last source key that already exists
            // here (the anchor). Each import is repositioned to sit right
            // after its anchor instead of at the end — appending left every
            // non-keep-alive session (absent from the normal persist, so only
            // importable here) piled at the bottom after each daemon swap:
            // the "rows in weird places after restart" bug.
            let mut order_lookup: HashMap<String, String> = self
                .server
                .live_session_order
                .iter()
                .map(|key| (normalize(key), key.clone()))
                .collect();
            let mut anchor: Option<String> = None;
            for live in saved.live_sessions {
                let normalized = normalize(&live.key);
                if existing.contains(&normalized) {
                    if let Some(current) = order_lookup.get(&normalized) {
                        anchor = Some(current.clone());
                    }
                    continue;
                }
                if !crate::persisted_live_session_is_recoverable(&live) {
                    continue;
                }
                let key = live.key.clone();
                self.server.restore_live_session(live);
                self.server.move_live_session_after(&key, anchor.as_deref());
                existing.insert(normalized.clone());
                order_lookup.insert(normalized, key.clone());
                anchor = Some(key.clone());
                imported_keys.push(key);
            }
        }
        if prepared.is_empty() && prepare_errors.is_empty() {
            return;
        }
        if !imported_keys.is_empty() {
            let _ = self.persist();
        }
        append_trace_event(
            self.store.home_dir(),
            "daemon",
            "lifecycle",
            "superseded_daemon_takeover",
            serde_json::json!({
                "prepared": prepared,
                "prepare_errors": prepare_errors,
                "imported_keys": imported_keys,
            }),
        );
    }

    /// Run #19 permanent squish fix: pin the REMOTE daemon's PTY to the new
    /// grid whenever the local daemon resizes a remote session's attachment.
    /// The implicit chain (local PTY SIGWINCH → ssh window-change → remote tty
    /// → bridge size poll → remote daemon resize) has silent failure points —
    /// live-caught with the remote codex PTY stuck at DEFAULT 120×36 under a
    /// 159×63 client. Latest-wins per session, one in-flight SSH per session,
    /// always off the request loop. No-op for non-remote paths.
    fn forward_remote_pty_resize(&mut self, path: &str, cols: u16, rows: u16) {
        // Kill switch (size-war lesson: every resize writer needs one).
        if std::env::var("YGGTERM_DISABLE_REMOTE_PTY_RESIZE_FORWARD")
            .map(|value| !value.trim().is_empty() && value.trim() != "0")
            .unwrap_or(false)
        {
            return;
        }
        if cols == 0 || rows == 0 {
            return;
        }
        // BOTH remote agent kinds, not just codex. This resolver used to be
        // `remote_shutdown_target_for_path`, which parses `remote-session://`
        // only — so every `remote-cc://` session returned None here and its
        // remote PTY was never resized for its whole life.
        let Some((machine, session_id, kind)) = self.server.remote_agent_pty_target_for_path(path)
        else {
            return;
        };
        {
            let mut pending = self
                .pending_remote_pty_resizes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.insert(path.to_string(), (machine, session_id, kind, cols, rows));
        }
        {
            let mut in_flight = self
                .remote_pty_resize_in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !in_flight.insert(path.to_string()) {
                // A worker is already draining this session; it will pick up
                // the latest pending grid.
                return;
            }
        }
        let pending = Arc::clone(&self.pending_remote_pty_resizes);
        let in_flight = Arc::clone(&self.remote_pty_resize_in_flight);
        let home = self.store.home_dir().to_path_buf();
        let worker_path = path.to_string();
        let spawn_result = std::thread::Builder::new()
            .name("yggterm-remote-pty-resize".to_string())
            .spawn(move || {
                let path = worker_path;
                loop {
                    let next = {
                        let mut pending = pending
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        pending.remove(&path)
                    };
                    let Some((machine, session_id, kind, cols, rows)) = next else {
                        let mut in_flight = in_flight
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        in_flight.remove(&path);
                        // Close the drain/release race: a resize that landed
                        // between the empty check and the release re-claims.
                        let has_pending = {
                            let pending = pending
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            pending.contains_key(&path)
                        };
                        if has_pending && in_flight.insert(path.clone()) {
                            continue;
                        }
                        break;
                    };
                    let result = crate::resize_remote_agent_session_pty(
                        &machine,
                        &session_id,
                        kind,
                        cols,
                        rows,
                    );
                    append_trace_event(
                        &home,
                        "daemon",
                        "terminal_resize",
                        "remote_pty_resize_forwarded",
                        serde_json::json!({
                            "path": path,
                            "kind": kind,
                            "cols": cols,
                            "rows": rows,
                            "ok": result.is_ok(),
                            "error": result.as_ref().err().map(|error| error.to_string()),
                        }),
                    );
                    // A remote PTY stuck at the wrong grid is INVISIBLE to every
                    // daemon-side instrument (they read the local vt100 mirror,
                    // which already holds the client's grid), so the user sees a
                    // TUI painting for a screen that no longer exists while the
                    // telemetry reports a healthy session. Make the failure loud.
                    if let Err(error) = result {
                        append_trace_event(
                            &home,
                            "daemon",
                            "terminal_resize",
                            "remote_pty_resize_failed",
                            serde_json::json!({
                                "path": path,
                                "kind": kind,
                                "cols": cols,
                                "rows": rows,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
            });
        if spawn_result.is_err() {
            let mut in_flight = self
                .remote_pty_resize_in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            in_flight.remove(path);
        }
    }

    /// Apply preserved-owner registry removals discovered by the background
    /// duplicate-runtime prune. Runs on the request loop (registry owner).
    fn drain_pending_preserved_owner_removals(&mut self) {
        let drained = {
            let mut pending = self
                .pending_preserved_owner_removals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *pending)
        };
        for (runtime_key, reason) in drained {
            self.remove_preserved_owner(&runtime_key, reason);
        }
    }
    fn preserved_owner_status_for_runtime_key(
        &mut self,
        runtime_key: &str,
    ) -> Option<(ServerEndpoint, ServerRuntimeStatus)> {
        let endpoint = self.preserved_owner_for_runtime_key(runtime_key)?;
        match status(&endpoint) {
            Ok(status) if preserved_owner_status_serves_runtime_key(&status, runtime_key) => {
                Some((endpoint, status))
            }
            Ok(status) => {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "preserved_owner_missing_runtime",
                    serde_json::json!({
                        "runtime_key": runtime_key,
                        "owner_endpoint": format!("{endpoint:?}"),
                        "owner_version": status.server_version,
                        "owner_pid": status.server_pid,
                    }),
                );
                self.remove_preserved_owner(runtime_key, "owner_missing_runtime");
                None
            }
            Err(error) => {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "preserved_owner_unreachable",
                    serde_json::json!({
                        "runtime_key": runtime_key,
                        "owner_endpoint": format!("{endpoint:?}"),
                        "error": error.to_string(),
                    }),
                );
                self.remove_preserved_owner(runtime_key, "owner_unreachable");
                None
            }
        }
    }

    fn discover_keep_alive_preserved_owner_for_runtime_key(
        &mut self,
        path: &str,
        runtime_key: &str,
    ) -> Option<(ServerEndpoint, ServerRuntimeStatus)> {
        if !path.starts_with("remote-session://")
            || !self.server.live_session_keep_alive(path)
            || self.terminals.has_session(runtime_key)
            || !self.server.represents_terminal_runtime_key(runtime_key)
        {
            return None;
        }
        let current_endpoint = default_endpoint(self.store.home_dir());
        let statuses = reachable_versioned_daemon_statuses_excluding_endpoint(
            self.store.home_dir(),
            &current_endpoint,
        );
        let (owner_endpoint, owner_status) =
            preserved_owner_candidate_for_runtime_key(statuses, runtime_key, std::process::id())?;
        match self.preserved_terminal_owners.upsert_runtime_owner(
            self.store.home_dir(),
            runtime_key,
            &owner_endpoint,
            &owner_status,
            Some(SERVER_PROTOCOL_VERSION.to_string()),
        ) {
            Ok(changed) => {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "keep_alive_preserved_owner_discovered",
                    serde_json::json!({
                        "path": path,
                        "runtime_key": runtime_key,
                        "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                        "owner_server_version": owner_status.server_version,
                        "owner_server_build_id": owner_status.server_build_id,
                        "owner_server_pid": owner_status.server_pid,
                        "registry_changed": changed,
                        "source": "reachable_daemon_scan",
                    }),
                );
                Some((owner_endpoint, owner_status))
            }
            Err(error) => {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "keep_alive_preserved_owner_discovery_failed",
                    serde_json::json!({
                        "path": path,
                        "runtime_key": runtime_key,
                        "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                        "owner_server_version": owner_status.server_version,
                        "owner_server_pid": owner_status.server_pid,
                        "error": error.to_string(),
                    }),
                );
                None
            }
        }
    }

    fn preserved_owner_endpoint_for_request(
        &mut self,
        runtime_key: &str,
    ) -> Option<ServerEndpoint> {
        if self.terminals.has_session(runtime_key) {
            return None;
        }
        self.preserved_owner_for_runtime_key(runtime_key)
    }

    fn preserved_owner_error_means_missing_runtime(
        runtime_key: &str,
        error: &anyhow::Error,
    ) -> bool {
        let error_text = format!("{error:#}").to_ascii_lowercase();
        error_text.contains("terminal session not found")
            && error_text.contains(&runtime_key.to_ascii_lowercase())
    }

    fn handle_preserved_owner_request_error(
        &mut self,
        runtime_key: &str,
        owner_endpoint: &ServerEndpoint,
        error: &anyhow::Error,
    ) {
        append_trace_event(
            self.store.home_dir(),
            "daemon",
            "hot_update",
            "preserved_owner_request_failed",
            serde_json::json!({
                "runtime_key": runtime_key,
                "owner_endpoint": format!("{owner_endpoint:?}"),
                "error": error.to_string(),
            }),
        );
        if Self::preserved_owner_error_means_missing_runtime(runtime_key, error) {
            self.remove_preserved_owner(runtime_key, "owner_reported_missing_runtime");
            return;
        }
        // A timeout/connect-shaped failure means the owner socket is dead or
        // wedged: re-probing it with status() costs ANOTHER full request
        // timeout against the same dead socket (the post-swap 30s deaf
        // window). Mark it unreachable and remove without the re-probe.
        if preserved_owner_error_is_transport_shaped(error) {
            self.mark_preserved_owner_unreachable(owner_endpoint);
            self.remove_preserved_owner(runtime_key, "owner_unreachable_after_error");
            return;
        }
        match status(owner_endpoint) {
            Ok(status) if preserved_owner_status_serves_runtime_key(&status, runtime_key) => {}
            Ok(_) => self.remove_preserved_owner(runtime_key, "owner_missing_runtime_after_error"),
            Err(_) => {
                self.mark_preserved_owner_unreachable(owner_endpoint);
                self.remove_preserved_owner(runtime_key, "owner_unreachable_after_error")
            }
        }
    }

    /// Local write for a client keystroke, with recovery when no runtime
    /// holds the key ("the most helpless state", telemetry campaign
    /// 2026-07-17): after a daemon handoff the successor may never have
    /// created the runtime; surfacing the raw error printed
    /// "terminal session not found" into the PTY and left the session
    /// permanently untypeable — the user's only recovery was a manual
    /// session restart.
    fn write_local_terminal_with_lost_runtime_recovery(
        &mut self,
        path: &str,
        runtime_key: &str,
        data: &str,
    ) -> Result<ServerResponse> {
        match self.terminals.write(runtime_key, data) {
            Ok(()) => Ok(ServerResponse::Ack { message: None }),
            Err(error) if Self::preserved_owner_error_means_missing_runtime(runtime_key, &error) => {
                self.recover_terminal_write_lost_runtime(path, runtime_key, data, error)
            }
            Err(error) => Err(error),
        }
    }

    /// Recovery order: (1) ADOPT — another reachable daemon actually OWNS
    /// the runtime; register it as preserved owner and retry the write there
    /// (keystroke preserved). (2) RELAUNCH — this daemon represents the
    /// session, so re-create the runtime exactly like a user-initiated
    /// restart; the triggering keystroke is dropped (there was no PTY to
    /// receive it). (3) surface the original error.
    fn recover_terminal_write_lost_runtime(
        &mut self,
        path: &str,
        runtime_key: &str,
        data: &str,
        original_error: anyhow::Error,
    ) -> Result<ServerResponse> {
        let current_endpoint = default_endpoint(self.store.home_dir());
        let statuses = reachable_versioned_daemon_statuses_excluding_endpoint(
            self.store.home_dir(),
            &current_endpoint,
        );
        if let Some((owner_endpoint, owner_status)) =
            preserved_owner_candidate_for_runtime_key(statuses, runtime_key, std::process::id())
        {
            let registered = self.preserved_terminal_owners.upsert_runtime_owner(
                self.store.home_dir(),
                runtime_key,
                &owner_endpoint,
                &owner_status,
                Some(SERVER_PROTOCOL_VERSION.to_string()),
            );
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "hot_update",
                "terminal_write_adopted_preserved_owner",
                serde_json::json!({
                    "path": path,
                    "runtime_key": runtime_key,
                    "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                    "owner_server_version": owner_status.server_version,
                    "owner_server_pid": owner_status.server_pid,
                    "registered": registered.is_ok(),
                }),
            );
            match terminal_write(&owner_endpoint, runtime_key, data) {
                Ok(_) => return Ok(ServerResponse::Ack { message: None }),
                Err(error) => {
                    self.handle_preserved_owner_request_error(
                        runtime_key,
                        &owner_endpoint,
                        &error,
                    );
                }
            }
        }
        if self.server.represents_terminal_runtime_key(runtime_key)
            && self.terminal_write_relaunch_recovery_is_due(runtime_key)
        {
            match self.ensure_terminal_for_path(path) {
                Ok(prepare_message) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "terminal_io",
                        "terminal_write_relaunch_recovery",
                        serde_json::json!({
                            "path": path,
                            "runtime_key": runtime_key,
                            "dropped_bytes": data.len(),
                            "prepare_message": prepare_message,
                        }),
                    );
                    return Ok(ServerResponse::Ack {
                        message: Some(format!(
                            "terminal session {runtime_key} relaunched after lost runtime; keystroke dropped"
                        )),
                    });
                }
                Err(error) => {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "terminal_io",
                        "terminal_write_relaunch_recovery_failed",
                        serde_json::json!({
                            "path": path,
                            "runtime_key": runtime_key,
                            "error": format!("{error:#}"),
                        }),
                    );
                }
            }
        }
        Err(original_error)
    }

    /// One relaunch attempt per key per cooldown window, so a held key or
    /// autorepeat burst cannot storm `ensure_terminal_for_path`.
    fn terminal_write_relaunch_recovery_is_due(&mut self, runtime_key: &str) -> bool {
        const RELAUNCH_RECOVERY_COOLDOWN_MS: u64 = 30_000;
        let now_ms = current_millis_u64();
        let due = self
            .terminal_write_relaunch_attempted_at_ms
            .get(runtime_key)
            .is_none_or(|attempted| {
                now_ms.saturating_sub(*attempted) >= RELAUNCH_RECOVERY_COOLDOWN_MS
            });
        if due {
            self.terminal_write_relaunch_attempted_at_ms
                .insert(runtime_key.to_string(), now_ms);
        }
        due
    }

    fn reject_preserved_owner_saved_session_mismatch(
        &mut self,
        path: &str,
        runtime_path: &str,
        owner_endpoint: &ServerEndpoint,
        reason: &'static str,
    ) -> bool {
        match terminal_snapshot(owner_endpoint, runtime_path) {
            Ok((
                snapshot,
                _running,
                runtime_output_seen,
                _post_resize_output_seen,
                _last_resize_seq,
                _runtime_spawn_id,
            )) => {
                if !runtime_output_seen || snapshot.trim().is_empty() {
                    return false;
                }
                let runtime_output_mismatches_path = self
                    .server
                    .remote_resume_runtime_output_mismatches_path(path, snapshot.as_bytes());
                if !preserved_owner_saved_session_mismatch_should_detach(
                    path,
                    self.server.live_session_keep_alive(path),
                    self.server.live_session_is_temporary_update_restore(path),
                    runtime_output_mismatches_path,
                ) {
                    if runtime_output_mismatches_path {
                        append_trace_event(
                            self.store.home_dir(),
                            "daemon",
                            "hot_update",
                            "preserved_owner_saved_session_mismatch_ignored_for_keep_alive",
                            serde_json::json!({
                                "path": path,
                                "runtime_path": runtime_path,
                                "owner_endpoint": owner_endpoint_label(owner_endpoint),
                                "reason": reason,
                                "snapshot_bytes": snapshot.len(),
                            }),
                        );
                    }
                    return false;
                }
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "preserved_owner_saved_session_mismatch",
                    serde_json::json!({
                        "path": path,
                        "runtime_path": runtime_path,
                        "owner_endpoint": owner_endpoint_label(owner_endpoint),
                        "reason": reason,
                        "snapshot_bytes": snapshot.len(),
                    }),
                );
                self.remove_preserved_owner_runtime(runtime_path, reason);
                true
            }
            Err(error) => {
                self.handle_preserved_owner_request_error(runtime_path, owner_endpoint, &error);
                self.preserved_owner_endpoint_for_request(runtime_path)
                    .is_none()
            }
        }
    }

    fn ensure_terminal_for_path(&mut self, path: &str) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size(path, None)
    }

    fn adopt_legacy_local_codex_runtime(&mut self, session_id: &str, runtime_key: &str) {
        if !runtime_key.starts_with("codex-runtime://") {
            return;
        }
        let legacy_key = crate::local_live_runtime_key(session_id);
        if legacy_key == runtime_key {
            return;
        }
        let renamed = self.terminals.rename_session(&legacy_key, runtime_key);
        if renamed {
            info!(
                legacy_key = legacy_key.as_str(),
                runtime_key, "adopted legacy local Codex runtime key"
            );
        }
    }

    fn ensure_terminal_for_path_with_initial_size(
        &mut self,
        path: &str,
        initial_size: Option<(u16, u16)>,
    ) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size_and_seed(path, initial_size, true)
    }

    fn ensure_terminal_for_path_for_startup_prewarm(
        &mut self,
        path: &str,
        seed_remote_snapshot: bool,
    ) -> Result<Option<String>> {
        self.ensure_terminal_for_path_with_initial_size_and_seed(path, None, seed_remote_snapshot)
    }

    fn ensure_terminal_for_path_with_initial_size_and_seed(
        &mut self,
        path: &str,
        initial_size: Option<(u16, u16)>,
        seed_remote_snapshot: bool,
    ) -> Result<Option<String>> {
        let prepare_message = {
            let _perf = yggterm_core::PerfGuard::new(
                self.store.home_dir(),
                "attach",
                "managed_cli_ensure",
            );
            self.server.ensure_managed_cli_for_session_path(path)?
        };
        if path.starts_with("remote-session://")
            && let Ok(home) = crate::resolve_yggterm_home()
        {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "remote_saved_session_preflight_elided_runtime_launch",
                serde_json::json!({
                    "path": path,
                    "reason": "resume_command_owns_missing_session_failure",
                }),
            );
        }
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "before_request_terminal_launch",
                serde_json::json!({
                    "path": path,
                    "prepare_message": prepare_message,
                }),
            );
        }
        {
            let _perf = yggterm_core::PerfGuard::new(
                self.store.home_dir(),
                "attach",
                "request_terminal_launch",
            );
            if seed_remote_snapshot {
                self.server.request_terminal_launch_for_path(path);
            } else {
                self.server
                    .request_terminal_launch_for_path_preserving_active(path);
            }
        }
        let runtime_path = self.terminal_runtime_key_for_path(path);
        let preserved_owner_status = self
            .preserved_owner_status_for_runtime_key(&runtime_path)
            .or_else(|| {
                self.discover_keep_alive_preserved_owner_for_runtime_key(path, &runtime_path)
            });
        if let Some((owner_endpoint, _owner_status)) = preserved_owner_status
            && !self.reject_preserved_owner_saved_session_mismatch(
                path,
                &runtime_path,
                &owner_endpoint,
                "saved_session_mismatch",
            )
        {
            return Ok(Some(format!(
                "using hot-update preserved terminal owner for {runtime_path}"
            )));
        }
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "after_request_terminal_launch",
                serde_json::json!({
                    "path": path,
                }),
            );
        }
        // A dead local CC runtime must be relaunched from CURRENT on-disk truth,
        // never by replaying the stored point-in-time command (birth-shaped
        // `--session-id` collides with its own transcript; a mis-attributed
        // identity rebind points it at a foreign id — "session id already in
        // use", jojo 2026-07-13). Guarded on not-running so a live runtime's
        // spec is never changed underneath it.
        if !self.terminals.session_is_running(&runtime_path)
            && self.server.refresh_local_cc_relaunch_launch_command(path)
        {
            let _ = self.persist_state_only();
        }
        let Some((stored_launch_command, cwd)) = self.server.terminal_spec(path) else {
            bail!("no terminal spec for session: {path}");
        };
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "terminal_spec_resolved",
                serde_json::json!({
                    "path": path,
                    "cwd": cwd,
                    "stored_launch_command": stored_launch_command,
                }),
            );
        }
        // The server-owned session model updates the managed session launch command
        // before terminal ensure runs. Recomputing a legacy direct-attach command here
        // can silently bypass the daemon-owned remote runtime path on startup restore.
        let launch_command = terminal_launch_command_for_path(
            path,
            stored_launch_command,
            self.server
                .remote_direct_attach_launch_command_for_path(path),
        );
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "server",
                "terminal_spec",
                "resolved",
                serde_json::json!({
                    "path": path,
                    "cwd": cwd,
                    "launch_command": launch_command,
                }),
            );
        }
        let seed_prefill =
            if terminal_ensure_should_seed_remote_snapshot(path, seed_remote_snapshot) {
                match self.server.remote_resume_seed_snapshot_for_path(path) {
                    Ok(Some(prefill)) => {
                        if let Ok(home) = crate::resolve_yggterm_home() {
                            append_trace_event(
                                &home,
                                "daemon",
                                "terminal_ensure",
                                "remote_resume_seed_snapshot",
                                serde_json::json!({
                                    "path": path,
                                    "bytes": prefill.len(),
                                }),
                            );
                        }
                        Some(prefill)
                    }
                    Ok(None) => self.server.remote_resume_seed_fallback_for_path(path),
                    Err(error) => {
                        if let Ok(home) = crate::resolve_yggterm_home() {
                            append_trace_event(
                                &home,
                                "daemon",
                                "terminal_ensure",
                                "remote_resume_seed_snapshot_error",
                                serde_json::json!({
                                    "path": path,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        self.server.remote_resume_seed_fallback_for_path(path)
                    }
                }
            } else {
                if path.starts_with("remote-session://")
                    && let Ok(home) = crate::resolve_yggterm_home()
                {
                    append_trace_event(
                        &home,
                        "daemon",
                        "terminal_ensure",
                        "remote_resume_seed_snapshot_skipped",
                        serde_json::json!({
                            "path": path,
                            "reason": "startup_prewarm_latency_budget",
                        }),
                    );
                }
                None
            };
        let fresh_remote_codex_start = self.server.session_starts_new_remote_codex(path);
        let remote_attach_startup_grace_ms = if fresh_remote_codex_start {
            REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS
        } else {
            REMOTE_ATTACH_STARTUP_GRACE_MS
        };
        if self.terminals.has_session(&runtime_path) {
            let still_running = self.terminals.session_is_running(&runtime_path);
            let has_runtime_output = self.terminals.session_has_runtime_output(&runtime_path);
            let runtime_age_ms = self
                .terminals
                .session_runtime_age_ms(&runtime_path)
                .unwrap_or(0);
            let runtime_snapshot = if has_runtime_output {
                self.terminals.session_snapshot(&runtime_path)
            } else {
                None
            };
            let runtime_saved_session_mismatch = path
                .strip_prefix("remote-session://")
                .and_then(|_| {
                    runtime_snapshot.as_deref().map(|snapshot| {
                        self.server
                            .remote_resume_runtime_output_mismatches_path(path, snapshot.as_bytes())
                    })
                })
                .unwrap_or(false);
            let remote_runtime_output_requires_restart = has_runtime_output
                && self
                    .terminals
                    .read(&runtime_path, 0)
                    .map(|stream| {
                        let snapshot = stream
                            .chunks
                            .into_iter()
                            .map(|chunk| chunk.data)
                            .collect::<String>();
                        remote_resume_runtime_output_requires_restart(snapshot.as_bytes())
                    })
                    .unwrap_or(false);
            let remote_resume_path =
                path.starts_with("remote-session://") || runtime_saved_session_mismatch;
            let stale_remote_attach = remote_resume_path
                && remote_resume_stale_attach(
                    has_runtime_output,
                    runtime_age_ms,
                    remote_runtime_output_requires_restart,
                    remote_attach_startup_grace_ms,
                );
            let remote_saved_session_mismatch_requires_restart =
                remote_resume_saved_session_mismatch_requires_restart(
                    remote_resume_path,
                    has_runtime_output,
                    runtime_age_ms,
                    runtime_saved_session_mismatch,
                    remote_attach_startup_grace_ms,
                );
            let blank_remote_attach = remote_resume_path
                && !has_runtime_output
                && runtime_age_ms >= remote_attach_startup_grace_ms;
            let spec_matches =
                self.terminals
                    .session_matches_spec(&runtime_path, &launch_command, cwd.as_deref());
            let keep_alive_runtime = self.server.live_session_keep_alive(path);
            let temporary_update_restore_runtime =
                self.server.live_session_is_temporary_update_restore(path);
            let restart_protected_runtime = keep_alive_runtime || temporary_update_restore_runtime;
            let (needs_restart, restart_blocked_by_protected_runtime) =
                terminal_reuse_needs_restart(
                    still_running,
                    remote_resume_path,
                    restart_protected_runtime,
                    stale_remote_attach,
                    blank_remote_attach,
                    remote_saved_session_mismatch_requires_restart,
                    spec_matches,
                );
            if let Ok(home) = crate::resolve_yggterm_home() {
                append_trace_event(
                    &home,
                    "server",
                    "terminal_spec",
                    "reuse_check",
                    serde_json::json!({
                        "path": path,
                        "still_running": still_running,
                        "has_runtime_output": has_runtime_output,
                        "runtime_age_ms": runtime_age_ms,
                        "remote_runtime_output_requires_restart": remote_runtime_output_requires_restart,
                        "stale_remote_attach": stale_remote_attach,
                        "blank_remote_attach": blank_remote_attach,
                        "fresh_remote_codex_start": fresh_remote_codex_start,
                        "remote_attach_startup_grace_ms": remote_attach_startup_grace_ms,
                        "runtime_saved_session_mismatch": runtime_saved_session_mismatch,
                        "remote_saved_session_mismatch_requires_restart": remote_saved_session_mismatch_requires_restart,
                        "keep_alive_runtime": keep_alive_runtime,
                        "temporary_update_restore_runtime": temporary_update_restore_runtime,
                        "restart_protected_runtime": restart_protected_runtime,
                        "restart_blocked_by_keep_alive": restart_blocked_by_protected_runtime,
                        "restart_blocked_by_protected_runtime": restart_blocked_by_protected_runtime,
                        "spec_matches": spec_matches,
                        "remote_resume_path": remote_resume_path,
                        "needs_restart": needs_restart,
                    }),
                );
            }
            if !needs_restart {
                let remote_reuse_missing_retained_scrollback = path
                    .starts_with("remote-session://")
                    && has_runtime_output
                    && seed_prefill
                        .as_deref()
                        .is_some_and(terminal_data_has_scrollback_text)
                    && !self
                        .terminals
                        .session_initial_read_has_scrollback(&runtime_path);
                if (!has_runtime_output || remote_reuse_missing_retained_scrollback)
                    && let Some(prefill) = seed_prefill.as_deref()
                {
                    if let Ok(home) = crate::resolve_yggterm_home() {
                        append_trace_event(
                            &home,
                            "daemon",
                            "terminal_ensure",
                            "reuse_session_seed_prefill",
                            serde_json::json!({
                                "path": path,
                                "bytes": prefill.len(),
                                "reason": if remote_reuse_missing_retained_scrollback {
                                    "remote_reuse_missing_retained_scrollback"
                                } else {
                                    "no_runtime_output"
                                },
                            }),
                        );
                    }
                    self.terminals.seed_session(&runtime_path, prefill)?;
                }
                self.prune_duplicate_legacy_owned_runtime_sessions("terminal_ensure_reuse");
                return Ok(prepare_message);
            }
            if needs_restart {
                if let Some(reason) = self.server.terminal_launch_blocked_reason(path) {
                    if let Ok(home) = crate::resolve_yggterm_home() {
                        append_trace_event(
                            &home,
                            "daemon",
                            "terminal_ensure",
                            "restart_blocked_by_launch_error",
                            serde_json::json!({
                                "path": path,
                                "runtime_path": runtime_path,
                                "reason": reason,
                            }),
                        );
                    }
                    bail!("{reason}");
                }
                let stop_command = self.server.terminal_stop_command(path);
                if let Ok(home) = crate::resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "terminal_ensure",
                        "restart_session_begin",
                        serde_json::json!({
                            "path": path,
                            "cwd": cwd,
                            "launch_command": launch_command,
                            "stop_command": stop_command,
                            "initial_cols": initial_size.map(|(cols, _)| cols),
                            "initial_rows": initial_size.map(|(_, rows)| rows),
                        }),
                    );
                }
                self.terminals.restart_session_with_size(
                    &runtime_path,
                    &launch_command,
                    cwd.as_deref(),
                    stop_command.as_deref(),
                    initial_size,
                )?;
                if let Ok(home) = crate::resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "terminal_ensure",
                        "restart_session_end",
                        serde_json::json!({
                            "path": path,
                        }),
                    );
                }
                if let Some(prefill) = seed_prefill.as_deref() {
                    self.terminals.seed_session(&runtime_path, prefill)?;
                }
            }
            self.prune_duplicate_legacy_owned_runtime_sessions("terminal_ensure_restart");
            return Ok(prepare_message);
        }
        if let Some(reason) = self.server.terminal_launch_blocked_reason(path) {
            if let Ok(home) = crate::resolve_yggterm_home() {
                append_trace_event(
                    &home,
                    "daemon",
                    "terminal_ensure",
                    "ensure_session_blocked_by_launch_error",
                    serde_json::json!({
                        "path": path,
                        "runtime_path": runtime_path,
                        "reason": reason,
                    }),
                );
            }
            bail!("{reason}");
        }
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "ensure_session_begin",
                serde_json::json!({
                    "path": path,
                    "cwd": cwd,
                    "launch_command": launch_command,
                    "initial_cols": initial_size.map(|(cols, _)| cols),
                    "initial_rows": initial_size.map(|(_, rows)| rows),
                }),
            );
        }
        // XTERM-BUG: squish-and-bottom-paint-on-reresume — when no client grid is
        // supplied (e.g. the startup re-resume after a daemon-PROCESS restart, which
        // otherwise defaults to 120x36 and only repairs the ACTIVE session), fall back
        // to the session's PERSISTED grid so EVERY re-resumed session comes back at its
        // real grid, not just the mounted one.
        let effective_initial_size = initial_size.or_else(|| self.server.session_pty_grid(path));
        self.terminals.ensure_session_with_size(
            &runtime_path,
            &launch_command,
            cwd.as_deref(),
            effective_initial_size,
        )?;
        // D1 (campaign-xterm-dealbreakers): ensure_session_with_size only applies
        // the grid when it CREATES the PTY. After a daemon restart/handoff the
        // successor auto-resumes the session at DEFAULT 120x36, and a later client
        // (re)attach that carries the client's REAL grid would leave the PTY stale
        // → codex repaints ~120 cols inside the 159-col xterm (squish), and the
        // partial reflow drops composer cell bg (bottom-paint bg-split). When the
        // client provides a grid that differs from the running PTY, resize the PTY
        // to match so codex repaints full-width and clean. Best-effort: never fail
        // the ensure if the resize errors. See campaign D1.
        if let Some((cols, rows)) = effective_initial_size
            && cols > 0
            && rows > 0
            && self.terminals.session_size(&runtime_path) != Some((cols, rows))
        {
            // Grid mismatch on re-resume = the squish case (PTY spawned at a stale/default
            // size). NUDGE: resize to a transient rows-1 then the real grid, so codex's
            // event loop sees TWO distinct SIGWINCH events rather than one — maximizing the
            // chance an idle codex actually processes the resize and repaints full-size.
            // Best-effort safety net: idle codex MAY still ignore SIGWINCH entirely (only
            // genuine input reliably repaints it — empirically tested), in which case the
            // born-at-correct-size persistence above is what actually prevents the squish;
            // this only fires on a real mismatch so a brief reflow is strictly better than a
            // stuck squish. See campaign D1 / the squish root cause.
            let nudge_rows = rows.saturating_sub(1).max(1);
            if nudge_rows != rows {
                let _ = self.terminals.resize(&runtime_path, cols, nudge_rows);
            }
            let resized = self.terminals.resize(&runtime_path, cols, rows);
            if let Ok(home) = crate::resolve_yggterm_home() {
                append_trace_event(
                    &home,
                    "daemon",
                    "terminal_ensure",
                    "reattach_grid_resync",
                    serde_json::json!({
                        "path": path,
                        "client_cols": cols,
                        "client_rows": rows,
                        "nudged": nudge_rows != rows,
                        "ok": resized.is_ok(),
                    }),
                );
            }
        }
        // RECORD-ON-CREATE (born-at-correct-size invariant): persist whatever grid this
        // session was ensured at — whether client-supplied (`initial_size`) or the
        // persisted fallback — so a FUTURE re-resume always has the real grid and codex
        // initial-paints correctly, even if the client never sends a separate
        // TerminalResize. Flush only on change. Pairs with the synchronous flush in the
        // TerminalResize handler. See campaign D1 / the squish root cause.
        if let Some((cols, rows)) = effective_initial_size
            && self.server.record_session_pty_grid(path, cols, rows)
        {
            let _ = self.persist_state_only();
        }
        // Run #19: EVERY ensure of a remote session pins the REMOTE daemon's
        // PTY to the known grid — not just the local-mismatch resync. The
        // squish lives on the remote owner, where the local attachment can be
        // correct while the remote runtime was recreated at DEFAULT (the
        // local-mismatch gate never fires then). Latest-wins + single
        // in-flight makes this cheap and idempotent; no-op for local paths.
        if let Some((cols, rows)) = effective_initial_size {
            self.forward_remote_pty_resize(path, cols, rows);
        }
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "terminal_ensure",
                "ensure_session_end",
                serde_json::json!({
                    "path": path,
                }),
            );
        }
        if let Some(prefill) = seed_prefill.as_deref() {
            self.terminals.seed_session(&runtime_path, prefill)?;
        }
        self.prune_duplicate_legacy_owned_runtime_sessions("terminal_ensure_new");
        Ok(prepare_message)
    }

    fn ensure_terminal_for_active(&mut self) -> Result<()> {
        let Some(path) = self.server.active_session_path().map(ToOwned::to_owned) else {
            bail!("no active session");
        };
        let _ = self.ensure_terminal_for_path(&path)?;
        Ok(())
    }

    fn persist(&mut self) -> Result<()> {
        if self.update_restart_state_written || self.superseded_routine_persist_muted {
            // Never clobber the update-restart snapshot during retire, and
            // never clobber the SUCCESSOR's state file once a newer daemon is
            // live (gate #8 — see the field docs).
            return Ok(());
        }
        let _perf = yggterm_core::PerfGuard::new(self.store.home_dir(), "daemon", "persist");
        self.refresh_live_codex_runtime_identities_for_persistence();
        self.refresh_live_claude_code_runtime_identities_for_persistence();
        // Ledger chokepoint: every lifecycle event that can change the live
        // order flows through here, so the shared-scope ledger observes the
        // order exactly once per mutation, with no per-handler bookkeeping.
        self.record_row_order_ledger(None);
        write_persisted_state(&self.state_path, &self.server.persisted_state())
    }

    /// Record the current live order into the shared row-order ledger scope
    /// and, when a request carried a client scope, into that scope too. Saves
    /// the ledger only when something changed.
    fn record_row_order_ledger(&mut self, client_scope: Option<&str>) {
        let live_order = self.server.live_session_order_keys().to_vec();
        let mut changed = self.row_order_ledger.record_live_order(
            crate::row_order_ledger::SHARED_ROW_ORDER_SCOPE,
            &live_order,
        );
        if let Some(scope) = client_scope
            .map(str::trim)
            .filter(|scope| !scope.is_empty() && *scope != crate::row_order_ledger::SHARED_ROW_ORDER_SCOPE)
        {
            changed |= self.row_order_ledger.record_live_order(scope, &live_order);
        }
        if changed
            && let Err(error) = self.row_order_ledger.save(self.store.home_dir())
        {
            tracing::warn!(%error, "failed to save row-order ledger");
        }
    }

    /// Restore `path` to its remembered slot in the live order after it
    /// (re)entered the live set at a daemon-native position. `Unknown` rows
    /// keep the caller's native placement.
    fn apply_row_order_ledger_placement(&mut self, path: &str, client_scope: Option<&str>) {
        let scope = client_scope
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .unwrap_or(crate::row_order_ledger::SHARED_ROW_ORDER_SCOPE);
        let Some((live_key, _)) = self.server.resolve_live_session_entry(path) else {
            return;
        };
        let live_order = self.server.live_session_order_keys().to_vec();
        let is_live = |candidate: &str| {
            candidate != live_key && live_order.iter().any(|entry| entry == candidate)
        };
        match self.row_order_ledger.placement_for(scope, path, is_live) {
            crate::row_order_ledger::RowLedgerPlacement::AfterLive(anchor) => {
                let anchor_key = self
                    .server
                    .resolve_live_session_entry(&anchor)
                    .map(|(key, _)| key)
                    .unwrap_or(anchor);
                self.server
                    .move_live_session_after(&live_key, Some(anchor_key.as_str()));
            }
            crate::row_order_ledger::RowLedgerPlacement::Front => {
                self.server.move_live_session_after(&live_key, None);
            }
            crate::row_order_ledger::RowLedgerPlacement::Unknown => {}
        }
    }

    /// Write persisted state WITHOUT re-resolving live-session runtime identities.
    /// The full `persist()` runs `refresh_live_codex/claude_runtime_identities_*`, which
    /// re-keys live sessions — fine on genuine lifecycle events, but running it on the
    /// HIGH-FREQUENCY grid-flush path (every TerminalResize / ensure) churns identity
    /// resolution far more than needed and widens the window for the local://<->
    /// codex-runtime:// re-key split (Bug 9 / the focus->start-page + phantom regression).
    /// The grid flush only needs the grid on disk, so skip the identity refresh here.
    /// Genuine lifecycle events still call the full `persist()`. See campaign D1 / the
    /// born-at-correct-size synchronous flush.
    fn persist_state_only(&self) -> Result<()> {
        if self.update_restart_state_written || self.superseded_routine_persist_muted {
            return Ok(());
        }
        write_persisted_state(&self.state_path, &self.server.persisted_state())
    }

    fn persisted_state_for_update_restart(&mut self) -> PersistedDaemonState {
        self.refresh_live_codex_runtime_identities_for_persistence();
        self.refresh_live_claude_code_runtime_identities_for_persistence();
        self.server.persisted_state_for_update_restart()
    }

    fn handle_request(&mut self, request: ServerRequest) -> Result<ServerResponse> {
        // Apply registry removals queued by the background duplicate-runtime
        // prune (cheap; usually empty).
        self.drain_pending_preserved_owner_removals();
        let request_name = server_request_name(&request);
        // App profiling system: time every daemon request, tagged by name, so
        // `server perf-summary` surfaces the slow ones (the switch path is `terminal_ensure`).
        // Drop-based so `?` early returns and panics still record. No-op when the
        // perf_profiling_enabled setting is off.
        let _perf_request = yggterm_core::PerfGuard::new(
            self.store.home_dir(),
            "daemon_request",
            request_name,
        );
        let trace_request = daemon_request_trace_enabled(request_name);
        if trace_request {
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "request",
                "begin",
                serde_json::json!({ "request": request_name }),
            );
        }
        let response = match request {
            ServerRequest::Ping => ServerResponse::Pong,
            ServerRequest::Status => ServerResponse::Status(self.status()),
            ServerRequest::WorkingFlags => ServerResponse::WorkingFlags {
                flags: self.working_flags(),
            },
            ServerRequest::Snapshot => self.snapshot_response(None),
            ServerRequest::PrepareUpdateRestart => {
                // The snapshot write is BEST-EFFORT: it lets the relaunched GUI
                // restore its session view, but a failed write must NOT abort the
                // update restart. Previously the `?` propagated the error and the
                // GUI surfaced "Update Restart Blocked" (the session-protection
                // step the user hit on popo), leaving them stranded on the old
                // version. The daemon-owned PTYs survive the GUI relaunch
                // regardless, and PrepareClientClose still preserves non-keep-alive
                // sessions via the staged-version gate even without a fresh
                // snapshot. Always set the flag so that gate trips, and proceed.
                let state = self.persisted_state_for_update_restart();
                if let Err(error) = write_persisted_state(&self.state_path, &state) {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "lifecycle",
                        "update_restart_state_write_failed",
                        serde_json::json!({
                            "state_path": self.state_path.display().to_string(),
                            "error": error.to_string(),
                        }),
                    );
                    tracing::warn!(
                        state_path = %self.state_path.display(),
                        error = %error,
                        "failed to persist daemon state for update restart; proceeding without blocking the restart"
                    );
                }
                self.update_restart_state_written = true;
                ServerResponse::Ack {
                    message: Some("update restart prepared".to_string()),
                }
            }
            ServerRequest::PrepareClientClose => {
                // GATE #5 ROOT (persistence saga, 2026-06-11): a GUI restart
                // during an UPDATE looks exactly like the user closing the
                // GUI — this handler then removes every non-keep-alive live
                // session "legitimately" per [[session-keep-alive-spec]]
                // ("only user-closing-GUI may kill non-keep-alive sessions").
                // Every deploy swap nuked the local rows through this front
                // door, which is why no persistence gate ever traced. When an
                // update is staged (install-state names a different version)
                // or the update-restart snapshot was already written, the
                // close is part of an update restart: preserve everything.
                let staged_version = staged_direct_install_version();
                if client_close_should_preserve_for_update(
                    self.update_restart_state_written,
                    staged_version.as_deref(),
                    SERVER_PROTOCOL_VERSION,
                ) {
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "lifecycle",
                        "client_close_preserved_for_update",
                        serde_json::json!({
                            "staged_version": staged_version,
                            "running_version": SERVER_PROTOCOL_VERSION,
                            "update_restart_state_written": self.update_restart_state_written,
                        }),
                    );
                    return Ok(ServerResponse::Ack {
                        message: Some(
                            "update staged; preserving non-keep-alive live sessions".to_string(),
                        ),
                    });
                }
                let force_after =
                    std::time::Duration::from_secs(CLIENT_CLOSE_FORCE_SHUTDOWN_AFTER_SECS);
                let paths = self.server.non_keep_alive_live_session_paths();
                let mut metadata_removed = 0usize;
                let mut terminal_shutdowns = 0usize;
                let mut remote_shutdowns = 0usize;
                let mut errors = Vec::<String>::new();
                for path in paths {
                    let remote_target = self.server.remote_shutdown_target_for_path(&path);
                    let stop_command = self.server.terminal_stop_command(&path);
                    let runtime_path = self.server.terminal_runtime_key_for_path(&path);
                    if let Some((machine, session_id)) = remote_target {
                        match request_remote_codex_session_shutdown(
                            &machine,
                            &session_id,
                            force_after,
                        ) {
                            Ok(()) => remote_shutdowns += 1,
                            Err(error) => errors.push(format!("{path}: {error}")),
                        }
                    }
                    match self.terminals.remove_session_gracefully_with_force_after(
                        &runtime_path,
                        stop_command.as_deref(),
                        force_after,
                    ) {
                        Ok(true) => terminal_shutdowns += 1,
                        Ok(false) => {}
                        Err(error) => errors.push(format!("{path}: {error}")),
                    }
                    match self.server.remove_live_session(&path) {
                        Ok(true) => metadata_removed += 1,
                        Ok(false) => {}
                        Err(error) => errors.push(format!("{path}: {error}")),
                    }
                    self.remove_preserved_owner_runtime(&runtime_path, "client_close_non_keep");
                }
                self.persist()?;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "lifecycle",
                    "client_close_prepared",
                    serde_json::json!({
                        "metadata_removed": metadata_removed,
                        "terminal_shutdowns": terminal_shutdowns,
                        "remote_shutdowns": remote_shutdowns,
                        "force_after_seconds": CLIENT_CLOSE_FORCE_SHUTDOWN_AFTER_SECS,
                        "errors": errors,
                    }),
                );
                let message = if metadata_removed == 0 {
                    "no non-keep-alive live sessions to close".to_string()
                } else {
                    format!(
                        "closing {metadata_removed} non-keep-alive live session(s) gracefully; force cleanup is scheduled after 1 hour"
                    )
                };
                ServerResponse::Ack {
                    message: Some(message),
                }
            }
            ServerRequest::HotRestart {
                daemon_executable,
                expected_version,
                expected_build_id,
                reason,
                force,
            } => {
                let runtime_status = self.status();
                let owned_terminal_session_keys = self.terminals.session_keys();
                let terminal_session_key_set = runtime_status
                    .terminal_session_keys
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>();
                if hot_restart_should_defer_for_session_survival(&owned_terminal_session_keys) {
                    if let Some(duplicate_owner) = hot_restart_duplicate_runtime_owner_status(
                        self.store.home_dir(),
                        expected_version.as_deref(),
                        &owned_terminal_session_keys,
                        std::process::id(),
                    ) {
                        let mut registry =
                            PreservedTerminalOwnerRegistry::load(self.store.home_dir());
                        let mut removed_runtime_keys = Vec::new();
                        for runtime_key in &owned_terminal_session_keys {
                            if registry.remove_key(runtime_key) {
                                removed_runtime_keys.push(runtime_key.clone());
                            }
                        }
                        if !removed_runtime_keys.is_empty() {
                            registry.save(self.store.home_dir())?;
                        }
                        append_trace_event(
                            self.store.home_dir(),
                            "daemon",
                            "lifecycle",
                            "hot_restart_duplicate_runtime_owner_retired",
                            serde_json::json!({
                                "daemon_executable": daemon_executable,
                                "expected_version": expected_version,
                                "expected_build_id": expected_build_id,
                                "reason": reason,
                                "server_version": SERVER_PROTOCOL_VERSION,
                                "server_build_id": current_build_id(),
                                "pid": std::process::id(),
                                "owned_terminal_session_count": owned_terminal_session_keys.len(),
                                "owned_terminal_session_keys": &owned_terminal_session_keys,
                                "duplicate_owner_pid": duplicate_owner.server_pid,
                                "duplicate_owner_version": duplicate_owner.server_version,
                                "duplicate_owner_build_id": duplicate_owner.server_build_id,
                                "duplicate_owner_terminal_session_keys": &duplicate_owner.terminal_session_keys,
                                "removed_preserved_owner_keys": removed_runtime_keys,
                                "update_priority": "duplicate_runtime_retire",
                            }),
                        );
                        return Ok(ServerResponse::Ack {
                            message: Some(format!(
                                "retiring duplicate stale daemon; {} runtime(s) are already owned by {} pid {}",
                                owned_terminal_session_keys.len(),
                                duplicate_owner.server_version,
                                duplicate_owner.server_pid,
                            )),
                        });
                    }
                    if !force
                        && expected_version
                            .as_deref()
                            .is_none_or(|version| version == SERVER_PROTOCOL_VERSION)
                    {
                        return Ok(ServerResponse::Error {
                            message: "hot update handoff requires a different target daemon version when live terminal runtimes are present (pass --force to override for dev/agent deploys)".to_string(),
                        });
                    }
                    // NO idle gate here. This handoff PRESERVES every runtime —
                    // its own success message is "preserving N live terminal
                    // runtime(s)": this process keeps its PTY fds and lingers as
                    // the preserved owner while the successor adopts the streams.
                    // Gating it on session activity meant one busy agent session
                    // blocked the handoff for all of them, and since progressive
                    // migration only starts AFTER a handoff, the very mechanism
                    // built to drain sessions "all but the busy few" could never
                    // run. jojo sat on 2.9.63 for a day that way.
                    // The gate now guards the cold-shutdown retire, which is the
                    // path that actually kills PTYs.
                    // [[finding-hot-update-never-converges-idle-gate]]
                    let daemon_executable = canonical_hot_restart_executable(&daemon_executable)?;
                    // Under a managed Direct install, every launched binary
                    // re-execs to install-state.active_version. Flip install-state
                    // to the handoff TARGET before spawning so the successor stays
                    // on the target version, binds its own version socket, and
                    // adopts the preserved owners — instead of re-exec'ing back to
                    // the old active version and deferring to the live old daemon.
                    // Best-effort: raw/dev installs return false (nothing to flip).
                    // See [[finding-hot-update-interrupts-remote-sessions]].
                    if let Some(target_version) = expected_version.as_deref() {
                        let target_gui_executable = daemon_executable.with_file_name(
                            companion_gui_executable_file_name(&daemon_executable),
                        );
                        match yggterm_core::promote_direct_install_active_version(
                            target_version,
                            &target_gui_executable,
                        ) {
                            Ok(managed) => append_trace_event(
                                self.store.home_dir(),
                                "daemon",
                                "hot_update",
                                "hot_update_install_state_promoted",
                                serde_json::json!({
                                    "target_version": target_version,
                                    "target_gui_executable": target_gui_executable.display().to_string(),
                                    "managed_direct_install": managed,
                                }),
                            ),
                            Err(error) => append_trace_event(
                                self.store.home_dir(),
                                "daemon",
                                "hot_update",
                                "hot_update_install_state_promote_failed",
                                serde_json::json!({
                                    "target_version": target_version,
                                    "error": error.to_string(),
                                }),
                            ),
                        }
                    }
                    let state = self
                        .server
                        .persisted_state_for_update_restart_with_runtime_keys(
                            &terminal_session_key_set,
                        );
                    write_persisted_state(&self.state_path, &state)?;
                    self.update_restart_state_written = true;
                    let owner_endpoint = default_endpoint(self.store.home_dir());
                    let represented_preserved_owner_keys = runtime_status
                        .preserved_terminal_owner_keys
                        .iter()
                        .cloned()
                        .collect::<HashSet<_>>();
                    let owned_terminal_session_key_set = owned_terminal_session_keys
                        .iter()
                        .cloned()
                        .collect::<HashSet<_>>();
                    let preserved_owner_status_cache = self.preserved_owner_status_cache();
                    let existing_entries = preserved_owner_entries_live_for_handoff(
                        &self.preserved_terminal_owners.entries,
                        &owned_terminal_session_key_set,
                        &represented_preserved_owner_keys,
                        &preserved_owner_status_cache,
                    );
                    let registry = PreservedTerminalOwnerRegistry::write_handoff(
                        self.store.home_dir(),
                        &owner_endpoint,
                        &runtime_status,
                        expected_version.clone(),
                        owned_terminal_session_keys.clone(),
                        existing_entries,
                    )?;
                    let spawn_result =
                        spawn_hot_restart_daemon_process(&daemon_executable, &owner_endpoint);
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "lifecycle",
                        "hot_update_handoff_prepared",
                        serde_json::json!({
                            "daemon_executable": daemon_executable.display().to_string(),
                            "expected_version": expected_version,
                            "expected_build_id": expected_build_id,
                            "reason": reason,
                            "server_version": SERVER_PROTOCOL_VERSION,
                            "server_build_id": current_build_id(),
                            "pid": std::process::id(),
                            "owner_endpoint": format!("{owner_endpoint:?}"),
                            "owned_terminal_session_count": owned_terminal_session_keys.len(),
                            "owned_terminal_session_keys": &owned_terminal_session_keys,
                            "terminal_session_count": runtime_status.terminal_session_count,
                            "terminal_session_keys": &runtime_status.terminal_session_keys,
                            "restored_live_sessions": runtime_status.restored_live_sessions,
                            "managed_session_count": runtime_status.managed_session_count,
                            "handoff_runtime_keys": registry.keys(),
                            "spawn_ok": spawn_result.is_ok(),
                            "spawn_error": spawn_result.as_ref().err().map(|error| error.to_string()),
                            "update_priority": "handoff_preserve_sessions",
                        }),
                    );
                    spawn_result?;
                    return Ok(ServerResponse::HotUpdateHandoff {
                        message: Some(format!(
                            "hot update handoff started: preserving {} live terminal runtime(s) on {}",
                            registry.entries.len(),
                            owner_endpoint_label(&owner_endpoint),
                        )),
                        owner_endpoint: owner_endpoint_label(&owner_endpoint),
                        owner_server_version: runtime_status.server_version,
                        owner_server_pid: runtime_status.server_pid,
                        target_server_version: expected_version,
                        runtime_keys: registry.keys(),
                    });
                }
                let daemon_executable = canonical_hot_restart_executable(&daemon_executable)?;
                let state = self
                    .server
                    .persisted_state_for_update_restart_with_runtime_keys(
                        &terminal_session_key_set,
                    );
                write_persisted_state(&self.state_path, &state)?;
                self.update_restart_state_written = true;
                let represented_preserved_owner_keys = runtime_status
                    .preserved_terminal_owner_keys
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>();
                let preserved_owner_status_cache = self.preserved_owner_status_cache();
                let retained_preserved_owner_keys = preserved_owner_entries_live_for_handoff(
                    &self.preserved_terminal_owners.entries,
                    &HashSet::new(),
                    &represented_preserved_owner_keys,
                    &preserved_owner_status_cache,
                )
                .into_iter()
                .map(|entry| entry.runtime_key)
                .collect::<HashSet<_>>();
                let removed_preserved_owner_keys = self
                    .preserved_terminal_owners
                    .retain_represented_keys(|key| retained_preserved_owner_keys.contains(key));
                if !removed_preserved_owner_keys.is_empty() {
                    let _ = self.preserved_terminal_owners.save(self.store.home_dir());
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "hot_update",
                        "preserved_owner_registry_pruned",
                        serde_json::json!({
                            "reason": "hot_restart_prepare",
                            "removed_runtime_keys": removed_preserved_owner_keys,
                            "remaining_runtime_keys": self.preserved_terminal_owners.keys(),
                        }),
                    );
                }
                let preserved_owner_registry_retargeted = self
                    .preserved_terminal_owners
                    .retarget_expected_server_version(
                        self.store.home_dir(),
                        expected_version.clone(),
                    )?;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "lifecycle",
                    "hot_restart_prepared",
                    serde_json::json!({
                        "daemon_executable": daemon_executable.display().to_string(),
                        "expected_version": expected_version,
                        "expected_build_id": expected_build_id,
                        "reason": reason,
                        "server_version": SERVER_PROTOCOL_VERSION,
                        "server_build_id": current_build_id(),
                        "pid": std::process::id(),
                        "owned_terminal_session_count": runtime_status.owned_terminal_session_count,
                        "owned_terminal_session_keys": &runtime_status.owned_terminal_session_keys,
                        "terminal_session_count": runtime_status.terminal_session_count,
                        "terminal_session_keys": &runtime_status.terminal_session_keys,
                        "preserved_terminal_owner_count": runtime_status.preserved_terminal_owner_count,
                        "preserved_terminal_owner_keys": &runtime_status.preserved_terminal_owner_keys,
                        "preserved_owner_registry_retargeted": preserved_owner_registry_retargeted,
                    }),
                );
                ServerResponse::Ack {
                    message: Some(format!(
                        "hot restart prepared: {}",
                        daemon_executable.display()
                    )),
                }
            }
            ServerRequest::RetireDaemon { reason } => {
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "lifecycle",
                    "retire_daemon_requested",
                    serde_json::json!({
                        "reason": reason,
                        "server_version": SERVER_PROTOCOL_VERSION,
                        "server_build_id": current_build_id(),
                        "pid": std::process::id(),
                    }),
                );
                ServerResponse::Ack {
                    message: Some("daemon retire prepared".to_string()),
                }
            }
            ServerRequest::OpenStoredSession {
                session_kind,
                path,
                session_id,
                cwd,
                title_hint,
                view_mode,
            } => {
                let document = if session_kind == SessionKind::Document {
                    self.store.load_document(&path)?
                } else {
                    None
                };
                let live_order_before_open = self.server.live_session_order_keys().to_vec();
                self.server.open_or_focus_session(
                    session_kind,
                    &path,
                    session_id.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    document.as_ref(),
                );
                let mut opened_in_terminal = false;
                match view_mode.unwrap_or(WorkspaceViewMode::Rendered) {
                    WorkspaceViewMode::Rendered => {
                        self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    }
                    WorkspaceViewMode::Terminal => {
                        if self.server.active_session_supports_terminal() {
                            self.server.set_view_mode(WorkspaceViewMode::Terminal);
                            self.ensure_terminal_for_active()?;
                            opened_in_terminal = true;
                        } else {
                            self.server.set_view_mode(WorkspaceViewMode::Rendered);
                        }
                    }
                }
                // A row that newly ENTERED the live set lands at the
                // daemon-native position; restore its remembered ledger slot.
                // A row that was already live keeps the user's arrangement.
                if self
                    .server
                    .resolve_live_session_entry(&path)
                    .is_some_and(|(live_key, _)| {
                        !live_order_before_open.iter().any(|key| key == &live_key)
                    })
                {
                    self.apply_row_order_ledger_placement(&path, None);
                }
                self.persist()?;
                self.snapshot_response(Some(if opened_in_terminal {
                    format!("opened {path}")
                } else if session_kind == SessionKind::Document
                    && view_mode == Some(WorkspaceViewMode::Terminal)
                {
                    format!("opened {path} in preview")
                } else {
                    format!("opened {path}")
                }))
            }
            ServerRequest::ConnectSsh { target_ix } => {
                let (key, reused) = self.server.connect_ssh_target(target_ix)?;
                if key.is_some() && self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(key.map(|key| {
                    if reused {
                        format!("focused existing {key}")
                    } else {
                        format!("connected {key}")
                    }
                }))
            }
            ServerRequest::ConnectSshCustom { target, prefix } => {
                let (key, reused) = self.server.connect_ssh_custom(&target, prefix.as_deref())?;
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(if reused {
                    format!("focused existing {key}")
                } else {
                    format!("connected {key}")
                }))
            }
            ServerRequest::StartSshSession {
                target,
                prefix,
                cwd,
                title_hint,
                terminal_appearance,
                insert_after,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                let key = self.server.start_ssh_session(
                    &target,
                    prefix.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
                self.server
                    .place_live_session_after(&key, insert_after.as_deref());
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::StartRemoteCodexSession {
                target,
                prefix,
                cwd,
                title_hint,
                terminal_appearance,
                insert_after,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                let key = self.server.start_remote_codex_session(
                    &target,
                    prefix.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
                self.server
                    .place_live_session_after(&key, insert_after.as_deref());
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::StartRemoteClaudeSession {
                target,
                prefix,
                cwd,
                title_hint,
                terminal_appearance,
                insert_after,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                let key = self.server.start_remote_claude_session(
                    &target,
                    prefix.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
                self.server
                    .place_live_session_after(&key, insert_after.as_deref());
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::OpenRemoteSession {
                machine_key,
                session_id,
                cwd,
                title_hint,
                view_mode,
            } => {
                let live_order_before_open = self.server.live_session_order_keys().to_vec();
                let key = self.server.open_remote_scanned_session_with_view(
                    &machine_key,
                    &session_id,
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    view_mode,
                )?;
                let mut opened_in_terminal = false;
                if let Some(mode) = view_mode {
                    if mode == WorkspaceViewMode::Terminal
                        && !self.server.active_session_supports_terminal()
                    {
                        self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    } else {
                        self.server.set_view_mode(mode);
                    }
                    if mode == WorkspaceViewMode::Terminal
                        && self.server.active_session_supports_terminal()
                    {
                        self.ensure_terminal_for_active()?;
                        opened_in_terminal = true;
                    }
                }
                if !live_order_before_open.iter().any(|entry| entry == &key) {
                    self.apply_row_order_ledger_placement(&key, None);
                }
                self.persist()?;
                self.snapshot_response(Some(
                    if view_mode == Some(WorkspaceViewMode::Terminal) && !opened_in_terminal {
                        format!("opened {key} in preview")
                    } else {
                        format!("opened {key}")
                    },
                ))
            }
            ServerRequest::RefreshRemoteMachine { machine_key } => {
                self.server.refresh_remote_machine_by_key(&machine_key)?;
                self.persist()?;
                self.snapshot_response(Some(format!("refreshed {machine_key}")))
            }
            ServerRequest::RefreshManagedCli {
                machine_key,
                background,
            } => {
                let message = if background {
                    self.server
                        .queue_background_managed_cli_refresh(machine_key.as_deref())?
                } else {
                    self.server
                        .refresh_managed_cli(machine_key.as_deref(), background)?
                };
                ServerResponse::Ack {
                    message: Some(message),
                }
            }
            ServerRequest::RefreshPreview {
                path,
                full_remote_payload,
            } => {
                self.server
                    .refresh_session_preview_from_source_with_remote_payload(
                        &path,
                        full_remote_payload,
                    )?;
                self.persist()?;
                self.snapshot_response(Some(format!("refreshed preview {path}")))
            }
            ServerRequest::UpdateSessionCopy {
                path,
                title,
                precis,
                summary,
            } => {
                let remote_copy_target = self.server.remote_copy_target_for_session_path(&path);
                if let Some(title) = title.as_deref() {
                    self.server.set_session_title_hint(&path, title);
                }
                if let Some(precis) = precis.as_deref() {
                    self.server.set_session_precis_hint(&path, precis);
                }
                if let Some(summary) = summary.as_deref() {
                    self.server.set_session_summary_hint(&path, summary);
                }
                self.persist()?;
                if let Some((machine, session_id, cwd)) = remote_copy_target {
                    spawn_remote_generated_copy_persist(
                        machine, session_id, cwd, title, precis, summary, "manual",
                    );
                }
                ServerResponse::Ack { message: None }
            }
            ServerRequest::RemoveSshTarget { machine_key } => {
                let removed = self.server.remove_ssh_targets_for_machine(&machine_key);
                self.persist()?;
                self.snapshot_response(Some(if removed == 0 {
                    format!("no saved ssh target for {machine_key}")
                } else if removed == 1 {
                    format!("removed saved ssh target for {machine_key}")
                } else {
                    format!("removed {removed} saved ssh targets for {machine_key}")
                }))
            }
            ServerRequest::RemoveSession { path } => {
                if remove_session_should_detach_keep_alive_runtime(
                    self.server.live_session_keep_alive(&path),
                ) {
                    let detached = self.server.detach_live_session_view(&path)?;
                    self.persist()?;
                    append_trace_event(
                        self.store.home_dir(),
                        "daemon",
                        "session",
                        "keep_alive_live_session_close_detached",
                        serde_json::json!({
                            "path": path,
                            "detached": detached,
                        }),
                    );
                    return Ok(self.snapshot_response(Some(if detached {
                        format!("detached terminal viewport for {path}")
                    } else {
                        format!("no live session for {path}")
                    })));
                }
                let stop_command = self.server.terminal_stop_command(&path);
                let runtime_path = self.server.terminal_runtime_key_for_path(&path);
                let remote_target = self.server.remote_shutdown_target_for_path(&path);
                let removed_terminal = self
                    .terminals
                    .remove_session(&runtime_path, stop_command.as_deref())?;
                let removed_session = self.server.remove_live_session(&path)?;
                if removed_session || removed_terminal {
                    self.remove_preserved_owner_runtime(&runtime_path, "live_session_removed");
                }
                self.prune_unrepresented_preserved_owners("live_session_removed");
                self.persist()?;
                if let Some((machine, session_id)) = remote_target {
                    spawn_explicit_remote_session_shutdown(
                        self.store.home_dir(),
                        &path,
                        machine,
                        session_id,
                    );
                }
                self.snapshot_response(Some(if removed_terminal {
                    format!("closed terminal runtime for {path}")
                } else if removed_session {
                    format!("closed terminal metadata for {path}")
                } else {
                    format!("no live session for {path}")
                }))
            }
            ServerRequest::DropTerminalRuntime {
                runtime_key,
                reason,
            } => {
                let removed_terminal = self.terminals.remove_session(&runtime_key, None)?;
                append_trace_event(
                    self.store.home_dir(),
                    "daemon",
                    "hot_update",
                    "terminal_runtime_dropped",
                    serde_json::json!({
                        "runtime_key": runtime_key,
                        "reason": reason,
                        "removed_terminal": removed_terminal,
                        "server_version": SERVER_PROTOCOL_VERSION,
                        "server_build_id": current_build_id(),
                        "pid": std::process::id(),
                    }),
                );
                ServerResponse::Ack {
                    message: Some(if removed_terminal {
                        format!("dropped terminal runtime {runtime_key}")
                    } else {
                        format!("no terminal runtime for {runtime_key}")
                    }),
                }
            }
            ServerRequest::SetSessionKeepAlive { path, keep_alive } => {
                // Keep-alive is the session's declared persistence PREFERENCE,
                // not a statement that a terminal runtime currently exists. A
                // live session with no local runtime yet (a scanned remote
                // session the user never opened this GUI run) is a valid
                // keep-alive target — the snapshot layer already preserves that
                // exact state for restore (see
                // daemon_snapshot_preserves_keep_alive_remote_live_without_runtime_for_restore).
                // The old runtime gate refused those with a NON-ERROR ack, so a
                // bulk "Keep Alive (N sessions)" silently skipped every
                // unopened row (reported 2026-07-10: only the opened rows
                // turned green).
                let runtime_pending = keep_alive && {
                    let runtime_path = self.server.terminal_runtime_key_for_path(&path);
                    !self.terminals.has_session(&runtime_path)
                        && self
                            .preserved_owner_for_runtime_key(&runtime_path)
                            .is_none()
                };
                let updated = self.server.set_live_session_keep_alive(&path, keep_alive)?;
                self.prune_unrepresented_preserved_owners("keep_alive_updated");
                self.persist()?;
                self.snapshot_response(Some(if updated {
                    if keep_alive {
                        if runtime_pending {
                            format!(
                                "kept {path} alive (no terminal runtime yet; persistence applies when it attaches)"
                            )
                        } else {
                            format!("kept {path} alive")
                        }
                    } else {
                        format!("stopped keeping {path} alive")
                    }
                } else {
                    format!("no live session for {path}")
                }))
            }
            ServerRequest::ReorderLiveSessions {
                ordered_paths,
                client_scope,
            } => {
                let changed = self.server.replace_live_session_order(&ordered_paths);
                self.record_row_order_ledger(client_scope.as_deref());
                self.persist()?;
                self.snapshot_response(Some(if changed {
                    "reordered live sessions".to_string()
                } else {
                    "live session order unchanged".to_string()
                }))
            }
            ServerRequest::RowOrderLedgerReport { scope } => {
                let ledger = &self.row_order_ledger;
                let live_order = self.server.live_session_order_keys();
                let scope_report = |name: &str| {
                    serde_json::json!({
                        "scope": name,
                        "rows": ledger.scope_rows(name).iter().map(|row| {
                            serde_json::json!({
                                "session_path": row,
                                "live": live_order.iter().any(|entry| entry == row),
                            })
                        }).collect::<Vec<_>>(),
                    })
                };
                let report = match scope.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(name) => serde_json::json!({ "scopes": [scope_report(name)] }),
                    None => serde_json::json!({
                        "scopes": ledger
                            .scope_names()
                            .map(scope_report)
                            .collect::<Vec<_>>(),
                    }),
                };
                ServerResponse::Ack {
                    message: Some(report.to_string()),
                }
            }
            ServerRequest::StartLocalSession {
                session_kind,
                cwd,
                title_hint,
                terminal_appearance,
                insert_after,
            } => {
                // Phantom-spawn investigation: record that this birth was
                // GUI/IPC-initiated (vs an internal server-side creation) —
                // pairs with the live_session_birth chokepoint trace.
                if let Ok(home) = resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "session",
                        "start_local_session_request",
                        serde_json::json!({
                            "kind": format!("{session_kind:?}"),
                            "cwd": cwd,
                            "title_hint": title_hint,
                            "insert_after": insert_after,
                        }),
                    );
                }
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                let key = self.server.start_local_session(
                    session_kind,
                    cwd.as_deref(),
                    title_hint.as_deref(),
                );
                self.server
                    .place_live_session_after(&key, insert_after.as_deref());
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::SwitchAgentSessionMode { path, session_kind } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if self
                    .terminals
                    .recent_activity(&runtime_path, std::time::Duration::from_secs(4))
                {
                    bail!("session is still active; wait for it to settle before switching modes");
                }
                let stop_command = self.server.terminal_stop_command(&path);
                self.server.switch_agent_session_mode(&path, session_kind)?;
                if let Some((launch_command, cwd)) = self.server.terminal_spec(&path) {
                    if self.terminals.has_session(&runtime_path) {
                        self.terminals.restart_session(
                            &runtime_path,
                            &launch_command,
                            cwd.as_deref(),
                            stop_command.as_deref(),
                        )?;
                    }
                }
                self.persist()?;
                self.snapshot_response(Some(format!(
                    "switched to {}",
                    match session_kind {
                        SessionKind::Codex => "codex",
                        SessionKind::CodexLiteLlm => "codex-litellm",
                        SessionKind::ClaudeCode => "claude-code",
                        SessionKind::Shell => "shell",
                        SessionKind::SshShell => "ssh",
                        SessionKind::Document => "document",
                    }
                )))
            }
            ServerRequest::StartCommandSession {
                cwd,
                title_hint,
                launch_command,
                source_label,
                terminal_appearance,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                let key = self.server.start_command_session(
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    &launch_command,
                    source_label.as_deref(),
                );
                if self.server.active_session_supports_terminal() {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::EnsureRemoteRuntimeCodexSession {
                session_id,
                cwd,
                require_existing,
                terminal_appearance,
                initial_cols,
                initial_rows,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                let key = self.server.ensure_remote_runtime_codex_session(
                    &session_id,
                    cwd.as_deref(),
                    require_existing,
                    terminal_appearance.as_deref(),
                )?;
                self.adopt_legacy_local_codex_runtime(&session_id, &key);
                let _ = self.ensure_terminal_for_path_with_initial_size(
                    &key,
                    valid_initial_terminal_size(initial_cols, initial_rows),
                )?;
                self.persist()?;
                ServerResponse::Ack { message: Some(key) }
            }
            ServerRequest::StartRemoteRuntimeCodexSession {
                session_id,
                cwd,
                terminal_appearance,
                initial_cols,
                initial_rows,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                let key = self.server.start_remote_runtime_codex_session(
                    &session_id,
                    cwd.as_deref(),
                    terminal_appearance.as_deref(),
                )?;
                self.adopt_legacy_local_codex_runtime(&session_id, &key);
                let _ = self.ensure_terminal_for_path_with_initial_size(
                    &key,
                    valid_initial_terminal_size(initial_cols, initial_rows),
                )?;
                self.persist()?;
                ServerResponse::Ack { message: Some(key) }
            }
            ServerRequest::EnsureRemoteRuntimeCcSession {
                session_id,
                cwd,
                require_existing,
                terminal_appearance,
                initial_cols,
                initial_rows,
                claude_extra_args,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                sync_claude_extra_args_for_request(claude_extra_args.as_deref());
                let key = self.server.ensure_remote_runtime_cc_session(
                    &session_id,
                    cwd.as_deref(),
                    require_existing,
                    terminal_appearance.as_deref(),
                )?;
                self.adopt_legacy_local_codex_runtime(&session_id, &key);
                let _ = self.ensure_terminal_for_path_with_initial_size(
                    &key,
                    valid_initial_terminal_size(initial_cols, initial_rows),
                )?;
                self.persist()?;
                ServerResponse::Ack { message: Some(key) }
            }
            ServerRequest::StartRemoteRuntimeCcSession {
                session_id,
                cwd,
                terminal_appearance,
                initial_cols,
                initial_rows,
                claude_extra_args,
            } => {
                sync_terminal_identity_for_request(terminal_appearance.as_deref(), None);
                sync_claude_extra_args_for_request(claude_extra_args.as_deref());
                let key = self.server.start_remote_runtime_cc_session(
                    &session_id,
                    cwd.as_deref(),
                    terminal_appearance.as_deref(),
                )?;
                self.adopt_legacy_local_codex_runtime(&session_id, &key);
                let _ = self.ensure_terminal_for_path_with_initial_size(
                    &key,
                    valid_initial_terminal_size(initial_cols, initial_rows),
                )?;
                self.persist()?;
                ServerResponse::Ack { message: Some(key) }
            }
            ServerRequest::EnsureShellSession {
                session_id,
                cwd,
                initial_cols,
                initial_rows,
            } => {
                let key = self
                    .server
                    .ensure_shell_runtime_session(&session_id, cwd.as_deref())?;
                let _ = self.ensure_terminal_for_path_with_initial_size(
                    &key,
                    valid_initial_terminal_size(initial_cols, initial_rows),
                )?;
                self.persist()?;
                ServerResponse::Ack { message: Some(key) }
            }
            ServerRequest::FocusLive { key, view_mode } => {
                self.server.focus_live_session(&key);
                let mut focused_in_terminal = false;
                if let Some(mode) = view_mode {
                    if mode == WorkspaceViewMode::Terminal
                        && !self.server.active_session_supports_terminal()
                    {
                        self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    } else {
                        self.server.set_view_mode(mode);
                    }
                    if mode == WorkspaceViewMode::Terminal
                        && self.server.active_session_supports_terminal()
                    {
                        self.ensure_terminal_for_active()?;
                        focused_in_terminal = true;
                    }
                }
                self.persist()?;
                self.snapshot_response(Some(
                    if view_mode == Some(WorkspaceViewMode::Terminal) && !focused_in_terminal {
                        format!("focused {key} in preview")
                    } else {
                        format!("focused {key}")
                    },
                ))
            }
            ServerRequest::SetViewMode { mode } => {
                let mut effective_mode = mode;
                if mode == WorkspaceViewMode::Terminal
                    && !self.server.active_session_supports_terminal()
                {
                    self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    effective_mode = WorkspaceViewMode::Rendered;
                } else {
                    self.server.set_view_mode(mode);
                }
                if mode == WorkspaceViewMode::Terminal
                    && self.server.active_session_supports_terminal()
                {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(match effective_mode {
                    WorkspaceViewMode::Rendered => "preview mode".to_string(),
                    WorkspaceViewMode::Terminal => "terminal mode".to_string(),
                }))
            }
            ServerRequest::TogglePreviewBlock { block_ix } => {
                self.server.toggle_preview_block(block_ix);
                self.persist()?;
                self.snapshot_response(Some(format!("preview block {}", block_ix + 1)))
            }
            ServerRequest::SetAllPreviewBlocksFolded { folded } => {
                self.server.set_all_preview_blocks_folded(folded);
                self.persist()?;
                self.snapshot_response(Some(if folded {
                    "collapsed preview".to_string()
                } else {
                    "expanded preview".to_string()
                }))
            }
            ServerRequest::RequestTerminalLaunch => {
                self.ensure_terminal_for_active()?;
                self.server.set_view_mode(WorkspaceViewMode::Terminal);
                self.persist()?;
                self.snapshot_response(Some("requested terminal".to_string()))
            }
            ServerRequest::RequestTerminalLaunchForPath { path } => {
                self.server.request_terminal_launch_for_path(&path);
                self.ensure_terminal_for_active()?;
                self.server.set_view_mode(WorkspaceViewMode::Terminal);
                self.persist()?;
                self.snapshot_response(Some(format!("requested terminal for {path}")))
            }
            ServerRequest::TerminalEnsure { path } => {
                let message = self.ensure_terminal_for_path(&path)?;
                ServerResponse::Ack { message }
            }
            ServerRequest::TerminalRead { path, cursor } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_read(&owner_endpoint, &runtime_path, cursor) {
                        Ok((
                            cursor,
                            chunks,
                            running,
                            runtime_output_seen,
                            eof_without_output,
                            post_resize_output_seen,
                            last_resize_seq,
                            resync_required,
                        )) => {
                            return Ok(ServerResponse::TerminalStream {
                                cursor,
                                chunks,
                                running,
                                runtime_output_seen,
                                eof_without_output,
                                post_resize_output_seen,
                                last_resize_seq,
                                resync_required,
                            });
                        }
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            if self
                                .preserved_owner_endpoint_for_request(&runtime_path)
                                .is_some()
                            {
                                return Err(error);
                            }
                            let _ = self.ensure_terminal_for_path(&path)?;
                        }
                    }
                }
                if let Some(recovery_reason) =
                    terminal_read_runtime_recovery_reason(&path, &runtime_path, &self.terminals)
                    && let Some((stored_launch_command, cwd)) = self.server.terminal_spec(&path)
                {
                    // Runtime-owned terminals must stay on their stored daemon-owned launch path.
                    // Falling back to a legacy direct-attach command here can silently switch
                    // the recovery path back to the old semantics after startup.
                    let launch_command = stored_launch_command;
                    let stop_command = self.server.terminal_stop_command(&path);
                    if let Ok(home) = crate::resolve_yggterm_home() {
                        append_trace_event(
                            &home,
                            "server",
                            "terminal_runtime",
                            "restart_before_read",
                            serde_json::json!({
                                "path": path,
                                "runtime_path": runtime_path,
                                "reason": recovery_reason,
                            }),
                        );
                    }
                    self.terminals.restart_session(
                        &runtime_path,
                        &launch_command,
                        cwd.as_deref(),
                        stop_command.as_deref(),
                    )?;
                }
                let stream = self.terminals.read(&runtime_path, cursor)?;
                ServerResponse::TerminalStream {
                    cursor: stream.cursor,
                    chunks: stream
                        .chunks
                        .into_iter()
                        .map(|chunk| TerminalStreamChunk {
                            seq: chunk.seq,
                            data: chunk.data,
                        })
                        .collect(),
                    running: stream.running,
                    runtime_output_seen: stream.runtime_output_seen,
                    eof_without_output: stream.eof_without_output,
                    post_resize_output_seen: stream.post_resize_output_seen,
                    last_resize_seq: stream.last_resize_seq,
                    resync_required: stream.resync_required,
                }
            }
            ServerRequest::TerminalSnapshot { path } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    if self.reject_preserved_owner_saved_session_mismatch(
                        &path,
                        &runtime_path,
                        &owner_endpoint,
                        "terminal_snapshot_saved_session_mismatch",
                    ) {
                        let _ = self.ensure_terminal_for_path(&path)?;
                    } else {
                        match terminal_snapshot(&owner_endpoint, &runtime_path) {
                            Ok((
                                text,
                                running,
                                runtime_output_seen,
                                post_resize_output_seen,
                                last_resize_seq,
                                runtime_spawn_id,
                            )) => {
                                return Ok(ServerResponse::TerminalSnapshot {
                                    text,
                                    running,
                                    runtime_output_seen,
                                    post_resize_output_seen,
                                    last_resize_seq,
                                    runtime_spawn_id,
                                });
                            }
                            Err(error) => {
                                self.handle_preserved_owner_request_error(
                                    &runtime_path,
                                    &owner_endpoint,
                                    &error,
                                );
                                return Err(error);
                            }
                        }
                    }
                }
                let text = self
                    .terminals
                    .session_screen_snapshot(&runtime_path)
                    .unwrap_or_default();
                ServerResponse::TerminalSnapshot {
                    text,
                    running: self.terminals.session_is_running(&runtime_path),
                    runtime_output_seen: self.terminals.session_has_runtime_output(&runtime_path),
                    post_resize_output_seen: self
                        .terminals
                        .session_post_resize_output_seen(&runtime_path),
                    last_resize_seq: self.terminals.session_last_resize_seq(&runtime_path),
                    runtime_spawn_id: self.terminals.session_runtime_spawn_id(&runtime_path),
                }
            }
            ServerRequest::TerminalRetainedSnapshot { path } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    if self.reject_preserved_owner_saved_session_mismatch(
                        &path,
                        &runtime_path,
                        &owner_endpoint,
                        "terminal_retained_snapshot_saved_session_mismatch",
                    ) {
                        let _ = self.ensure_terminal_for_path(&path)?;
                    } else {
                        match terminal_retained_snapshot(&owner_endpoint, &runtime_path) {
                            Ok((
                                text,
                                running,
                                runtime_output_seen,
                                post_resize_output_seen,
                                last_resize_seq,
                                runtime_spawn_id,
                            )) => {
                                return Ok(ServerResponse::TerminalRetainedSnapshot {
                                    text,
                                    running,
                                    runtime_output_seen,
                                    post_resize_output_seen,
                                    last_resize_seq,
                                    runtime_spawn_id,
                                });
                            }
                            Err(error) => {
                                self.handle_preserved_owner_request_error(
                                    &runtime_path,
                                    &owner_endpoint,
                                    &error,
                                );
                                return Err(error);
                            }
                        }
                    }
                }
                let text = self
                    .terminals
                    .session_snapshot(&runtime_path)
                    .unwrap_or_default();
                ServerResponse::TerminalRetainedSnapshot {
                    text,
                    running: self.terminals.session_is_running(&runtime_path),
                    runtime_output_seen: self.terminals.session_has_runtime_output(&runtime_path),
                    post_resize_output_seen: self
                        .terminals
                        .session_post_resize_output_seen(&runtime_path),
                    last_resize_seq: self.terminals.session_last_resize_seq(&runtime_path),
                    runtime_spawn_id: self.terminals.session_runtime_spawn_id(&runtime_path),
                }
            }
            ServerRequest::TerminalHistory { path } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_history(&owner_endpoint, &runtime_path) {
                        Ok((rows, running)) => {
                            return Ok(ServerResponse::TerminalHistory { rows, running });
                        }
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            return Err(error);
                        }
                    }
                }
                let rows = self
                    .terminals
                    .session_history_rows(&runtime_path)
                    .unwrap_or_default();
                ServerResponse::TerminalHistory {
                    rows,
                    running: self.terminals.session_is_running(&runtime_path),
                }
            }
            ServerRequest::TerminalWrite { path, data } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_write(&owner_endpoint, &runtime_path, &data) {
                        Ok(_) => return Ok(ServerResponse::Ack { message: None }),
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            // Entry kept => the owner is healthy and the
                            // failure is this request's to surface. Entry
                            // REMOVED => fall through: the local/adopt/
                            // relaunch path below is now the only route that
                            // can serve this key, and erroring here is what
                            // printed "terminal session not found" into the
                            // PTY and left the session untypeable after a
                            // daemon handoff (the manual-restart-only state).
                            if self
                                .preserved_owner_for_runtime_key(&runtime_path)
                                .is_some()
                            {
                                return Err(error);
                            }
                        }
                    }
                }
                let local_runtime_running = self.terminals.session_is_running(&runtime_path);
                let write_strategy = terminal_write_strategy_for_path(&path, local_runtime_running);
                if matches!(write_strategy, TerminalWriteStrategy::LocalRuntime) {
                    return self.write_local_terminal_with_lost_runtime_recovery(
                        &path,
                        &runtime_path,
                        &data,
                    );
                }
                if matches!(write_strategy, TerminalWriteStrategy::RemoteDirectFallback) {
                    match self.server.remote_terminal_write_for_path(&path, &data) {
                        Ok(true) => {
                            return Ok(ServerResponse::Ack { message: None });
                        }
                        Ok(false) => {}
                        Err(error) => {
                            if let Ok(home) = resolve_yggterm_home() {
                                append_trace_event(
                                    &home,
                                    "daemon",
                                    "terminal_io",
                                    "remote_terminal_write_direct_failed",
                                    serde_json::json!({
                                        "path": path,
                                        "runtime_path": runtime_path,
                                        "error": format!("{error:#}"),
                                        "bytes": data.len(),
                                    }),
                                );
                            }
                            return Err(error);
                        }
                    }
                }
                match self.server.remote_terminal_write_for_path(&path, &data) {
                    Ok(true) => {
                        return Ok(ServerResponse::Ack { message: None });
                    }
                    Ok(false) => {}
                    Err(error) => {
                        if let Ok(home) = resolve_yggterm_home() {
                            append_trace_event(
                                &home,
                                "daemon",
                                "terminal_io",
                                "remote_terminal_write_direct_failed_fallback_local_pty",
                                serde_json::json!({
                                    "path": path,
                                    "runtime_path": runtime_path,
                                    "error": format!("{error:#}"),
                                    "bytes": data.len(),
                                }),
                            );
                        }
                    }
                }
                return self.write_local_terminal_with_lost_runtime_recovery(
                    &path,
                    &runtime_path,
                    &data,
                );
            }
            ServerRequest::TerminalResize { path, cols, rows } => {
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                if let Some(owner_endpoint) =
                    self.preserved_owner_endpoint_for_request(&runtime_path)
                {
                    match terminal_resize(&owner_endpoint, &runtime_path, cols, rows) {
                        Ok(_) => return Ok(ServerResponse::Ack { message: None }),
                        Err(error) => {
                            self.handle_preserved_owner_request_error(
                                &runtime_path,
                                &owner_endpoint,
                                &error,
                            );
                            return Err(error);
                        }
                    }
                }
                self.terminals.resize(&runtime_path, cols, rows)?;
                // XTERM-BUG: squish-and-bottom-paint-on-reresume — persist the grid so a
                // later daemon-process restart re-resumes EVERY session at its real grid
                // (not DEFAULT 120x36). FLUSH SYNCHRONOUSLY on change: relying on the next
                // periodic persist cycle lost the grid when the daemon was killed/handed-off
                // (auto-update, deploy) between the resize and the flush → the successor
                // defaulted to 120x36 → squish. Only flush when the grid actually changed
                // (avoids drag-resize write storms). See campaign D1.
                if self.server.record_session_pty_grid(&path, cols, rows) {
                    let _ = self.persist_state_only();
                }
                // Run #19: pin the remote daemon's PTY explicitly too — the
                // implicit SIGWINCH→ssh→bridge chain is not reliable.
                self.forward_remote_pty_resize(&path, cols, rows);
                ServerResponse::Ack { message: None }
            }
            ServerRequest::TerminalRestart {
                path,
                terminal_appearance,
                force_remote,
                initial_cols,
                initial_rows,
            } => {
                if let Some(terminal_appearance) = terminal_appearance.as_deref() {
                    sync_terminal_identity_for_request(Some(terminal_appearance), None);
                }
                let recovered_remote_scan = self
                    .server
                    .recover_remote_scanned_terminal_for_restart(&path)?;
                let launch_refreshed = self
                    .server
                    .refresh_terminal_identity_launch_command_for_path(&path);
                let runtime_path = self.terminal_runtime_key_for_path(&path);
                let (launch_command, cwd) = self
                    .server
                    .terminal_spec(&path)
                    .with_context(|| format!("terminal session not found: {path}"))?;
                let stop_command = self.server.terminal_stop_command(&path);
                if force_remote {
                    if let Some((machine, session_id)) =
                        self.server.remote_shutdown_target_for_path(&path)
                    {
                        terminate_remote_codex_session(&machine, &session_id).with_context(
                            || {
                                format!(
                                    "terminating remote codex session {session_id} before restart"
                                )
                            },
                        )?;
                    }
                    if self
                        .preserved_owner_endpoint_for_request(&runtime_path)
                        .is_some()
                    {
                        self.remove_preserved_owner_runtime(&runtime_path, "terminal_restart");
                    }
                }
                // Run #19: like the ensure path, a restart without a client
                // grid falls back to the session's PERSISTED grid before the
                // 120×36 default — the remote restore/bootstrap restart was
                // the squish entry point (practice daemon restart spawned
                // codex at DEFAULT while the client rendered 159×63).
                let initial_size = initial_cols
                    .zip(initial_rows)
                    .or_else(|| self.server.session_pty_grid(&path));
                self.terminals.restart_session_with_size(
                    &runtime_path,
                    &launch_command,
                    cwd.as_deref(),
                    stop_command.as_deref(),
                    initial_size,
                )?;
                // RECORD-ON-CREATE: persist the client-supplied restart grid so a later
                // re-resume recreates at the real size (the persist() below flushes it).
                if let Some((cols, rows)) = initial_size {
                    self.server.record_session_pty_grid(&path, cols, rows);
                }
                self.prune_duplicate_legacy_owned_runtime_sessions("terminal_restart");
                self.persist()?;
                if force_remote {
                    spawn_force_remote_restart_daemon_cleanup(
                        &self.store,
                        self.preserved_terminal_owners.entries.is_empty(),
                    );
                }
                self.snapshot_response(Some(format!(
                    "restarted {path}; launch_refreshed={launch_refreshed}; recovered_remote_scan={recovered_remote_scan}; force_remote={force_remote}"
                )))
            }
            ServerRequest::SyncExternalWindow => {
                let message = self.server.sync_external_terminal_window_for_active();
                self.persist()?;
                self.snapshot_response(Some(message))
            }
            ServerRequest::RaiseExternalWindow => {
                let message = self.server.raise_external_terminal_window_for_active();
                self.persist()?;
                self.snapshot_response(Some(message))
            }
            ServerRequest::SyncTheme { theme } => {
                self.server.sync_theme(theme);
                self.persist()?;
                self.snapshot_response(Some("theme synced".to_string()))
            }
            ServerRequest::SyncTerminalIdentity {
                terminal_appearance,
                terminal_profile,
            } => {
                sync_terminal_identity_for_request(
                    Some(&terminal_appearance),
                    terminal_profile.as_ref(),
                );
                let refreshed = self.server.refresh_terminal_identity_launch_commands();
                if refreshed > 0 {
                    self.persist()?;
                }
                ServerResponse::Ack {
                    message: Some(format!(
                        "terminal identity synced; refreshed {refreshed} launch commands"
                    )),
                }
            }
            ServerRequest::Shutdown => {
                let remote_targets = self.server.remote_shutdown_targets();
                let mut remote_errors = Vec::new();
                let mut remote_stopped = 0usize;
                for (machine, session_id) in remote_targets {
                    match terminate_remote_codex_session(&machine, &session_id) {
                        Ok(()) => remote_stopped += 1,
                        Err(error) => remote_errors
                            .push(format!("{}:{}: {}", machine.machine_key, session_id, error)),
                    }
                }
                let summary = self
                    .terminals
                    .shutdown_all(|path| self.server.terminal_stop_command(path));
                self.persist()?;
                let total_errors = summary.errors.len() + remote_errors.len();
                ServerResponse::Ack {
                    message: Some(if total_errors == 0 {
                        format!(
                            "stopped {} terminal sessions and {} remote persistent sessions",
                            summary.stopped, remote_stopped
                        )
                    } else {
                        format!(
                            "stopped {} terminal sessions and {} remote persistent sessions, {} errors",
                            summary.stopped, remote_stopped, total_errors
                        )
                    }),
                }
            }
        };
        if trace_request {
            append_trace_event(
                self.store.home_dir(),
                "daemon",
                "request",
                "end",
                serde_json::json!({ "request": request_name }),
            );
        }
        Ok(response)
    }
}

fn sync_terminal_identity_for_request(
    terminal_appearance: Option<&str>,
    terminal_profile: Option<&TerminalIdentityColorProfile>,
) {
    if let Some(appearance) = terminal_appearance
        .map(str::trim)
        .filter(|appearance| !appearance.is_empty())
    {
        sync_terminal_identity_appearance_with_profile(appearance, terminal_profile);
    }
}

/// Same process-wide-env pattern as terminal identity: the CC daemon-runtime
/// requests carry the CLIENT's configured claude extra args, which the launch
/// builder reads via YGGTERM_CC_EXTRA_ARGS when spawning `claude`.
fn sync_claude_extra_args_for_request(claude_extra_args: Option<&str>) {
    if let Some(args) = claude_extra_args
        .map(str::trim)
        .filter(|args| !args.is_empty())
    {
        unsafe {
            std::env::set_var(crate::codex_cli::ENV_YGGTERM_CC_EXTRA_ARGS, args);
        }
    }
}

fn snapshot_session_requires_terminal_runtime(session: &SnapshotSessionView) -> bool {
    matches!(
        session.source,
        crate::SessionSource::LiveLocal | crate::SessionSource::LiveSsh
    ) && session.kind != SessionKind::Document
}

fn snapshot_session_is_pending_runtime_launch(session: &SnapshotSessionView) -> bool {
    snapshot_session_requires_terminal_runtime(session)
        && matches!(
            session.launch_phase,
            crate::TerminalLaunchPhase::Queued
                | crate::TerminalLaunchPhase::BridgePending
                | crate::TerminalLaunchPhase::RemoteBootstrap
        )
}

fn snapshot_session_is_keep_alive_recovery_target(session: &SnapshotSessionView) -> bool {
    snapshot_session_requires_terminal_runtime(session)
        && session
            .metadata
            .iter()
            .any(|entry| entry.label == "Runtime Persistence" && entry.value == "keep-alive")
}

/// Per the first-class agent-session spec: an agent CLI session's row
/// re-derives from the agent CLI's own store (codex / Claude Code JSONL), so a
/// runtime exit must never erase the row from the snapshot — the row stays and
/// the next open re-resumes via the CLI. This covers BOTH local agents
/// (re-derive from the local store) AND remote agents (re-derive from the remote
/// store via resume-codex / resume-cc): recognizing only the LOCAL agent here
/// stripped every non-keep-alive REMOTE agent from the Live Sessions snapshot
/// whenever its runtime was not currently connected (live-caught on jojo
/// 2026-07-08). Plain shells stay runtime-gated here: a non-keep-alive shell
/// with no live PTY is a husk, so it is retained only when its runtime IS
/// present — never as a recovery target. (Run #16 gate-#5 family: runtime exit
/// was the pre-swap row eraser for local codex rows.)
fn snapshot_session_is_agent_store_recoverable(session: &SnapshotSessionView) -> bool {
    // Source alone is not enough: restored/recovery-created rows carry
    // source=Stored while holding a live local runtime key (the 2.8.79
    // persistence lesson, live-recaught here at the 90→91 swap when restored
    // local rows vanished from the snapshot while sitting in the persisted
    // order). Mirror managed_live_session_is_recoverable: a local-keyed row
    // is a live row by construction.
    let is_local_agent = (matches!(session.source, crate::SessionSource::LiveLocal)
        || crate::local_runtime_id_from_key(&session.session_path).is_some())
        && matches!(
            session.kind,
            SessionKind::Codex | SessionKind::CodexLiteLlm | SessionKind::ClaudeCode
        );
    is_local_agent || crate::session_path_is_remote_agent(&session.session_path)
}

fn apply_terminal_runtime_truth_to_snapshot(
    server: &YggtermServer,
    runtime_keys: &HashSet<String>,
    snapshot: &mut ServerUiSnapshot,
) {
    if let Some(active_session) = snapshot.active_session.as_mut() {
        let runtime_owned = runtime_keys
            .contains(&server.terminal_runtime_key_for_path(&active_session.session_path));
        if !runtime_owned
            && (snapshot_session_is_keep_alive_recovery_target(active_session)
                || snapshot_session_is_agent_store_recoverable(active_session))
        {
            active_session.launch_phase = crate::TerminalLaunchPhase::RemoteBootstrap;
        } else if runtime_owned && snapshot_session_is_pending_runtime_launch(active_session) {
            // Runtime truth outranks a stale stored phase in BOTH directions: a
            // session whose PTY the daemon owns is running, no matter what label
            // the open path last wrote. A stuck RemoteBootstrap on an owned
            // session made every GUI post-snapshot rearm treat the healthy
            // active session as a pending launch and cold-remount it (the
            // sudden-blank-viewport loop, live-caught on jojo 2026-07-09).
            active_session.launch_phase = crate::TerminalLaunchPhase::Running;
        }
    }
    for session in &mut snapshot.live_sessions {
        let runtime_owned =
            runtime_keys.contains(&server.terminal_runtime_key_for_path(&session.session_path));
        if !runtime_owned
            && (snapshot_session_is_keep_alive_recovery_target(session)
                || snapshot_session_is_agent_store_recoverable(session))
        {
            session.launch_phase = crate::TerminalLaunchPhase::RemoteBootstrap;
        } else if runtime_owned && snapshot_session_is_pending_runtime_launch(session) {
            session.launch_phase = crate::TerminalLaunchPhase::Running;
        }
    }
    let active_path = snapshot.active_session_path.clone().or_else(|| {
        snapshot
            .active_session
            .as_ref()
            .map(|session| session.session_path.clone())
    });
    snapshot.live_sessions.retain(|session| {
        runtime_keys.contains(&server.terminal_runtime_key_for_path(&session.session_path))
            || (active_path.as_deref() == Some(session.session_path.as_str())
                && snapshot_session_is_pending_runtime_launch(session))
            || snapshot_session_is_keep_alive_recovery_target(session)
            || snapshot_session_is_agent_store_recoverable(session)
    });

    let Some(active_path) = active_path else {
        return;
    };
    let active_runtime_present =
        runtime_keys.contains(&server.terminal_runtime_key_for_path(&active_path));
    let active_requires_runtime = snapshot
        .active_session
        .as_ref()
        .is_some_and(snapshot_session_requires_terminal_runtime)
        || snapshot.active_view_mode == WorkspaceViewMode::Terminal;
    let active_pending_runtime_launch = snapshot
        .active_session
        .as_ref()
        .is_some_and(snapshot_session_is_pending_runtime_launch);
    let active_keep_alive_recovery_target = snapshot
        .active_session
        .as_ref()
        .is_some_and(snapshot_session_is_keep_alive_recovery_target);
    if !active_requires_runtime
        || active_runtime_present
        || active_pending_runtime_launch
        || active_keep_alive_recovery_target
    {
        return;
    }

    if let Some(preview) = server.remote_preview_snapshot_session_for_path(&active_path) {
        snapshot.active_session_path = Some(preview.session_path.clone());
        snapshot.active_session = Some(preview);
    } else {
        snapshot.active_session_path = None;
        snapshot.active_session = None;
    }
    snapshot.active_view_mode = WorkspaceViewMode::Rendered;
}

fn unrepresented_preserved_owner_runtime_keys(
    server: &YggtermServer,
    owner_registry_keys: &HashSet<String>,
    all_registry_keys: &HashSet<String>,
    current_owned_runtime_keys: &HashSet<String>,
    owner_runtime_keys: Vec<String>,
) -> Vec<String> {
    let mut stale_runtime_keys = owner_runtime_keys
        .into_iter()
        .filter(|runtime_key| !owner_registry_keys.contains(runtime_key))
        .filter(|runtime_key| {
            current_owned_runtime_keys.contains(runtime_key)
                || all_registry_keys.contains(runtime_key)
                || !server.represents_terminal_runtime_key(runtime_key)
        })
        .collect::<Vec<_>>();
    stale_runtime_keys.sort();
    stale_runtime_keys.dedup();
    stale_runtime_keys
}

fn daemon_status_version_triplet(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let patch = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    (major, minor, patch)
}

fn duplicate_legacy_owned_runtime_keys(
    current_runtime_keys: &HashSet<String>,
    current_version: &str,
    current_build_id: u64,
    current_pid: u32,
    owner_status: &ServerRuntimeStatus,
) -> Vec<String> {
    if current_runtime_keys.is_empty()
        || owner_status.server_pid == current_pid
        || owner_status.owned_terminal_session_keys.is_empty()
    {
        return Vec::new();
    }
    let owner_rank = (
        daemon_status_version_triplet(&owner_status.server_version),
        owner_status.server_build_id,
        owner_status.server_pid,
    );
    let current_rank = (
        daemon_status_version_triplet(current_version),
        current_build_id,
        current_pid,
    );
    if owner_rank >= current_rank {
        return Vec::new();
    }
    let mut duplicate_runtime_keys = owner_status
        .owned_terminal_session_keys
        .iter()
        .filter(|runtime_key| current_runtime_keys.contains(runtime_key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    duplicate_runtime_keys.sort();
    duplicate_runtime_keys.dedup();
    duplicate_runtime_keys
}

/// A preserved owner can only SERVE a runtime it actually OWNS. Trusting the
/// represented set (`terminal_session_keys`) let hollow chains survive: a
/// predecessor that merely re-preserves a key toward an even older, dead
/// daemon still LISTS the key but cannot serve a single byte of it. Live-
/// caught 2026-07-17: 8 registry entries pointed at a 2.11.1 daemon that
/// owned exactly 1 of its 15 represented keys, pinning
/// `hot_update_handoff_active` true for four days.
fn preserved_owner_status_serves_runtime_key(
    status: &ServerRuntimeStatus,
    runtime_key: &str,
) -> bool {
    status
        .owned_terminal_session_keys
        .iter()
        .any(|key| key == runtime_key)
}

fn preserved_owner_candidate_for_runtime_key(
    statuses: Vec<(ServerEndpoint, ServerRuntimeStatus)>,
    runtime_key: &str,
    current_pid: u32,
) -> Option<(ServerEndpoint, ServerRuntimeStatus)> {
    statuses
        .into_iter()
        .filter(|(_endpoint, status)| status.server_pid != current_pid)
        .filter(|(_endpoint, status)| {
            preserved_owner_status_serves_runtime_key(status, runtime_key)
        })
        .max_by_key(|(_endpoint, status)| {
            (
                daemon_status_version_triplet(&status.server_version),
                status.server_build_id,
                status.server_pid,
            )
        })
}

fn trim_terminal_buffers(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    idle_trim_after_ms: u64,
) -> Result<TerminalTrimTick> {
    let runtime = lock_daemon_runtime(runtime, "trim_terminal_buffers");
    let before = runtime.terminals.stats();
    let trim = runtime
        .terminals
        .trim_idle_buffers(std::time::Duration::from_millis(idle_trim_after_ms));
    let after = runtime.terminals.stats();
    Ok(TerminalTrimTick {
        before,
        after,
        trimmed_sessions: trim.trimmed_sessions,
        reclaimed_bytes: trim.reclaimed_bytes,
    })
}

#[derive(Debug, Clone, Default)]
struct TerminalTrimTick {
    before: TerminalBufferStats,
    after: TerminalBufferStats,
    trimmed_sessions: usize,
    reclaimed_bytes: usize,
}

fn source_updated_at_for_path(path: &Path) -> Option<OffsetDateTime> {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|dur| OffsetDateTime::from_unix_timestamp(dur.as_secs() as i64).ok())
}

fn source_updated_at_for_remote_epoch(epoch: i64) -> Option<OffsetDateTime> {
    (epoch > 0)
        .then(|| OffsetDateTime::from_unix_timestamp(epoch).ok())
        .flatten()
}

fn background_machine_key(raw_machine_key: &str) -> String {
    match raw_machine_key.trim().to_ascii_lowercase().as_str() {
        "juju" | "jujo" => "jojo".to_string(),
        value => value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string(),
    }
}

fn collect_local_copy_candidates(node: &SessionNode, out: &mut Vec<BackgroundCopyCandidate>) {
    if node.kind == SessionNodeKind::CodexSession {
        if let (Some(session_id), Some(cwd)) = (node.session_id.as_ref(), node.cwd.as_ref()) {
            out.push(BackgroundCopyCandidate {
                session_path: node.path.to_string_lossy().to_string(),
                session_id: session_id.clone(),
                cwd: cwd.clone(),
                title: node.title.clone().unwrap_or_else(|| node.name.clone()),
                source_updated_at: source_updated_at_for_path(&node.path),
                remote_machine: None,
                generation_context: None,
                storage_path: None,
                cached_summary: None,
                live_local_agent: false,
            });
        }
    }
    for child in &node.children {
        collect_local_copy_candidates(child, out);
    }
}

fn session_cwd_for_background_copy(session: &ManagedSessionView) -> String {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == "Cwd")
        .map(|entry| entry.value.clone())
        .unwrap_or_default()
}

fn collect_live_copy_candidates(
    store: &SessionStore,
    live_sessions: &[ManagedSessionView],
    working_paths: &HashSet<String>,
    out: &mut Vec<BackgroundCopyCandidate>,
) {
    for session in live_sessions {
        if session.session_path.starts_with("remote-session://") {
            continue;
        }
        // Working-indicator trigger ([[spec-title-summary-working-indicator]]):
        // a LIVE agent session becomes a title/summary candidate only while it
        // is (recently) WORKING — generation rides real turns instead of
        // scanning idle sessions every tick. Documents have no PTY and keep
        // the old path.
        if session.kind != SessionKind::Document
            && !working_paths.contains(&session.session_path)
        {
            continue;
        }
        let generation_context = if session.kind == SessionKind::Document {
            store
                .load_document(&session.session_path)
                .ok()
                .flatten()
                .and_then(|document| {
                    let trimmed = document.body.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                })
        } else {
            None
        };
        if generation_context.is_none()
            && !(Path::new(&session.session_path).exists()
                && session.session_path.ends_with(".jsonl"))
        {
            // User bug 5: a yggterm-spawned LOCAL codex session has a
            // `local://<uuid>` key (not a .jsonl path) so it never became a
            // title candidate — its cwd-derived launch-hint title never
            // updated. The rebind poll records the CLI's on-disk JSONL in the
            // "Storage" metadata; once it exists the session can be titled
            // from the real transcript.
            let storage = if matches!(
                session.kind,
                SessionKind::Codex | SessionKind::CodexLiteLlm
            ) {
                session
                    .metadata
                    .iter()
                    .find(|entry| entry.label == "Storage")
                    .map(|entry| entry.value.clone())
                    .filter(|value| {
                        value.ends_with(".jsonl") && Path::new(value).exists()
                    })
            } else {
                None
            };
            let Some(storage) = storage else {
                continue;
            };
            // Summary timelines: the JSONL mtime drives the short-horizon
            // refresh gate so a working session's summary evolves (and the
            // timeline grows) as the transcript does.
            let source_updated_at = source_updated_at_for_path(Path::new(&storage));
            out.push(BackgroundCopyCandidate {
                session_path: session.session_path.clone(),
                session_id: session.id.clone(),
                cwd: session_cwd_for_background_copy(session),
                title: session.title.clone(),
                source_updated_at,
                remote_machine: None,
                generation_context: None,
                storage_path: Some(storage),
                cached_summary: None,
                live_local_agent: true,
            });
            continue;
        }
        out.push(BackgroundCopyCandidate {
            session_path: session.session_path.clone(),
            session_id: session.id.clone(),
            cwd: session_cwd_for_background_copy(session),
            title: session.title.clone(),
            source_updated_at: None,
            remote_machine: None,
            generation_context,
            storage_path: None,
            cached_summary: None,
            live_local_agent: false,
        });
    }
}

fn collect_remote_copy_candidates(
    remote_machines: &[RemoteMachineSnapshot],
) -> Vec<BackgroundCopyCandidate> {
    let mut out = Vec::new();
    for machine in remote_machines {
        let machine_ref = RemoteMachineSnapshot {
            machine_key: machine.machine_key.clone(),
            label: machine.label.clone(),
            ssh_target: machine.ssh_target.clone(),
            prefix: machine.prefix.clone(),
            remote_binary_expr: machine.remote_binary_expr.clone(),
            remote_deploy_state: machine.remote_deploy_state,
            health: machine.health,
            sessions: Vec::new(),
        };
        for session in &machine.sessions {
            out.push(BackgroundCopyCandidate {
                session_path: session.session_path.clone(),
                session_id: session.session_id.clone(),
                cwd: session.cwd.clone(),
                title: session.title_hint.clone(),
                source_updated_at: source_updated_at_for_remote_epoch(session.modified_epoch),
                remote_machine: Some(machine_ref.clone()),
                generation_context: (!session.recent_context.trim().is_empty())
                    .then(|| session.recent_context.clone()),
                storage_path: (!session.storage_path.trim().is_empty())
                    .then(|| session.storage_path.clone()),
                cached_summary: session.cached_summary.clone(),
                live_local_agent: false,
            });
        }
    }
    out
}

/// User bug 4/5 (Claude Code half): CC maintains its OWN title (ai-title /
/// custom-title records in its JSONL) — per [[spec-codex-cc-title-summary]]
/// yggterm respects it rather than LLM-generating one. The scanned cwd-tree
/// rows already read it, but a LIVE local CC session kept its launch-hint
/// title forever. Sync the live row's title from the CC JSONL whenever CC's
/// title exists and differs (yggterm renames are written back into the JSONL,
/// so the JSONL stays the single source of truth).
fn collect_live_cc_title_syncs(
    live_sessions: &[ManagedSessionView],
    working_paths: &HashSet<String>,
) -> Vec<BackgroundCopyUpdate> {
    let mut updates = Vec::new();
    for session in live_sessions {
        // local:// AND cc-runtime:// both live on THIS machine (the host
        // daemon that owns the PTY also owns ~/.claude/projects), so both
        // sync from the local JSONL. remote-cc:// rows belong to another
        // machine and ride collect_remote_cc_title_syncs instead.
        if session.kind != SessionKind::ClaudeCode
            || !(session.session_path.starts_with("local://")
                || session.session_path.starts_with("cc-runtime://"))
        {
            continue;
        }
        // Spec: pick up CC's own title (incl. /rename mid-session) on WORKING
        // turns — a rename necessarily makes the session active+working, so
        // polling here is complete coverage with zero idle cost.
        if !working_paths.contains(&session.session_path) {
            continue;
        }
        // The session id equals CC's own rollout uuid because a fresh CC session
        // is launched with `--session-id <uuid>` (build_live_session) and a
        // resumed one with `--resume <uuid>`, so this id-keyed lookup finds the
        // JSONL on the very first turn. (A bare `claude` would mint its own uuid
        // → this lookup fails → stuck launch hint; live-caught drift 2026-07-01.)
        let Some(jsonl) = local_cc_session_jsonl_path(&session.id) else {
            continue;
        };
        let Some(title) = read_cc_session_title(&jsonl).ok().flatten() else {
            continue;
        };
        let title = title.trim().to_string();
        if !title.is_empty() && title != session.title {
            if let Ok(home) = crate::resolve_yggterm_home() {
                append_trace_event(
                    &home,
                    "daemon",
                    "title_trigger",
                    "cc_title_pickup",
                    serde_json::json!({
                        "session_path": session.session_path,
                        "previous_title": session.title,
                        "new_title": title,
                    }),
                );
            }
            updates.push(BackgroundCopyUpdate {
                session_path: session.session_path.clone(),
                title: Some(title),
                summary: None,
            });
        }
    }
    updates
}

/// Reads CC titles for a given list of session ids on the remote machine.
/// Head (512 KB) catches the early `ai-title`; the tail window catches a
/// late `custom-title` (a /rename appends at the END of a large JSONL, past
/// any head cap). Latest record wins, custom-title over ai-title — the same
/// precedence CC itself displays.
const REMOTE_CC_TITLE_SCRIPT: &str = r#"
import json, os, sys
from pathlib import Path
HEAD_BYTES = 512 * 1024
TAIL_BYTES = 128 * 1024

def titles_from_lines(lines):
    custom = None
    ai = None
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            r = json.loads(line)
        except Exception:
            continue
        t = r.get('type', '')
        if t == 'custom-title':
            ct = (r.get('customTitle') or '').strip()
            if ct:
                custom = ct
        elif t == 'ai-title':
            at = (r.get('aiTitle') or '').strip()
            if at:
                ai = at
    return custom, ai

ids = [i for i in sys.argv[1:] if i.strip()]
projects = Path(os.path.expanduser('~/.claude/projects'))
if not projects.exists() or not ids:
    sys.exit(0)
for sid in ids:
    found = None
    for project_dir in projects.iterdir():
        if not project_dir.is_dir():
            continue
        candidate = project_dir / (sid + '.jsonl')
        if candidate.is_file():
            found = candidate
            break
    if found is None:
        continue
    try:
        size = found.stat().st_size
        with open(found, encoding='utf-8', errors='ignore') as f:
            head = f.read(HEAD_BYTES).splitlines()
        tail = []
        if size > HEAD_BYTES:
            with open(found, 'rb') as f:
                f.seek(max(0, size - TAIL_BYTES))
                raw = f.read().decode('utf-8', errors='ignore')
            # First chunk is likely a partial line; drop it.
            tail = raw.splitlines()[1:]
    except Exception:
        continue
    custom_h, ai_h = titles_from_lines(head)
    custom_t, ai_t = titles_from_lines(tail)
    title = custom_t or custom_h or ai_t or ai_h
    if title:
        print(json.dumps({'session_id': sid, 'title': title}, ensure_ascii=False))
"#;

/// Pure selection of which live `remote-cc://` rows to poll for a title this
/// tick: rows on a WORKING turn (renames and ai-titles land during turns)
/// plus — once per daemon lifetime — every row whose title was never
/// confirmed against the remote JSONL (heals stale launch-hint titles left
/// over from restores). Returns (machine_key, session_id, session_path).
fn remote_cc_title_poll_paths(
    live_sessions: &[ManagedSessionView],
    working_paths: &HashSet<String>,
    confirmed_paths: &HashSet<String>,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for session in live_sessions {
        if session.kind != SessionKind::ClaudeCode {
            continue;
        }
        let Some((machine_key, session_id)) =
            crate::parse_remote_cc_session_path(&session.session_path)
        else {
            continue;
        };
        if !working_paths.contains(&session.session_path)
            && confirmed_paths.contains(&session.session_path)
        {
            continue;
        }
        out.push((
            machine_key.to_string(),
            session_id.to_string(),
            session.session_path.clone(),
        ));
    }
    out
}

/// Remote half of the CC title sync ([[spec-codex-cc-title-summary]]): live
/// `remote-cc://machine/uuid` rows read their title from the remote machine's
/// CC JSONL — the single source of truth (CC writes ai-title/custom-title
/// there, and yggterm renames are appended there too). One ssh per machine
/// per tick, only for the rows `remote_cc_title_poll_paths` selects.
fn collect_remote_cc_title_syncs(
    live_sessions: &[ManagedSessionView],
    working_paths: &HashSet<String>,
    ssh_targets: &[SshConnectTarget],
    confirmed_paths: &mut HashSet<String>,
) -> Vec<BackgroundCopyUpdate> {
    let targets = remote_cc_title_poll_paths(live_sessions, working_paths, confirmed_paths);
    if targets.is_empty() {
        return Vec::new();
    }
    let mut by_machine: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (machine_key, session_id, session_path) in targets {
        by_machine
            .entry(machine_key)
            .or_default()
            .push((session_id, session_path));
    }
    let mut updates = Vec::new();
    for (machine_key, rows) in by_machine {
        let Some(target) = ssh_targets
            .iter()
            .find(|target| background_machine_key(&target.label) == machine_key)
        else {
            continue;
        };
        let ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();
        let lines = match crate::run_remote_python_lines(
            &target.ssh_target,
            target.prefix.as_deref(),
            REMOTE_CC_TITLE_SCRIPT,
            &ids,
        ) {
            Ok(lines) => lines,
            // ssh failed: leave the rows unconfirmed so the next tick retries
            // (the chore's idle backoff bounds the retry rate).
            Err(_) => continue,
        };
        let mut titles: HashMap<String, String> = HashMap::new();
        for line in &lines {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line)
                && let (Some(id), Some(title)) = (
                    value.get("session_id").and_then(|v| v.as_str()),
                    value.get("title").and_then(|v| v.as_str()),
                )
            {
                titles.insert(id.to_string(), title.trim().to_string());
            }
        }
        for (session_id, session_path) in rows {
            confirmed_paths.insert(session_path.clone());
            let Some(title) = titles.get(&session_id).filter(|title| !title.is_empty()) else {
                continue;
            };
            let current = live_sessions
                .iter()
                .find(|session| session.session_path == session_path)
                .map(|session| session.title.as_str())
                .unwrap_or("");
            if title.as_str() != current {
                if let Ok(home) = crate::resolve_yggterm_home() {
                    append_trace_event(
                        &home,
                        "daemon",
                        "title_trigger",
                        "remote_cc_title_pickup",
                        serde_json::json!({
                            "session_path": session_path,
                            "machine_key": machine_key,
                            "previous_title": current,
                            "new_title": title,
                        }),
                    );
                }
                updates.push(BackgroundCopyUpdate {
                    session_path,
                    title: Some(title.clone()),
                    summary: None,
                });
            }
        }
    }
    updates
}

/// Whether the background copy chore should (re)generate a title.
/// Live local agent sessions (user bug 5) carry a cwd-derived launch-hint
/// title the fallback recognizer can't enumerate, so for them the resolver
/// DB alone is the freshness gate — manual renames are saved there and stay
/// protected. Everything else keeps the historical double gate.
fn background_copy_title_missing(
    live_local_agent: bool,
    candidate_title: &str,
    stored_title: Option<&str>,
) -> bool {
    let stored_missing = stored_title.is_none_or(looks_like_generated_fallback_title);
    if live_local_agent {
        stored_missing
    } else {
        looks_like_generated_fallback_title(candidate_title) && stored_missing
    }
}

fn copy_target_context(
    candidate: &BackgroundCopyCandidate,
    ssh_targets: &[SshConnectTarget],
) -> Result<Option<String>> {
    if let Some(context) = candidate.generation_context.as_ref()
        && !context.trim().is_empty()
    {
        return Ok(Some(context.clone()));
    }
    let (Some(machine), Some(storage_path)) = (
        candidate.remote_machine.as_ref(),
        candidate.storage_path.as_deref(),
    ) else {
        return Ok(None);
    };
    let Some(target) = ssh_targets
        .iter()
        .find(|target| background_machine_key(&target.label) == machine.machine_key)
    else {
        return Ok(None);
    };
    fetch_remote_generation_context(target, storage_path).map(Some)
}

fn background_copy_error_is_rate_limit(error: &anyhow::Error) -> bool {
    let rate_limited = format!("{error:#}").contains("429");
    if rate_limited && let Ok(home) = crate::resolve_yggterm_home() {
        append_trace_event(
            &home,
            "daemon",
            "title_trigger",
            "rate_limited_backoff",
            serde_json::json!({ "error": format!("{error:#}") }),
        );
    }
    rate_limited
}

fn build_background_copy_updates(
    store: &SessionStore,
    settings: &AppSettings,
    local_root: &SessionNode,
    live_sessions: &[ManagedSessionView],
    remote_machines: &[RemoteMachineSnapshot],
    ssh_targets: &[SshConnectTarget],
    working_paths: &HashSet<String>,
    generation_enabled: bool,
) -> Result<Vec<BackgroundCopyUpdate>> {
    let mut updates = collect_live_cc_title_syncs(live_sessions, working_paths);
    // LLM title/summary generation is opt-in (env-gated); the CC title sync
    // above is a cheap local-file read and always runs — CC's JSONL is the
    // SSOT for CC titles and needs no LLM.
    if !generation_enabled {
        return Ok(updates);
    }
    let mut candidates = Vec::new();
    collect_local_copy_candidates(local_root, &mut candidates);
    collect_live_copy_candidates(store, live_sessions, working_paths, &mut candidates);
    candidates.extend(collect_remote_copy_candidates(remote_machines));

    let mut seen_candidates = HashSet::new();
    // Endpoint pacing: the litellm endpoint 429s under quick successive calls
    // (live-observed 2026-06-11), and each generation can be 2 LLM calls
    // (title+summary). Cap generations per tick; on a rate-limit error stop
    // generating for the rest of the tick (the chore retries in ~12s).
    let mut llm_generations_this_tick = 0usize;
    let mut rate_limited_this_tick = false;
    for candidate in candidates
        .into_iter()
        .filter(|candidate| seen_candidates.insert(candidate.session_path.clone()))
        .take(BACKGROUND_COPY_BUDGET_PER_TICK)
    {
        let stored_title = store
            .resolve_title_for_session_id(&candidate.session_id)
            .ok()
            .flatten();
        let title_missing = background_copy_title_missing(
            candidate.live_local_agent,
            &candidate.title,
            stored_title.as_deref(),
        );
        let stored_summary = store
            .resolve_summary_for_session_id(&candidate.session_id)
            .ok()
            .flatten();
        let summary_needs_refresh = candidate
            .source_updated_at
            .and_then(|updated_at| {
                if candidate.live_local_agent {
                    store
                        .summary_needs_refresh_for_live_session_id(&candidate.session_id, updated_at)
                        .ok()
                } else {
                    store
                        .summary_needs_refresh_for_session_id(&candidate.session_id, updated_at)
                        .ok()
                }
            })
            .unwrap_or(stored_summary.is_none());
        let summary_missing = summary_needs_refresh
            && candidate
                .cached_summary
                .as_deref()
                .is_none_or(|summary| summary.trim().is_empty());
        // Timeline refresh: an EXISTING live summary that aged past the live
        // horizon must regenerate — the generator returns the stored summary
        // unless forced (put_summary's timeline insert preserves history;
        // delete_summary leaves the timeline intact).
        let summary_force_refresh =
            candidate.live_local_agent && summary_needs_refresh && stored_summary.is_some();
        if !title_missing && !summary_missing {
            continue;
        }

        if rate_limited_this_tick || llm_generations_this_tick >= BACKGROUND_COPY_LLM_GENERATIONS_PER_TICK {
            continue;
        }
        llm_generations_this_tick += 1;
        if let Ok(home) = crate::resolve_yggterm_home() {
            append_trace_event(
                &home,
                "daemon",
                "title_trigger",
                "generation_begin",
                serde_json::json!({
                    "session_path": candidate.session_path,
                    "title_missing": title_missing,
                    "summary_missing": summary_missing,
                    "live_local_agent": candidate.live_local_agent,
                    "generation_index_this_tick": llm_generations_this_tick,
                }),
            );
        }

        let maybe_context = copy_target_context(&candidate, ssh_targets)?;
        let (title, summary) = if let Some(context) = maybe_context {
            let title = if title_missing {
                match store.generate_title_for_context(
                    settings,
                    &candidate.session_id,
                    &candidate.cwd,
                    &context,
                    false,
                ) {
                    Ok(title) => title,
                    Err(error) => {
                        rate_limited_this_tick |= background_copy_error_is_rate_limit(&error);
                        None
                    }
                }
            } else {
                None
            };
            let summary = if summary_missing && !rate_limited_this_tick {
                match store.generate_summary_for_context(
                    settings,
                    &candidate.session_id,
                    &candidate.cwd,
                    &context,
                    summary_force_refresh,
                ) {
                    Ok(summary) => summary,
                    Err(error) => {
                        rate_limited_this_tick |= background_copy_error_is_rate_limit(&error);
                        None
                    }
                }
            } else {
                None
            };
            if let Some(machine) = candidate.remote_machine.as_ref() {
                if title.is_some() || summary.is_some() {
                    persist_remote_generated_copy(
                        machine,
                        &candidate.session_id,
                        &candidate.cwd,
                        title.as_deref(),
                        None,
                        summary.as_deref(),
                        &settings.interface_llm_model,
                    )?;
                }
            }
            (title, summary)
        } else {
            // A live local agent candidate's session_path is the live
            // `local://` key; the readable transcript is its Storage JSONL.
            let source_path = candidate
                .storage_path
                .as_deref()
                .filter(|_| candidate.remote_machine.is_none())
                .unwrap_or(&candidate.session_path);
            let title = if title_missing {
                match store.generate_title_for_session_path(settings, source_path, false) {
                    Ok(title) => title,
                    Err(error) => {
                        rate_limited_this_tick |= background_copy_error_is_rate_limit(&error);
                        None
                    }
                }
            } else {
                None
            };
            let summary = if summary_missing && !rate_limited_this_tick {
                match store.generate_summary_for_session_path(settings, source_path, summary_force_refresh) {
                    Ok(summary) => summary,
                    Err(error) => {
                        rate_limited_this_tick |= background_copy_error_is_rate_limit(&error);
                        None
                    }
                }
            } else {
                None
            };
            (title, summary)
        };

        if title.is_some() || summary.is_some() {
            updates.push(BackgroundCopyUpdate {
                session_path: candidate.session_path,
                title,
                summary,
            });
        }
    }
    Ok(updates)
}

fn run_background_copy_chore(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    generation_enabled: bool,
    remote_cc_confirmed: &mut HashSet<String>,
) -> Result<usize> {
    let (store, settings, local_root, live_sessions, remote_machines, ssh_targets, perf_home, working_paths) = {
        let runtime = lock_daemon_runtime(runtime, "run_background_copy_chore_read");
        let settings = runtime.store.load_settings().unwrap_or_default();
        // Eventual propagation of a GUI profiling toggle into the daemon's gate.
        yggterm_core::set_perf_profiling_enabled(settings.perf_profiling_enabled);
        let local_root = runtime.store.load_codex_tree(&settings)?;
        // Working-indicator trigger: a live session counts as "working" when
        // its vt100 screen shows the agent working (esc-to-interrupt SSOT) or
        // its PTY produced output within the recent window — generation rides
        // real turns, never idle sessions.
        let working_paths: HashSet<String> = runtime
            .server
            .live_sessions()
            .iter()
            .filter(|session| {
                let runtime_path = runtime.terminal_runtime_key_for_path(&session.session_path);
                let screen_working = runtime
                    .terminals
                    .session_screen_snapshot(&runtime_path)
                    .is_some_and(|screen| {
                        yggterm_core::screen_text_shows_agent_working(&screen)
                    });
                let recently_active = runtime
                    .terminals
                    .session_idle_for_ms(&runtime_path)
                    .is_some_and(|idle_ms| idle_ms < BACKGROUND_COPY_WORKING_RECENT_MS);
                screen_working || recently_active
            })
            .map(|session| session.session_path.clone())
            .collect();
        (
            runtime.store.clone(),
            settings,
            local_root,
            runtime.server.live_sessions().to_vec(),
            runtime.server.remote_machines().to_vec(),
            runtime.server.ssh_targets().to_vec(),
            runtime.store.home_dir().to_path_buf(),
            working_paths,
        )
    };
    let perf = PerfSpan::start(&perf_home, "daemon", "background_copy_chore");
    let mut updates = build_background_copy_updates(
        &store,
        &settings,
        &local_root,
        &live_sessions,
        &remote_machines,
        &ssh_targets,
        &working_paths,
        generation_enabled,
    )?;
    updates.extend(collect_remote_cc_title_syncs(
        &live_sessions,
        &working_paths,
        &ssh_targets,
        remote_cc_confirmed,
    ));
    perf.finish(serde_json::json!({
        "updates": updates.len(),
        "live_sessions": live_sessions.len(),
        "remote_machines": remote_machines.len(),
    }));

    if updates.is_empty() {
        return Ok(0);
    }

    let mut runtime = lock_daemon_runtime(runtime, "run_background_copy_chore_apply");
    for update in &updates {
        if let Some(title) = update.title.as_deref() {
            runtime
                .server
                .set_session_title_hint(&update.session_path, title);
        }
        if let Some(summary) = update.summary.as_deref() {
            runtime
                .server
                .set_session_summary_hint(&update.session_path, summary);
        }
    }
    runtime.persist()?;
    append_trace_event(
        runtime.store.home_dir(),
        "daemon",
        "background_copy",
        "tick",
        serde_json::json!({ "updates": updates.len() }),
    );
    Ok(updates.len())
}

fn remote_codex_identity_poll_enabled() -> bool {
    // Default ON; an explicit truthy disable env opts out.
    !daemon_background_copy_chore_enabled_from_env(
        std::env::var(ENV_YGGTERM_DISABLE_REMOTE_CODEX_IDENTITY_POLL)
            .ok()
            .as_deref(),
    )
}

fn normalize_cwd_for_identity_match(cwd: &str) -> String {
    let trimmed = cwd.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Matches the Codex processes running on one machine to the live remote-Codex
/// rows that need rebinding, pairing by normalized cwd. Pure (no IO) so the
/// matching policy is unit-testable. A running identity is paired only when its
/// real session id differs from the row's synthesized id and has not already
/// been claimed by an earlier row at the same cwd, so two rows in the same cwd
/// never collapse onto one transcript.
fn match_codex_identities_to_targets(
    group: &[crate::RemoteCodexIdentityPollTarget],
    identities: &[crate::LocalAgentCliIdentity],
) -> Vec<(String, CodexRuntimeProcessIdentity)> {
    let mut by_cwd: HashMap<String, Vec<&crate::LocalAgentCliIdentity>> = HashMap::new();
    for identity in identities.iter().filter(|identity| identity.kind == "codex") {
        by_cwd
            .entry(normalize_cwd_for_identity_match(&identity.cwd))
            .or_default()
            .push(identity);
    }
    let mut claimed: HashSet<String> = HashSet::new();
    let mut rebinds = Vec::new();
    for target in group {
        let Some(candidates) = by_cwd.get(&normalize_cwd_for_identity_match(&target.cwd)) else {
            continue;
        };
        let Some(identity) = candidates.iter().find(|identity| {
            identity.session_id != target.current_id && !claimed.contains(&identity.session_id)
        }) else {
            continue;
        };
        claimed.insert(identity.session_id.clone());
        rebinds.push((
            target.key.clone(),
            CodexRuntimeProcessIdentity {
                storage_path: PathBuf::from(&identity.storage_path),
                session_id: identity.session_id.clone(),
                cwd: identity.cwd.clone(),
            },
        ));
    }
    rebinds
}

/// One tick of the remote-Codex identity poll
/// (`[[finding-uuidv4-codex-session-drift]]` Stage 2). For each live remote
/// Codex row still carrying a synthesized UUIDv4 id, SSH-queries the owning
/// machine for its running Codex processes, matches by cwd, and rebinds the row
/// to the real CLI session id through the same SSOT path the local rebind uses
/// (`apply_codex_runtime_identity_to_live_session`). Self-limiting: a row drops
/// out as soon as it is rebound, and `attempts` abandons rows that never match.
/// Returns the number of rows rebound this tick.
fn run_remote_codex_identity_poll_chore(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    attempts: &mut HashMap<String, u32>,
) -> Result<usize> {
    let (targets, yggterm_home) = {
        let runtime = lock_daemon_runtime(runtime, "remote_codex_identity_poll_read");
        (
            runtime
                .server
                .live_remote_codex_sessions_needing_identity_poll(),
            resolve_yggterm_home().ok(),
        )
    };

    // Forget attempt counters for rows that no longer need polling.
    let live_keys: HashSet<String> = targets.iter().map(|target| target.key.clone()).collect();
    attempts.retain(|key, _| live_keys.contains(key));

    // Skip rows that have exhausted their attempt budget (un-rebindable).
    let targets: Vec<crate::RemoteCodexIdentityPollTarget> = targets
        .into_iter()
        .filter(|target| {
            attempts.get(&target.key).copied().unwrap_or(0)
                < REMOTE_CODEX_IDENTITY_POLL_MAX_ATTEMPTS
        })
        .collect();
    if targets.is_empty() {
        return Ok(0);
    }

    // Query each machine once.
    let mut machines: HashMap<(String, Option<String>), Vec<crate::RemoteCodexIdentityPollTarget>> =
        HashMap::new();
    for target in targets {
        machines
            .entry((target.ssh_target.clone(), target.ssh_prefix.clone()))
            .or_default()
            .push(target);
    }

    let mut rebinds: Vec<(String, CodexRuntimeProcessIdentity)> = Vec::new();
    for ((ssh_target, ssh_prefix), group) in &machines {
        // Count the attempt for every row we are about to poll, regardless of outcome.
        for target in group {
            *attempts.entry(target.key.clone()).or_insert(0) += 1;
        }
        let identities =
            match poll_remote_local_codex_identities(ssh_target, ssh_prefix.as_deref()) {
                Ok(identities) => identities,
                Err(error) => {
                    warn!(ssh_target = %ssh_target, error = %error, "remote codex identity poll failed");
                    continue;
                }
            };
        rebinds.extend(match_codex_identities_to_targets(group, &identities));
    }

    if rebinds.is_empty() {
        return Ok(0);
    }

    let mut applied = 0usize;
    let mut runtime = lock_daemon_runtime(runtime, "remote_codex_identity_poll_apply");
    for (key, identity) in &rebinds {
        if runtime.server.apply_codex_runtime_identity_to_live_session(
            key,
            identity,
            yggterm_home.as_deref(),
        ) {
            applied += 1;
            attempts.remove(key);
            append_trace_event(
                runtime.store.home_dir(),
                "daemon",
                "persistence",
                "remote_codex_runtime_identity_refreshed",
                serde_json::json!({
                    "session_path": key,
                    "codex_session_id": identity.session_id,
                    "storage_path": identity.storage_path.display().to_string(),
                    "cwd": identity.cwd,
                }),
            );
        }
    }
    if applied > 0 {
        runtime.persist()?;
    }
    Ok(applied)
}

fn daemon_background_copy_chore_enabled_from_env(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn daemon_background_copy_chore_enabled() -> bool {
    daemon_background_copy_chore_enabled_from_env(
        std::env::var(ENV_YGGTERM_ENABLE_BACKGROUND_COPY_CHORE)
            .ok()
            .as_deref(),
    )
}

fn daemon_idle_shutdown_ms() -> u64 {
    std::env::var("YGGTERM_DAEMON_IDLE_SHUTDOWN_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_DAEMON_IDLE_SHUTDOWN_MS)
}

fn daemon_terminal_idle_trim_after_ms() -> u64 {
    std::env::var("YGGTERM_DAEMON_TERMINAL_IDLE_TRIM_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TERMINAL_IDLE_TRIM_AFTER_MS)
}

fn current_millis_u64() -> u64 {
    u64::try_from(current_millis()).unwrap_or(u64::MAX)
}

fn terminal_ensure_should_seed_remote_snapshot(path: &str, seed_remote_snapshot: bool) -> bool {
    path.starts_with("remote-session://") && seed_remote_snapshot
}

fn startup_prewarm_should_seed_remote_snapshot(
    path: &str,
    active_terminal_path: Option<&str>,
) -> bool {
    path.starts_with("remote-session://") && active_terminal_path == Some(path)
}

fn mark_daemon_activity(last_activity_ms: &AtomicU64) {
    last_activity_ms.store(current_millis_u64(), Ordering::Relaxed);
}

/// Parse a dotted version string ("2.8.6") into a comparable triple. A missing
/// patch component is treated as 0.
pub(crate) fn parse_daemon_version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.trim().parse::<u64>().ok()?;
    let minor = parts.next()?.trim().parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").trim().parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// Per [[bug-class-old-daemon-never-retires]]: in the managed-versions install
/// model every daemon binary lives at its own path, so the `(deleted)` exe poll
/// never fires and stale daemons never retire. Detect supersession structurally:
/// a strictly-newer daemon is reachable on another versioned socket. Only sockets
/// whose parsed version is HIGHER than ours are probed (so a busy machine with
/// many dead lower-version socket files stays cheap), and the probe confirms the
/// peer actually reports a newer version before we treat ourselves as superseded.
#[cfg(unix)]
fn daemon_is_superseded(home_dir: &Path) -> bool {
    let Some(my_version) = parse_daemon_version_triple(SERVER_PROTOCOL_VERSION) else {
        return false;
    };
    let own_socket = match default_endpoint(home_dir) {
        ServerEndpoint::UnixSocket(path) => fs::canonicalize(&path).ok(),
        #[allow(unreachable_patterns)]
        _ => None,
    };
    let Ok(entries) = fs::read_dir(home_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(socket_version) = parse_versioned_server_socket_name(&path) else {
            continue;
        };
        if socket_version <= my_version {
            continue;
        }
        if let Some(own) = own_socket.as_ref()
            && fs::canonicalize(&path).ok().as_ref() == Some(own)
        {
            continue;
        }
        if let Ok(status) = status(&ServerEndpoint::UnixSocket(path))
            && parse_daemon_version_triple(&status.server_version)
                .is_some_and(|peer| peer > my_version)
        {
            return true;
        }
    }
    false
}

fn daemon_should_idle_shutdown(
    home_dir: &Path,
    endpoint: &ServerEndpoint,
    last_activity_ms: &AtomicU64,
    idle_shutdown_ms: u64,
    terminal_session_count: usize,
) -> bool {
    // Never retire a daemon that still owns live sessions (including preserved
    // hot-update PTYs, which are counted here) — there is no fd-handoff, so a
    // shutdown would interrupt them.
    if terminal_session_count > 0 {
        return false;
    }
    let idle_for_ms = current_millis_u64().saturating_sub(last_activity_ms.load(Ordering::Relaxed));
    if idle_for_ms < idle_shutdown_ms {
        return false;
    }
    let clients_empty = match active_client_instance_records(home_dir, endpoint) {
        Ok(records) => records.is_empty(),
        Err(error) => {
            warn!(error=%error, "failed to read active client instances during daemon idle check");
            return false;
        }
    };
    if clients_empty {
        return true;
    }
    // Clients exist, but a client only ever connects to the socket matching its
    // own version. If a strictly-newer daemon is alive, those client records
    // belong to IT, not to us — and we own zero sessions — so this stale daemon
    // is safe to retire even though client records exist. This is what lets a
    // superseded daemon drain and exit in the managed-versions install model,
    // where the `(deleted)` exe poll never triggers. Per
    // [[bug-class-old-daemon-never-retires]].
    #[cfg(unix)]
    {
        if daemon_is_superseded(home_dir) {
            return true;
        }
    }
    false
}

#[cfg(unix)]
fn wait_for_listener_ready(fd: i32, timeout_ms: u64) -> Result<bool> {
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout_ms = i32::try_from(timeout_ms).unwrap_or(i32::MAX);
    loop {
        // SAFETY: `pollfd` points to valid memory for one file descriptor descriptor.
        let rc = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if rc > 0 {
            return Ok(true);
        }
        if rc == 0 {
            return Ok(false);
        }
        let err = std::io::Error::last_os_error();
        if err.kind() == ErrorKind::Interrupted {
            continue;
        }
        return Err(err).context("polling daemon listener");
    }
}

fn server_request_name(request: &ServerRequest) -> &'static str {
    match request {
        ServerRequest::Ping => "ping",
        ServerRequest::Status => "status",
        ServerRequest::WorkingFlags => "working_flags",
        ServerRequest::Snapshot => "snapshot",
        ServerRequest::PrepareUpdateRestart => "prepare_update_restart",
        ServerRequest::PrepareClientClose => "prepare_client_close",
        ServerRequest::HotRestart { .. } => "hot_restart",
        ServerRequest::RetireDaemon { .. } => "retire_daemon",
        ServerRequest::OpenStoredSession { .. } => "open_stored_session",
        ServerRequest::ConnectSsh { .. } => "connect_ssh",
        ServerRequest::ConnectSshCustom { .. } => "connect_ssh_custom",
        ServerRequest::StartSshSession { .. } => "start_ssh_session",
        ServerRequest::StartRemoteCodexSession { .. } => "start_remote_codex_session",
        ServerRequest::StartRemoteClaudeSession { .. } => "start_remote_claude_session",
        ServerRequest::OpenRemoteSession { .. } => "open_remote_session",
        ServerRequest::RefreshRemoteMachine { .. } => "refresh_remote_machine",
        ServerRequest::RefreshManagedCli { .. } => "refresh_managed_cli",
        ServerRequest::RefreshPreview { .. } => "refresh_preview",
        ServerRequest::UpdateSessionCopy { .. } => "update_session_copy",
        ServerRequest::RemoveSshTarget { .. } => "remove_ssh_target",
        ServerRequest::RemoveSession { .. } => "remove_session",
        ServerRequest::DropTerminalRuntime { .. } => "drop_terminal_runtime",
        ServerRequest::SetSessionKeepAlive { .. } => "set_session_keep_alive",
        ServerRequest::ReorderLiveSessions { .. } => "reorder_live_sessions",
        ServerRequest::RowOrderLedgerReport { .. } => "row_order_ledger_report",
        ServerRequest::StartLocalSession { .. } => "start_local_session",
        ServerRequest::SwitchAgentSessionMode { .. } => "switch_agent_session_mode",
        ServerRequest::StartCommandSession { .. } => "start_command_session",
        ServerRequest::EnsureRemoteRuntimeCodexSession { .. } => {
            "ensure_remote_runtime_codex_session"
        }
        ServerRequest::StartRemoteRuntimeCodexSession { .. } => {
            "start_remote_runtime_codex_session"
        }
        ServerRequest::EnsureRemoteRuntimeCcSession { .. } => "ensure_remote_runtime_cc_session",
        ServerRequest::StartRemoteRuntimeCcSession { .. } => "start_remote_runtime_cc_session",
        ServerRequest::EnsureShellSession { .. } => "ensure_shell_session",
        ServerRequest::FocusLive { .. } => "focus_live",
        ServerRequest::SetViewMode { .. } => "set_view_mode",
        ServerRequest::TogglePreviewBlock { .. } => "toggle_preview_block",
        ServerRequest::SetAllPreviewBlocksFolded { .. } => "set_all_preview_blocks_folded",
        ServerRequest::RequestTerminalLaunch => "request_terminal_launch",
        ServerRequest::RequestTerminalLaunchForPath { .. } => "request_terminal_launch_for_path",
        ServerRequest::TerminalEnsure { .. } => "terminal_ensure",
        ServerRequest::TerminalRead { .. } => "terminal_read",
        ServerRequest::TerminalSnapshot { .. } => "terminal_snapshot",
        ServerRequest::TerminalRetainedSnapshot { .. } => "terminal_retained_snapshot",
        ServerRequest::TerminalHistory { .. } => "terminal_history",
        ServerRequest::TerminalWrite { .. } => "terminal_write",
        ServerRequest::TerminalResize { .. } => "terminal_resize",
        ServerRequest::TerminalRestart { .. } => "terminal_restart",
        ServerRequest::SyncExternalWindow => "sync_external_window",
        ServerRequest::RaiseExternalWindow => "raise_external_window",
        ServerRequest::SyncTheme { .. } => "sync_theme",
        ServerRequest::SyncTerminalIdentity { .. } => "sync_terminal_identity",
        ServerRequest::Shutdown => "shutdown",
    }
}

fn current_build_id() -> u64 {
    std::env::current_exe()
        .ok()
        .and_then(|path| fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok())
        .and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs())
        .unwrap_or_default()
}

/// Epoch-ms at which THIS daemon process started. Forced during `run_daemon` so it is
/// the process's real age, not the age of whenever something first asked for status.
static DAEMON_STARTED_AT_MS: LazyLock<u64> = LazyLock::new(current_millis_u64);

/// Build id (exe mtime) of the binary this process was LAUNCHED from, captured at boot.
///
/// `current_build_id()` re-stats the executable PATH, so once a deploy overwrites the
/// binary in place the two diverge — and that divergence is precisely "a newer build is
/// sitting on disk waiting for a hot-restart". That state was previously invisible: on
/// jojo 2026-07-11 the daemon ran 2.10.3 for 19h44m while 2.10.13 sat on disk, and
/// nothing in the product said so. Surfacing it is the point.
static DAEMON_RUNNING_BUILD_ID: LazyLock<u64> = LazyLock::new(current_build_id);

#[cfg(unix)]
struct DaemonSocketLock {
    file: fs::File,
}

#[cfg(unix)]
impl Drop for DaemonSocketLock {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(unix)]
fn daemon_socket_lock_path(socket_path: &Path) -> PathBuf {
    let file_name = socket_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("server.sock");
    socket_path.with_file_name(format!("{file_name}.lock"))
}

#[cfg(unix)]
fn try_acquire_daemon_socket_lock(
    socket_path: &Path,
    home_dir: &Path,
) -> Result<Option<DaemonSocketLock>> {
    let lock_path = daemon_socket_lock_path(socket_path);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating daemon lock dir {}", parent.display()))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening daemon socket lock {}", lock_path.display()))?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Some(DaemonSocketLock { file }));
    }
    let error = std::io::Error::last_os_error();
    let raw_os_error = error.raw_os_error();
    if raw_os_error == Some(libc::EWOULDBLOCK) || raw_os_error == Some(libc::EAGAIN) {
        append_trace_event(
            home_dir,
            "daemon",
            "lifecycle",
            "bind_lock_busy",
            serde_json::json!({
                "socket": socket_path.display().to_string(),
                "lock": lock_path.display().to_string(),
            }),
        );
        return Ok(None);
    }
    Err(error).with_context(|| format!("locking daemon socket {}", lock_path.display()))
}

pub fn default_endpoint(home_dir: &Path) -> ServerEndpoint {
    #[cfg(unix)]
    {
        let socket_name = format!("server-{}.sock", SERVER_PROTOCOL_VERSION.replace('.', "-"));
        let home_socket = home_dir.join(&socket_name);
        let socket_path = if unix_socket_path_fits_platform(&home_socket) {
            home_socket
        } else {
            short_unix_socket_path_for_home(home_dir, &socket_name)
        };
        ServerEndpoint::UnixSocket(socket_path)
    }

    #[cfg(not(unix))]
    {
        let _ = home_dir;
        ServerEndpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: versioned_tcp_port(),
        }
    }
}

/// Which daemon a CLIENT (the GUI) should connect to, plus whether it had to
/// fall back across a version boundary.
#[derive(Debug, Clone)]
pub struct ClientDaemonEndpoint {
    pub endpoint: ServerEndpoint,
    /// `Some((client_version, daemon_version))` when the client's OWN-version
    /// socket was unreachable and we fell back to a reachable older daemon.
    /// The caller should surface this loudly (it means "deploy the matching
    /// daemon"); both strings are dotted (e.g. `"2.9.17"`).
    pub version_mismatch: Option<(String, String)>,
}

fn format_socket_version((major, minor, patch): (u64, u64, u64)) -> String {
    format!("{major}.{minor}.{patch}")
}

/// Pure fallback policy: among version-tagged daemon sockets, the version a
/// NEWER client should drop back to is the HIGHEST reachable one (closest to the
/// client). Returns `None` when none are reachable, so the caller keeps its own
/// endpoint and lets `ensure_local_daemon_running` spawn a fresh daemon.
/// Factored out so the selection is unit-testable without real sockets.
fn select_fallback_daemon_version(
    candidates: &[((u64, u64, u64), bool)],
) -> Option<(u64, u64, u64)> {
    candidates
        .iter()
        .filter(|(_, reachable)| *reachable)
        .map(|(version, _)| *version)
        .max()
}

/// Resolve the daemon socket the GUI should connect to.
///
/// The daemon binds to `server-<own-version>.sock` and only back-aliases socket
/// names for versions <= its own (`refresh_legacy_server_socket_aliases`). A GUI
/// NEWER than the running daemon therefore finds NO socket at its own version;
/// naively using [`default_endpoint`] then points every connection — including
/// the app-control reconcile and the retained-replay re-source — at a
/// non-existent socket, silently stranding each reopened terminal host on the
/// stale client snapshot with the boring-reveal shadow stuck (see
/// `finding-gui-only-deploy-version-socket-mismatch`).
///
/// When the own-version socket is unreachable but an older-version daemon IS
/// reachable, connect to it (so sessions are not stranded) and report the
/// mismatch so the caller can surface a loud "deploy the matching daemon"
/// banner. The same-version case (own socket reachable) and the no-daemon case
/// (nothing reachable → keep own endpoint so one is spawned) are unchanged, so
/// this only ever ACTIVATES on the broken newer-GUI-older-daemon configuration.
#[cfg(unix)]
pub fn resolve_client_daemon_endpoint(home_dir: &Path) -> ClientDaemonEndpoint {
    let primary = default_endpoint(home_dir);
    // Own-version socket reachable → unchanged behaviour.
    if ping(&primary).is_ok() {
        return ClientDaemonEndpoint {
            endpoint: primary,
            version_mismatch: None,
        };
    }
    let ServerEndpoint::UnixSocket(primary_path) = &primary else {
        return ClientDaemonEndpoint {
            endpoint: primary,
            version_mismatch: None,
        };
    };
    // Probe every existing `server-*.sock` for a reachable daemon.
    let probed: Vec<((u64, u64, u64), bool)> =
        versioned_server_socket_alias_candidates(primary_path)
            .into_iter()
            .filter_map(|candidate| {
                let version = parse_versioned_server_socket_name(&candidate)?;
                let reachable = ping(&ServerEndpoint::UnixSocket(candidate)).is_ok();
                Some((version, reachable))
            })
            .collect();
    let Some(target_version) = select_fallback_daemon_version(&probed) else {
        // Nothing reachable — keep the own endpoint so a daemon is spawned.
        return ClientDaemonEndpoint {
            endpoint: primary,
            version_mismatch: None,
        };
    };
    let Some(parent) = primary_path.parent() else {
        return ClientDaemonEndpoint {
            endpoint: primary,
            version_mismatch: None,
        };
    };
    let (major, minor, patch) = target_version;
    let target_path = parent.join(format!("server-{major}-{minor}-{patch}.sock"));
    let own_version = parse_versioned_server_socket_name(primary_path)
        .map(format_socket_version)
        .unwrap_or_else(|| SERVER_PROTOCOL_VERSION.to_string());
    ClientDaemonEndpoint {
        endpoint: ServerEndpoint::UnixSocket(target_path),
        version_mismatch: Some((own_version, format_socket_version(target_version))),
    }
}

#[cfg(not(unix))]
pub fn resolve_client_daemon_endpoint(home_dir: &Path) -> ClientDaemonEndpoint {
    ClientDaemonEndpoint {
        endpoint: default_endpoint(home_dir),
        version_mismatch: None,
    }
}

#[cfg(not(unix))]
fn versioned_tcp_port() -> u16 {
    let mut parts = SERVER_PROTOCOL_VERSION.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(2);
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0)
        .min(9);
    58000 + major.saturating_mul(100) + minor.saturating_mul(10) + patch
}

pub fn ping(endpoint: &ServerEndpoint) -> Result<()> {
    match send_request(endpoint, &ServerRequest::Ping)? {
        ServerResponse::Pong => Ok(()),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected ping response: {:?}", other),
    }
}

pub fn status(endpoint: &ServerEndpoint) -> Result<ServerRuntimeStatus> {
    match send_request(endpoint, &ServerRequest::Status)? {
        ServerResponse::Status(status) => Ok(status),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected status response: {:?}", other),
    }
}

pub fn working_flags(endpoint: &ServerEndpoint) -> Result<Vec<(String, bool)>> {
    match send_request(endpoint, &ServerRequest::WorkingFlags)? {
        ServerResponse::WorkingFlags { flags } => Ok(flags),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected working-flags response: {:?}", other),
    }
}

#[cfg(unix)]
/// Background-thread body of the duplicate-runtime prune (see
/// `DaemonRuntime::prune_duplicate_legacy_owned_runtime_sessions`): walk the
/// other reachable daemons, drop runtime keys the current daemon owns, retire
/// an owner left empty, and queue preserved-owner registry removals for the
/// request loop to apply. Talks ONLY to other daemons' sockets + the trace
/// file — never to this daemon's in-memory state.
const PRESERVED_OWNER_REVALIDATE_INTERVAL_MS: u64 = 5 * 60_000;
static PRESERVED_OWNER_LAST_REVALIDATE_MS: AtomicU64 = AtomicU64::new(0);

/// Periodic preserved-owner registry revalidation (background chore thread).
///
/// Registry entries are otherwise checked only when a REQUEST touches them,
/// so an idle hollow entry — owner alive but no longer OWNING the runtime,
/// e.g. a predecessor that merely re-preserves the key toward an even older,
/// dead daemon — survives indefinitely. Live-caught 2026-07-17: 8 such
/// entries pinned `hot_update_handoff_active` true for four days, deferring
/// hot updates the whole time. One status probe per distinct owner endpoint;
/// removals are queued and applied on the request loop (registry owner).
fn run_preserved_owner_revalidation_if_due(runtime: &Arc<Mutex<DaemonRuntime>>) {
    let now_ms = current_millis_u64();
    let last_ms = PRESERVED_OWNER_LAST_REVALIDATE_MS.load(Ordering::SeqCst);
    if now_ms.saturating_sub(last_ms) < PRESERVED_OWNER_REVALIDATE_INTERVAL_MS {
        return;
    }
    if PRESERVED_OWNER_LAST_REVALIDATE_MS
        .compare_exchange(last_ms, now_ms, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let (home_dir, entries, pending_removals) = {
        let runtime = lock_daemon_runtime(runtime, "preserved_owner_revalidation");
        (
            runtime.store.home_dir().to_path_buf(),
            runtime.preserved_terminal_owners.entries.clone(),
            Arc::clone(&runtime.pending_preserved_owner_removals),
        )
    };
    if entries.is_empty() {
        return;
    }
    let mut owner_statuses: HashMap<String, Option<ServerRuntimeStatus>> = HashMap::new();
    let mut dropped: Vec<(String, &'static str)> = Vec::new();
    for entry in &entries {
        let endpoint = entry.endpoint.to_endpoint();
        let label = owner_endpoint_label(&endpoint);
        let owner_status = owner_statuses
            .entry(label)
            .or_insert_with(|| status(&endpoint).ok());
        match owner_status {
            None => dropped.push((
                entry.runtime_key.clone(),
                "revalidate_owner_unreachable",
            )),
            Some(owner_status)
                if !preserved_owner_status_serves_runtime_key(
                    owner_status,
                    &entry.runtime_key,
                ) =>
            {
                dropped.push((
                    entry.runtime_key.clone(),
                    "revalidate_owner_does_not_own_runtime",
                ));
            }
            Some(_) => {}
        }
    }
    if dropped.is_empty() {
        return;
    }
    append_trace_event(
        &home_dir,
        "daemon",
        "hot_update",
        "preserved_owner_revalidation_dropped",
        serde_json::json!({
            "checked_entries": entries.len(),
            "dropped": dropped
                .iter()
                .map(|(key, reason)| serde_json::json!({
                    "runtime_key": key,
                    "reason": reason,
                }))
                .collect::<Vec<_>>(),
        }),
    );
    let mut pending = pending_removals
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.extend(dropped);
}

fn run_duplicate_legacy_owned_runtime_prune(
    home_dir: &Path,
    reason: &'static str,
    current_runtime_keys: &HashSet<String>,
    registry_owner_endpoints: &HashMap<String, ServerEndpoint>,
    pending_preserved_owner_removals: &Mutex<Vec<(String, &'static str)>>,
) {
    let current_pid = std::process::id();
    let current_build_id = current_build_id();
    let current_endpoint = default_endpoint(home_dir);
    let statuses =
        reachable_versioned_daemon_statuses_excluding_endpoint(home_dir, &current_endpoint);
    for (owner_endpoint, owner_status) in statuses {
        let duplicate_runtime_keys = duplicate_legacy_owned_runtime_keys(
            current_runtime_keys,
            SERVER_PROTOCOL_VERSION,
            current_build_id,
            current_pid,
            &owner_status,
        );
        if duplicate_runtime_keys.is_empty() {
            continue;
        }
        let mut removed_runtime_keys = Vec::new();
        let mut errors = Vec::new();
        for runtime_key in duplicate_runtime_keys {
            match drop_terminal_runtime(
                &owner_endpoint,
                &runtime_key,
                Some("duplicate_legacy_owned_runtime_prune"),
            ) {
                Ok(_message) => {
                    let registry_points_to_owner = registry_owner_endpoints
                        .get(&runtime_key)
                        .is_some_and(|entry| {
                            server_endpoints_same_target(entry, &owner_endpoint)
                        });
                    if registry_points_to_owner {
                        let mut pending = pending_preserved_owner_removals
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        pending.push((
                            runtime_key.clone(),
                            "duplicate_legacy_owned_runtime_pruned_current_owned",
                        ));
                    }
                    removed_runtime_keys.push(runtime_key);
                }
                Err(error) => errors.push(serde_json::json!({
                    "runtime_key": runtime_key,
                    "error": error.to_string(),
                })),
            }
        }
        let status_after = status(&owner_endpoint).ok();
        let remaining_duplicate_runtime_keys = status_after
            .as_ref()
            .map(|status_after| {
                duplicate_legacy_owned_runtime_keys(
                    current_runtime_keys,
                    SERVER_PROTOCOL_VERSION,
                    current_build_id,
                    current_pid,
                    status_after,
                )
            })
            .unwrap_or_default();
        let remaining_owned_runtime_keys = status_after
            .as_ref()
            .map(|status_after| status_after.owned_terminal_session_keys.clone())
            .unwrap_or_default();
        let stale_daemon_retire = if !removed_runtime_keys.is_empty()
            && remaining_duplicate_runtime_keys.is_empty()
            && remaining_owned_runtime_keys.is_empty()
        {
            Some(
                match retire_daemon(&owner_endpoint, Some("duplicate_runtime_pruned")) {
                    Ok(message) => serde_json::json!({
                        "attempted": true,
                        "ok": true,
                        "message": message,
                    }),
                    Err(error) => serde_json::json!({
                        "attempted": true,
                        "ok": false,
                        "error": error.to_string(),
                    }),
                },
            )
        } else {
            None
        };
        append_trace_event(
            home_dir,
            "daemon",
            "hot_update",
            "duplicate_legacy_owned_runtimes_pruned",
            serde_json::json!({
                "reason": reason,
                "owner_endpoint": owner_endpoint_label(&owner_endpoint),
                "owner_server_version": owner_status.server_version,
                "owner_server_build_id": owner_status.server_build_id,
                "owner_server_pid": owner_status.server_pid,
                "removed_runtime_keys": removed_runtime_keys,
                "errors": errors,
                "remaining_duplicate_runtime_keys": remaining_duplicate_runtime_keys,
                "remaining_owned_runtime_keys": remaining_owned_runtime_keys,
                "stale_daemon_retire": stale_daemon_retire,
            }),
        );
    }
}

fn versioned_server_status_probe_paths(home_dir: &Path) -> Vec<PathBuf> {
    let mut seen = HashSet::<PathBuf>::new();
    let mut seen_socket_identities = HashSet::<PathBuf>::new();
    let mut paths = Vec::<PathBuf>::new();
    let mut push_path = |path: PathBuf| {
        if !seen.insert(path.clone()) {
            return;
        }
        let socket_identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen_socket_identities.insert(socket_identity) {
            return;
        }
        paths.push(path);
    };

    let current_endpoint = default_endpoint(home_dir);
    if let ServerEndpoint::UnixSocket(current_path) = current_endpoint {
        push_path(current_path.clone());
        for candidate in versioned_server_socket_alias_candidates(&current_path) {
            push_path(candidate);
        }
    }

    if let Ok(entries) = fs::read_dir(home_dir) {
        let mut home_paths = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| parse_versioned_server_socket_name(path).is_some())
            .collect::<Vec<_>>();
        home_paths.sort_by(|a, b| {
            parse_versioned_server_socket_name(b)
                .cmp(&parse_versioned_server_socket_name(a))
                .then_with(|| a.cmp(b))
        });
        for path in home_paths {
            push_path(path);
        }
    }

    paths
}

#[cfg(unix)]
fn versioned_server_status_probe_paths_excluding_endpoint(
    home_dir: &Path,
    excluded_endpoint: &ServerEndpoint,
) -> Vec<PathBuf> {
    let excluded_identity = match excluded_endpoint {
        ServerEndpoint::UnixSocket(path) => Some(fs::canonicalize(path).unwrap_or(path.clone())),
        ServerEndpoint::Tcp { .. } => None,
    };
    versioned_server_status_probe_paths(home_dir)
        .into_iter()
        .filter(|path| {
            let Some(excluded_identity) = excluded_identity.as_ref() else {
                return true;
            };
            fs::canonicalize(path)
                .unwrap_or_else(|_| path.clone())
                .as_path()
                != excluded_identity.as_path()
        })
        .collect()
}

#[cfg(unix)]
pub fn reachable_versioned_daemon_statuses(
    home_dir: &Path,
) -> Vec<(ServerEndpoint, ServerRuntimeStatus)> {
    let paths = versioned_server_status_probe_paths(home_dir);
    let mut seen_pids = HashSet::<u32>::new();

    paths
        .into_iter()
        .filter_map(|path| {
            let endpoint = ServerEndpoint::UnixSocket(path);
            let runtime = status(&endpoint).ok()?;
            if !seen_pids.insert(runtime.server_pid) {
                return None;
            }
            Some((endpoint, runtime))
        })
        .collect()
}

#[cfg(unix)]
fn reachable_versioned_daemon_statuses_excluding_endpoint(
    home_dir: &Path,
    excluded_endpoint: &ServerEndpoint,
) -> Vec<(ServerEndpoint, ServerRuntimeStatus)> {
    let mut seen_pids = HashSet::<u32>::new();
    versioned_server_status_probe_paths_excluding_endpoint(home_dir, excluded_endpoint)
        .into_iter()
        .filter_map(|path| {
            let endpoint = ServerEndpoint::UnixSocket(path);
            let runtime = status(&endpoint).ok()?;
            if !seen_pids.insert(runtime.server_pid) {
                return None;
            }
            Some((endpoint, runtime))
        })
        .collect()
}

#[cfg(not(unix))]
pub fn reachable_versioned_daemon_statuses(
    home_dir: &Path,
) -> Vec<(ServerEndpoint, ServerRuntimeStatus)> {
    let endpoint = default_endpoint(home_dir);
    status(&endpoint)
        .ok()
        .map(|runtime| vec![(endpoint, runtime)])
        .unwrap_or_default()
}

#[cfg(not(unix))]
fn reachable_versioned_daemon_statuses_excluding_endpoint(
    _home_dir: &Path,
    _excluded_endpoint: &ServerEndpoint,
) -> Vec<(ServerEndpoint, ServerRuntimeStatus)> {
    Vec::new()
}

pub fn snapshot(endpoint: &ServerEndpoint) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::Snapshot)?)
}

pub fn open_stored_session(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    open_stored_session_with_view(endpoint, kind, path, session_id, cwd, title_hint, None)
}

pub fn open_stored_session_with_view(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    view_mode: Option<WorkspaceViewMode>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::OpenStoredSession {
            session_kind: kind,
            path: path.to_string(),
            session_id: session_id.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            view_mode,
        },
    )?)
}

pub fn connect_ssh(
    endpoint: &ServerEndpoint,
    target_ix: usize,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ConnectSsh { target_ix },
    )?)
}

pub fn connect_ssh_custom(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ConnectSshCustom {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
        },
    )?)
}

pub fn open_remote_session(
    endpoint: &ServerEndpoint,
    machine_key: &str,
    session_id: &str,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    open_remote_session_with_view(endpoint, machine_key, session_id, cwd, title_hint, None)
}

pub fn open_remote_session_with_view(
    endpoint: &ServerEndpoint,
    machine_key: &str,
    session_id: &str,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    view_mode: Option<WorkspaceViewMode>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::OpenRemoteSession {
            machine_key: machine_key.to_string(),
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            view_mode,
        },
    )?)
}

pub fn start_ssh_session_at(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_ssh_session_at_with_terminal_appearance(endpoint, target, prefix, cwd, title_hint, None)
}

pub fn start_ssh_session_at_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_ssh_session_placed(
        endpoint,
        target,
        prefix,
        cwd,
        title_hint,
        terminal_appearance,
        None,
    )
}

pub fn start_ssh_session_placed(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
    insert_after: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartSshSession {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            insert_after: insert_after.map(ToOwned::to_owned),
        },
    )?)
}

pub fn start_remote_codex_session_at(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_remote_codex_session_at_with_terminal_appearance(
        endpoint, target, prefix, cwd, title_hint, None,
    )
}

pub fn start_remote_codex_session_at_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_remote_codex_session_placed(
        endpoint,
        target,
        prefix,
        cwd,
        title_hint,
        terminal_appearance,
        None,
    )
}

pub fn start_remote_codex_session_placed(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
    insert_after: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartRemoteCodexSession {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            insert_after: insert_after.map(ToOwned::to_owned),
        },
    )?)
}

pub fn start_remote_claude_session_at_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_remote_claude_session_placed(
        endpoint,
        target,
        prefix,
        cwd,
        title_hint,
        terminal_appearance,
        None,
    )
}

pub fn start_remote_claude_session_placed(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
    insert_after: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartRemoteClaudeSession {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            insert_after: insert_after.map(ToOwned::to_owned),
        },
    )?)
}

pub fn refresh_remote_machine(
    endpoint: &ServerEndpoint,
    machine_key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RefreshRemoteMachine {
            machine_key: machine_key.to_string(),
        },
    )?)
}

pub fn refresh_managed_cli(
    endpoint: &ServerEndpoint,
    machine_key: Option<&str>,
    background: bool,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::RefreshManagedCli {
            machine_key: machine_key.map(ToOwned::to_owned),
            background,
        },
    )?)
}

pub fn refresh_preview(
    endpoint: &ServerEndpoint,
    path: &str,
    full_remote_payload: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RefreshPreview {
            path: path.to_string(),
            full_remote_payload,
        },
    )?)
}

pub fn update_session_copy(
    endpoint: &ServerEndpoint,
    path: &str,
    title: Option<&str>,
    precis: Option<&str>,
    summary: Option<&str>,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::UpdateSessionCopy {
            path: path.to_string(),
            title: title.map(ToOwned::to_owned),
            precis: precis.map(ToOwned::to_owned),
            summary: summary.map(ToOwned::to_owned),
        },
    )?)
}

pub fn remove_ssh_target(
    endpoint: &ServerEndpoint,
    machine_key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RemoveSshTarget {
            machine_key: machine_key.to_string(),
        },
    )?)
}

pub fn remove_session(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RemoveSession {
            path: path.to_string(),
        },
    )?)
}

pub fn drop_terminal_runtime(
    endpoint: &ServerEndpoint,
    runtime_key: &str,
    reason: Option<&str>,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::DropTerminalRuntime {
            runtime_key: runtime_key.to_string(),
            reason: reason.map(ToOwned::to_owned),
        },
    )?)
}

pub fn set_session_keep_alive(
    endpoint: &ServerEndpoint,
    path: &str,
    keep_alive: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetSessionKeepAlive {
            path: path.to_string(),
            keep_alive,
        },
    )?)
}

pub fn reorder_live_sessions(
    endpoint: &ServerEndpoint,
    ordered_paths: &[String],
) -> Result<(ServerUiSnapshot, Option<String>)> {
    reorder_live_sessions_scoped(endpoint, ordered_paths, None)
}

/// Fetch the row-order ledger report (JSON string) — all scopes or one.
pub fn row_order_ledger_report(
    endpoint: &ServerEndpoint,
    scope: Option<&str>,
) -> Result<String> {
    match send_request(
        endpoint,
        &ServerRequest::RowOrderLedgerReport {
            scope: scope.map(str::to_string),
        },
    )? {
        ServerResponse::Ack { message } => Ok(message.unwrap_or_else(|| "{}".to_string())),
        other => anyhow::bail!("unexpected response to row-order ledger report: {other:?}"),
    }
}

pub fn reorder_live_sessions_scoped(
    endpoint: &ServerEndpoint,
    ordered_paths: &[String],
    client_scope: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ReorderLiveSessions {
            ordered_paths: ordered_paths.to_vec(),
            client_scope: client_scope.map(str::to_string),
        },
    )?)
}

pub fn start_local_session(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_local_session_at(endpoint, kind, None, None)
}

pub fn start_local_session_at(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_local_session_at_with_terminal_appearance(endpoint, kind, cwd, title_hint, None)
}

pub fn start_local_session_at_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_local_session_placed(endpoint, kind, cwd, title_hint, terminal_appearance, None)
}

pub fn start_local_session_placed(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    terminal_appearance: Option<&str>,
    insert_after: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartLocalSession {
            session_kind: kind,
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            insert_after: insert_after.map(ToOwned::to_owned),
        },
    )?)
}

pub fn start_command_session(
    endpoint: &ServerEndpoint,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    launch_command: &str,
    source_label: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_command_session_with_terminal_appearance(
        endpoint,
        cwd,
        title_hint,
        launch_command,
        source_label,
        None,
    )
}

pub fn start_command_session_with_terminal_appearance(
    endpoint: &ServerEndpoint,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    launch_command: &str,
    source_label: Option<&str>,
    terminal_appearance: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartCommandSession {
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            launch_command: launch_command.to_string(),
            source_label: source_label.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
        },
    )?)
}

pub fn ensure_remote_runtime_codex_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    require_existing: bool,
    initial_size: Option<(u16, u16)>,
    terminal_appearance: Option<&str>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::EnsureRemoteRuntimeCodexSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            require_existing,
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
        },
    )?)?
    .with_context(|| format!("missing runtime session key for {session_id}"))
}

pub fn start_remote_runtime_codex_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    initial_size: Option<(u16, u16)>,
    terminal_appearance: Option<&str>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::StartRemoteRuntimeCodexSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
        },
    )?)?
    .with_context(|| format!("missing runtime session key for {session_id}"))
}

pub fn ensure_remote_runtime_cc_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    require_existing: bool,
    initial_size: Option<(u16, u16)>,
    terminal_appearance: Option<&str>,
    claude_extra_args: Option<&str>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::EnsureRemoteRuntimeCcSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            require_existing,
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
            claude_extra_args: claude_extra_args.map(ToOwned::to_owned),
        },
    )?)?
    .with_context(|| format!("missing runtime session key for {session_id}"))
}

pub fn start_remote_runtime_cc_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    initial_size: Option<(u16, u16)>,
    terminal_appearance: Option<&str>,
    claude_extra_args: Option<&str>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::StartRemoteRuntimeCcSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
            claude_extra_args: claude_extra_args.map(ToOwned::to_owned),
        },
    )?)?
    .with_context(|| format!("missing runtime session key for {session_id}"))
}

pub fn ensure_shell_session(
    endpoint: &ServerEndpoint,
    session_id: &str,
    cwd: Option<&str>,
    initial_size: Option<(u16, u16)>,
) -> Result<String> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::EnsureShellSession {
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
        },
    )?)?
    .with_context(|| format!("missing shell session key for {session_id}"))
}

pub fn focus_live(
    endpoint: &ServerEndpoint,
    key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    focus_live_with_view(endpoint, key, None)
}

pub fn focus_live_with_view(
    endpoint: &ServerEndpoint,
    key: &str,
    view_mode: Option<WorkspaceViewMode>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::FocusLive {
            key: key.to_string(),
            view_mode,
        },
    )?)
}

pub fn switch_agent_session_mode(
    endpoint: &ServerEndpoint,
    path: &str,
    kind: SessionKind,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SwitchAgentSessionMode {
            path: path.to_string(),
            session_kind: kind,
        },
    )?)
}

pub fn set_view_mode(
    endpoint: &ServerEndpoint,
    mode: WorkspaceViewMode,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetViewMode { mode },
    )?)
}

pub fn toggle_preview_block(
    endpoint: &ServerEndpoint,
    block_ix: usize,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::TogglePreviewBlock { block_ix },
    )?)
}

pub fn set_all_preview_blocks_folded(
    endpoint: &ServerEndpoint,
    folded: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetAllPreviewBlocksFolded { folded },
    )?)
}

pub fn request_terminal_launch(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RequestTerminalLaunch,
    )?)
}

pub fn request_terminal_launch_for_path(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RequestTerminalLaunchForPath {
            path: path.to_string(),
        },
    )?)
}

pub fn terminal_ensure(endpoint: &ServerEndpoint, path: &str) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalEnsure {
            path: path.to_string(),
        },
    )?)
}

pub fn terminal_read(
    endpoint: &ServerEndpoint,
    path: &str,
    cursor: u64,
) -> Result<(u64, Vec<TerminalStreamChunk>, bool, bool, bool, bool, u64, bool)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalRead {
            path: path.to_string(),
            cursor,
        },
    )? {
        ServerResponse::TerminalStream {
            cursor,
            chunks,
            running,
            runtime_output_seen,
            eof_without_output,
            post_resize_output_seen,
            last_resize_seq,
            resync_required,
        } => Ok((
            cursor,
            chunks,
            running,
            runtime_output_seen,
            eof_without_output,
            post_resize_output_seen,
            last_resize_seq,
            resync_required,
        )),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal stream response: {:?}", other),
    }
}

pub fn terminal_snapshot(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(String, bool, bool, bool, u64, u64)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalSnapshot {
            path: path.to_string(),
        },
    )? {
        ServerResponse::TerminalSnapshot {
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
            runtime_spawn_id,
        } => Ok((
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
            runtime_spawn_id,
        )),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal snapshot response: {:?}", other),
    }
}

pub fn terminal_retained_snapshot(
    endpoint: &ServerEndpoint,
    path: &str,
) -> Result<(String, bool, bool, bool, u64, u64)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalRetainedSnapshot {
            path: path.to_string(),
        },
    )? {
        ServerResponse::TerminalRetainedSnapshot {
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
            runtime_spawn_id,
        } => Ok((
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
            runtime_spawn_id,
        )),
        ServerResponse::Error { message } => bail!(message),
        other => bail!(
            "unexpected terminal retained snapshot response: {:?}",
            other
        ),
    }
}

pub fn terminal_history(endpoint: &ServerEndpoint, path: &str) -> Result<(Vec<String>, bool)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalHistory {
            path: path.to_string(),
        },
    )? {
        ServerResponse::TerminalHistory { rows, running } => Ok((rows, running)),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal history response: {:?}", other),
    }
}

pub fn terminal_write(endpoint: &ServerEndpoint, path: &str, data: &str) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalWrite {
            path: path.to_string(),
            data: data.to_string(),
        },
    )?)
}

pub fn terminal_resize(
    endpoint: &ServerEndpoint,
    path: &str,
    cols: u16,
    rows: u16,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalResize {
            path: path.to_string(),
            cols,
            rows,
        },
    )?)
}

pub fn terminal_restart(
    endpoint: &ServerEndpoint,
    path: &str,
    terminal_appearance: Option<&str>,
    force_remote: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    terminal_restart_with_size(endpoint, path, terminal_appearance, force_remote, None)
}

pub fn terminal_restart_with_size(
    endpoint: &ServerEndpoint,
    path: &str,
    terminal_appearance: Option<&str>,
    force_remote: bool,
    initial_size: Option<(u16, u16)>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::TerminalRestart {
            path: path.to_string(),
            terminal_appearance: terminal_appearance.map(ToOwned::to_owned),
            force_remote,
            initial_cols: initial_size.map(|(cols, _)| cols),
            initial_rows: initial_size.map(|(_, rows)| rows),
        },
    )?)
}

pub fn sync_external_window(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::SyncExternalWindow)?)
}

pub fn raise_external_window(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::RaiseExternalWindow)?)
}

pub fn sync_theme(
    endpoint: &ServerEndpoint,
    theme: UiTheme,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::SyncTheme { theme })?)
}

pub fn sync_terminal_identity(
    endpoint: &ServerEndpoint,
    terminal_appearance: &str,
) -> Result<Option<String>> {
    sync_terminal_identity_with_profile(endpoint, terminal_appearance, None)
}

pub fn sync_terminal_identity_with_profile(
    endpoint: &ServerEndpoint,
    terminal_appearance: &str,
    terminal_profile: Option<&TerminalIdentityColorProfile>,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::SyncTerminalIdentity {
            terminal_appearance: terminal_appearance.to_string(),
            terminal_profile: terminal_profile.cloned(),
        },
    )?)
}

pub fn prepare_update_restart(endpoint: &ServerEndpoint) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::PrepareUpdateRestart,
    )?)
}

pub fn prepare_client_close(endpoint: &ServerEndpoint) -> Result<Option<String>> {
    expect_ack(send_request(endpoint, &ServerRequest::PrepareClientClose)?)
}

pub fn hot_restart(
    endpoint: &ServerEndpoint,
    daemon_executable: &Path,
    expected_version: Option<&str>,
    expected_build_id: Option<u64>,
    reason: Option<&str>,
) -> Result<Option<String>> {
    Ok(hot_restart_detailed(
        endpoint,
        daemon_executable,
        expected_version,
        expected_build_id,
        reason,
        false,
    )?
    .message())
}

pub fn hot_restart_detailed(
    endpoint: &ServerEndpoint,
    daemon_executable: &Path,
    expected_version: Option<&str>,
    expected_build_id: Option<u64>,
    reason: Option<&str>,
    force: bool,
) -> Result<HotRestartResult> {
    let daemon_executable = daemon_executable
        .canonicalize()
        .unwrap_or_else(|_| daemon_executable.to_path_buf());
    match send_request(
        endpoint,
        &ServerRequest::HotRestart {
            daemon_executable: daemon_executable.display().to_string(),
            expected_version: expected_version.map(ToOwned::to_owned),
            expected_build_id,
            reason: reason.map(ToOwned::to_owned),
            force,
        },
    )? {
        ServerResponse::Ack { message } => Ok(HotRestartResult::Restarting { message }),
        ServerResponse::HotUpdateHandoff {
            message,
            owner_endpoint,
            owner_server_version,
            owner_server_pid,
            target_server_version,
            runtime_keys,
        } => Ok(HotRestartResult::Handoff {
            message,
            owner_endpoint,
            owner_server_version,
            owner_server_pid,
            target_server_version,
            runtime_keys,
        }),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected hot restart response: {:?}", other),
    }
}

/// The first daemon version whose `Shutdown` does not write into live prompts.
///
/// Before it, `shutdown_all` WROTE `/exit\r` (Claude Code), `/quit\r` (codex) or
/// `exit\r` (shells) into every live PTY. A PTY write is appended to whatever
/// the user has already typed and submitted, so a half-written prompt fired with
/// `/exit` stuck on the end. [[finding-never-type-into-a-live-prompt]]
pub const FIRST_VERSION_WITH_SIGNAL_BASED_SHUTDOWN: &str = "2.9.66";

fn daemon_version_triplet(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// Would asking a daemon of this version to `Shutdown` type into the user's
/// prompts? An unparseable version is assumed to be old — the safe direction.
pub fn daemon_shutdown_writes_into_prompts(server_version: &str) -> bool {
    let Some(version) = daemon_version_triplet(server_version) else {
        return true;
    };
    let first_safe = daemon_version_triplet(FIRST_VERSION_WITH_SIGNAL_BASED_SHUTDOWN)
        .expect("the constant is a valid triplet");
    version < first_safe
}

/// Stop a daemon, never making it type into a live prompt.
///
/// A current daemon gets `Shutdown`: its `terminal_stop_command` returns `None`
/// for anything with a prompt, so `shutdown_all` signals the children instead of
/// writing to them. A daemon older than
/// [`FIRST_VERSION_WITH_SIGNAL_BASED_SHUTDOWN`] would write, so it is asked to
/// `RetireDaemon` (which never touches terminals) and, if it lingers, signalled.
/// Closing the PTY master delivers SIGHUP to its children — the same thing a
/// terminal emulator does when its window closes.
///
/// This is the ONE entry point. Every caller — the GUI's legacy-daemon cleanup,
/// the headless monitor's fallback, the `server shutdown` verb — routes here, so
/// no code path can resurrect the prompt injection.
pub fn shutdown(endpoint: &ServerEndpoint) -> Result<Option<String>> {
    let legacy_pid = match status(endpoint) {
        Ok(runtime) if daemon_shutdown_writes_into_prompts(&runtime.server_version) => {
            Some(runtime.server_pid)
        }
        // Current daemon, or unreachable (nothing to protect).
        _ => None,
    };
    let Some(pid) = legacy_pid else {
        return expect_ack(send_request(endpoint, &ServerRequest::Shutdown)?);
    };
    let message = retire_daemon(endpoint, Some("legacy_shutdown_would_type_into_prompts"))?;
    for _ in 0..20 {
        if status(endpoint).is_err() {
            return Ok(message);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Still answering: close it the way a terminal emulator closes a window.
    // SAFETY: `pid` is the daemon that just answered our status request.
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    let _ = pid;
    Ok(message)
}

pub fn retire_daemon(endpoint: &ServerEndpoint, reason: Option<&str>) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::RetireDaemon {
            reason: reason.map(ToOwned::to_owned),
        },
    )?)
}

#[derive(Debug, Default, serde::Serialize)]
pub struct RetireStaleDaemonsReport {
    pub current_version: String,
    pub considered: Vec<RetireStaleDaemonOutcome>,
    pub retired_count: usize,
    pub skipped_count: usize,
    pub unreachable_count: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct RetireStaleDaemonOutcome {
    pub socket: String,
    pub status: String,
    pub server_version: Option<String>,
    pub server_pid: Option<u32>,
    pub message: Option<String>,
}

/// A terminal runtime key whose real PTY lives on a REMOTE host's daemon
/// (`remote-session://` and its Claude Code twin `remote-cc://`, per the
/// decentralized host-daemon architecture). Dropping a stale local daemon's
/// record for such a key is non-destructive: the remote daemon keeps the PTY
/// and the local attachment respawns on the next ensure — the same thing that
/// already happens on every deploy.
fn terminal_runtime_key_is_remote_hosted(key: &str) -> bool {
    crate::is_remote_scanned_live_session_path(key)
        || crate::parse_remote_cc_session_path(key).is_some()
}

/// Per [[bug-class-old-daemon-never-retires]]: session-safe coverage rule for
/// retiring a STALE daemon that still lists terminal sessions. Two legs:
/// (a) COVERAGE — every key the stale daemon lists (owned + preserved) must
///     also be present in the current daemon's records, so no session record
///     is lost; and
/// (b) OWNERSHIP SAFETY — every key the stale daemon actually OWNS (has a
///     live runtime for) must be remote-hosted (the PTY lives on the remote
///     machine's daemon; the attachment respawns on demand) or OWNED by the
///     current daemon (the stale copy is a superseded duplicate runtime).
/// Preserved-only records carry no process resources (the preserved-owner
/// registry is a shared file, and there is no fd handoff), so a stale daemon
/// with zero owned runtimes is retire-safe once coverage holds — the jojo
/// 2.8.87 lingering duplicate held 14 preserved-only records for hours.
/// A daemon reporting counts above its key lists cannot be verified and
/// stays running.
pub fn stale_daemon_retire_covered_by_current(
    current: &ServerRuntimeStatus,
    stale: &ServerRuntimeStatus,
) -> bool {
    if stale.terminal_session_count > stale.terminal_session_keys.len()
        || stale.owned_terminal_session_count > stale.owned_terminal_session_keys.len()
    {
        return false;
    }
    let covered: HashSet<&str> = current
        .terminal_session_keys
        .iter()
        .map(String::as_str)
        .collect();
    let owned_by_current: HashSet<&str> = current
        .owned_terminal_session_keys
        .iter()
        .map(String::as_str)
        .collect();
    stale
        .terminal_session_keys
        .iter()
        .all(|key| covered.contains(key.as_str()))
        && stale.owned_terminal_session_keys.iter().all(|key| {
            terminal_runtime_key_is_remote_hosted(key) || owned_by_current.contains(key.as_str())
        })
}

/// Per [[bug-class-old-daemon-never-retires]]: yggterm-headless processes
/// from older deploys keep running because the idle-shutdown gate is too
/// conservative (it counts preserved-owner sessions as live work) and there
/// is no disk-binary-version poll. This helper walks every
/// `server-*.sock` in `home_dir`, probes the version of the daemon behind
/// it, and sends `RetireDaemon` to any whose version differs from
/// `current_version`. Sockets that are unreachable are removed; the current
/// daemon's own socket is skipped. A stale daemon that still lists sessions
/// is retired only under [`stale_daemon_retire_covered_by_current`].
#[cfg(unix)]
pub fn retire_stale_daemons(
    home_dir: &Path,
    current_version: &str,
) -> Result<RetireStaleDaemonsReport> {
    let mut report = RetireStaleDaemonsReport {
        current_version: current_version.to_string(),
        ..RetireStaleDaemonsReport::default()
    };
    let current_endpoint_path = match default_endpoint(home_dir) {
        ServerEndpoint::UnixSocket(path) => Some(path),
        #[allow(unreachable_patterns)]
        _ => None,
    };
    // Probed once: the current daemon's status, used for the session-coverage
    // retire rule. Unreachable current daemon → no coverage-based retires.
    let current_status = current_endpoint_path
        .as_ref()
        .and_then(|path| status(&ServerEndpoint::UnixSocket(path.clone())).ok());
    let entries = match fs::read_dir(home_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(report),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading daemon socket dir {}", home_dir.display()));
        }
    };
    let mut socket_paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("server-") && n.ends_with(".sock"))
                .unwrap_or(false)
        })
        .collect();
    socket_paths.sort();
    let mut seen_inode = HashSet::<PathBuf>::new();
    for path in socket_paths {
        let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen_inode.insert(canonical.clone()) {
            continue;
        }
        if current_endpoint_path
            .as_ref()
            .map(|cur| fs::canonicalize(cur).unwrap_or_else(|_| cur.clone()) == canonical)
            .unwrap_or(false)
        {
            continue;
        }
        let endpoint = ServerEndpoint::UnixSocket(path.clone());
        let outcome = match status(&endpoint) {
            Ok(s) => {
                if s.server_version == current_version {
                    report.skipped_count += 1;
                    RetireStaleDaemonOutcome {
                        socket: path.display().to_string(),
                        status: "skipped_same_version".to_string(),
                        server_version: Some(s.server_version),
                        server_pid: Some(s.server_pid),
                        message: None,
                    }
                } else if s.terminal_session_count > 0
                    && !current_status
                        .as_ref()
                        .map(|current| stale_daemon_retire_covered_by_current(current, &s))
                        .unwrap_or(false)
                {
                    // SESSION-SAFE: never retire a daemon that still owns live
                    // sessions (incl. preserved hot-update PTYs) — there is no
                    // fd-handoff, so retiring it would interrupt those sessions —
                    // unless the current daemon covers every key and none of
                    // them is a local PTY the stale daemon could still hold
                    // (see stale_daemon_retire_covered_by_current). It will
                    // retire itself once it drains to zero (idle gate +
                    // supersession check). Per [[bug-class-old-daemon-never-retires]].
                    report.skipped_count += 1;
                    RetireStaleDaemonOutcome {
                        socket: path.display().to_string(),
                        status: "skipped_owns_live_sessions".to_string(),
                        server_version: Some(s.server_version),
                        server_pid: Some(s.server_pid),
                        message: Some(format!(
                            "owns {} live session(s); left running",
                            s.terminal_session_count
                        )),
                    }
                } else {
                    let covered_retire = s.terminal_session_count > 0;
                    let reason = if covered_retire {
                        "retire_stale_daemons_covered_by_current"
                    } else {
                        "retire_stale_daemons_cli"
                    };
                    match retire_daemon(&endpoint, Some(reason)) {
                        Ok(message) => {
                            report.retired_count += 1;
                            RetireStaleDaemonOutcome {
                                socket: path.display().to_string(),
                                status: if covered_retire {
                                    "retired_covered_by_current".to_string()
                                } else {
                                    "retired".to_string()
                                },
                                server_version: Some(s.server_version),
                                server_pid: Some(s.server_pid),
                                message,
                            }
                        }
                        Err(error) => RetireStaleDaemonOutcome {
                            socket: path.display().to_string(),
                            status: "retire_failed".to_string(),
                            server_version: Some(s.server_version),
                            server_pid: Some(s.server_pid),
                            message: Some(error.to_string()),
                        },
                    }
                }
            }
            Err(_) => {
                report.unreachable_count += 1;
                // Socket-litter cleanup (run #19): a dead socket file has no
                // listener; unlinking it is safe (a daemon removes/rebinds its
                // own path at startup anyway), and leaving it makes every
                // cross-daemon walk probe it forever (~600 had accumulated).
                let removed = fs::remove_file(&path).is_ok();
                RetireStaleDaemonOutcome {
                    socket: path.display().to_string(),
                    status: if removed {
                        "removed_stale_socket".to_string()
                    } else {
                        "unreachable".to_string()
                    },
                    server_version: None,
                    server_pid: None,
                    message: None,
                }
            }
        };
        report.considered.push(outcome);
    }
    Ok(report)
}

pub fn cleanup_legacy_daemons(endpoint: &ServerEndpoint, current_exe: &Path) -> Result<()> {
    #[cfg(unix)]
    cleanup_legacy_unix_daemons(endpoint)?;
    #[cfg(target_os = "linux")]
    cleanup_legacy_linux_daemon_processes(endpoint, current_exe)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_legacy_linux_daemon_cleanup(
    endpoint: ServerEndpoint,
    current_exe: PathBuf,
    home_dir: PathBuf,
) {
    if let Err(error) = std::thread::Builder::new()
        .name("yggterm-legacy-daemon-cleanup".to_string())
        .spawn(move || {
            append_trace_event(
                &home_dir,
                "daemon",
                "lifecycle",
                "legacy_cleanup_begin",
                serde_json::json!({
                    "endpoint": format!("{endpoint:?}"),
                    "current_exe": current_exe.display().to_string(),
                }),
            );
            match cleanup_legacy_linux_daemon_processes(&endpoint, &current_exe) {
                Ok(()) => append_trace_event(
                    &home_dir,
                    "daemon",
                    "lifecycle",
                    "legacy_cleanup_end",
                    serde_json::json!({}),
                ),
                Err(error) => {
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "lifecycle",
                        "legacy_cleanup_error",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                    warn!(error=%error, "legacy daemon cleanup failed");
                }
            }
        })
    {
        warn!(error=%error, "failed to spawn legacy daemon cleanup");
    }
}

#[cfg(target_os = "linux")]
fn spawn_force_remote_restart_daemon_cleanup(store: &SessionStore, owner_registry_empty: bool) {
    if !owner_registry_empty {
        return;
    }
    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    spawn_legacy_linux_daemon_cleanup(
        default_endpoint(store.home_dir()),
        current_exe,
        store.home_dir().to_path_buf(),
    );
}

#[cfg(not(target_os = "linux"))]
fn spawn_force_remote_restart_daemon_cleanup(_store: &SessionStore, _owner_registry_empty: bool) {}

/// Per [[bug-class-old-daemon-never-retires]]: poll `/proc/self/exe` every
/// 60s. Linux appends the literal " (deleted)" suffix to the link target
/// when the file on disk has been replaced (the running process holds the
/// old inode open; the path now resolves to a fresher binary). When we
/// detect this, send self a Shutdown RPC so the main accept loop exits
/// cleanly — the next client connection spawns a fresh daemon from the
/// new on-disk binary. Without this, old daemons sit holding preserved-
/// owner sessions and never voluntarily retire for newer code.
#[cfg(target_os = "linux")]
fn spawn_disk_binary_version_poll(
    endpoint: ServerEndpoint,
    home_dir: PathBuf,
    runtime: Arc<Mutex<DaemonRuntime>>,
) {
    std::thread::spawn(move || {
        // 20s (was 60s): shrink the window in which a stale OLD daemon coexists
        // with a newer successor after an update, so the split-brain that strands
        // remote sessions on the seed placeholder is cleared quickly.
        // [[finding-blank-on-restart-split-brain-daemon]]
        const POLL_INTERVAL_MS: u64 = 20_000;
        // Periodic stale-daemon sweep cadence: every Nth poll ≈ 3 minutes.
        const STALE_SWEEP_EVERY_N_POLLS: u32 = 9;
        let mut stale_sweep_countdown = STALE_SWEEP_EVERY_N_POLLS;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            // Retire trigger 1: our on-disk binary was replaced by an update.
            let exe_link = fs::read_link("/proc/self/exe")
                .map(|link| link.to_string_lossy().into_owned())
                .unwrap_or_default();
            let binary_replaced = exe_link.ends_with(" (deleted)");
            // Retire trigger 2: a strictly NEWER-version daemon is already live.
            // An update spawns a new-version successor, but THIS daemon's versioned
            // binary still exists on disk (different versioned path), so trigger 1
            // never fires — leaving a SPLIT-BRAIN of old+new daemons that strands
            // remote-session terminal streams on the seed placeholder (= blank
            // viewport on every update). Retiring the stale daemon collapses back to
            // one. See [[finding-blank-on-restart-split-brain-daemon]].
            let newer_daemon_version = if binary_replaced {
                None
            } else {
                reachable_versioned_daemon_statuses_excluding_endpoint(&home_dir, &endpoint)
                    .into_iter()
                    .map(|(_, status)| status.server_version)
                    .find(|version| {
                        server_version_is_strictly_newer(version, SERVER_PROTOCOL_VERSION)
                    })
            };
            let retire_trigger = if binary_replaced {
                "disk_binary_replaced"
            } else if newer_daemon_version.is_some() {
                "newer_daemon_live"
            } else {
                // We are the CURRENT daemon. Size-war lesson (2026-06-12): the
                // startup-only takeover/retire leaves a stale OLDER daemon
                // that survives that window (idle-gate defer) free to churn
                // duplicate attachments indefinitely — split-brain until a
                // human runs retire-stale-daemons. Sweep periodically instead
                // (~3min; session-safe coverage rules apply).
                stale_sweep_countdown = stale_sweep_countdown.saturating_sub(1);
                if stale_sweep_countdown == 0 {
                    stale_sweep_countdown = STALE_SWEEP_EVERY_N_POLLS;
                    let report = retire_stale_daemons(&home_dir, SERVER_PROTOCOL_VERSION);
                    if let Ok(report) = report
                        && report.retired_count > 0
                    {
                        append_trace_event(
                            &home_dir,
                            "daemon",
                            "lifecycle",
                            "periodic_stale_daemon_sweep",
                            serde_json::json!({
                                "retired": report.retired_count,
                                "skipped": report.skipped_count,
                                "unreachable": report.unreachable_count,
                            }),
                        );
                    }
                }
                continue;
            };
            {
                // GATE #8: a strictly newer daemon owns server-state.json from
                // here on — mute this daemon's routine persists so its stale
                // in-memory view can never clobber the successor's file.
                let mut rt = lock_daemon_runtime(&runtime, "disk_binary_replace_persist_mute");
                if retire_trigger == "newer_daemon_live" && !rt.superseded_routine_persist_muted {
                    rt.superseded_routine_persist_muted = true;
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "lifecycle",
                        "superseded_routine_persist_muted",
                        serde_json::json!({
                            "newer_daemon_version": newer_daemon_version,
                            "current_version": SERVER_PROTOCOL_VERSION,
                            "current_pid": std::process::id(),
                        }),
                    );
                }
            }
            // The idle gate is deliberately NOT applied here. It used to sit in
            // front of BOTH branches below, which meant one busy agent session
            // blocked the PRESERVING handoff for every other session — and the
            // handoff is exactly what spawns the successor that progressive
            // migration then drains sessions into, one at a time, as each goes
            // idle. So a daemon with a single always-active session could never
            // converge: jojo sat on 2.9.63 for a day while the disk binary was
            // 2.9.66, deferring on the user's focused Claude Code session.
            // The gate belongs on the COLD SHUTDOWN fallback below, which really
            // does kill PTYs and re-resume every agent. Preserving a runtime is
            // not "churning live work"; killing it is.
            // [[finding-hot-update-never-converges-idle-gate]]
            let exe_link_for_handoff = exe_link.clone();
            append_trace_event(
                &home_dir,
                "daemon",
                "lifecycle",
                "daemon_self_retire",
                serde_json::json!({
                    "retire_trigger": retire_trigger,
                    "exe_link": exe_link,
                    "newer_daemon_version": newer_daemon_version,
                    "current_version": SERVER_PROTOCOL_VERSION,
                    "current_pid": std::process::id(),
                }),
            );
            // A cold shutdown kills this daemon's PTY children; the next client
            // then recovery-spawns a fresh daemon that RE-RESUMES every agent on
            // a new PTY — the overnight "all CC bottoms broke" incident
            // ([[finding-deploy-armed-cold-self-retire-mass-re-resume]]). When the
            // replacement is a newer on-disk binary AND we still own live PTYs,
            // prefer the session-preserving hot-restart handoff (the same path the
            // explicit HotRestart RPC + the deploy-via-hot-restart spec use): keep
            // THIS process alive as the preserved PTY owner and spawn the
            // new-version successor to ADOPT the streams — no re-resume. The
            // newer_daemon_live split-brain case, the kill-switch, and any handoff
            // failure all fall back to the cold shutdown so we still retire.
            let handed_off = retire_trigger == "disk_binary_replaced"
                && !self_retire_handoff_disabled()
                && {
                    let owned = lock_daemon_runtime(&runtime, "disk_binary_replace_handoff")
                        .terminals
                        .session_keys();
                    !owned.is_empty()
                        && attempt_self_retire_preserving_handoff(
                            &endpoint,
                            &home_dir,
                            &exe_link_for_handoff,
                            &owned,
                        )
                };
            if handed_off {
                // The handler spawned the new-version successor and wrote the
                // preserved-owner registry; we now linger as the PTY owner. Do
                // NOT shutdown (that would kill the PTYs we just preserved). Stop
                // polling and, when progressive migration is enabled, hand our
                // sessions one-by-one to the successor as each becomes safe so
                // ownership converges to the newest daemon (working-state +
                // titles then work natively there); otherwise idle-shutdown /
                // the stale-daemon sweep retires us once our sessions drain.
                // See [[finding-daemon-authoritative-working-state-2945]].
                spawn_progressive_session_migration(
                    endpoint.clone(),
                    home_dir.clone(),
                    Arc::clone(&runtime),
                );
                break;
            }
            // No handoff: the only way out is a COLD SHUTDOWN, which kills this
            // daemon's PTY children and makes the next client recovery-spawn a
            // daemon that RE-RESUMES every agent on a fresh PTY — interrupting
            // any in-flight turn. THIS is what the idle gate exists to prevent.
            // Defer while any owned session is mid-turn or was active inside the
            // idle window, and re-check next poll; we retire only once idle, so a
            // busy agent's job is never broken.
            // Overridable via YGGTERM_HOT_UPDATE_IGNORE_IDLE_GATE.
            let block_reason = {
                let rt = lock_daemon_runtime(&runtime, "cold_shutdown_idle_gate");
                let owned = rt.terminals.session_keys();
                rt.hot_update_idle_gate_block_reason(&owned)
            };
            if let Some(reason) = block_reason {
                // PROBE hot_update_deferred: the ONLY durable record of why a daemon is
                // still running an old build. Without it a stale daemon is invisible
                // after the fact — jojo ran 2.10.3 for 19h44m with 2.10.13 on disk and
                // there was nothing to mine that said why ([[finding-stale-daemon-trap]]).
                // Carries EVERY blocker, so "which session pinned it, for how long" is
                // answerable from the trace alone. Once per poll is fine: the retire loop
                // ticks slowly and a stale daemon is exactly the thing worth over-logging.
                let blockers = {
                    let rt = lock_daemon_runtime(&runtime, "cold_shutdown_idle_gate_blockers");
                    let owned = rt.terminals.session_keys();
                    rt.hot_update_idle_gate_blockers(&owned)
                };
                append_trace_event(
                    &home_dir,
                    "daemon",
                    "lifecycle",
                    "daemon_cold_shutdown_deferred_idle_gate",
                    serde_json::json!({
                        "retire_trigger": retire_trigger,
                        "exe_link": exe_link,
                        "newer_daemon_version": newer_daemon_version,
                        "current_version": SERVER_PROTOCOL_VERSION,
                        "current_pid": std::process::id(),
                        "current_uptime_ms": current_millis_u64()
                            .saturating_sub(*DAEMON_STARTED_AT_MS),
                        "reason": reason,
                        "blocker_count": blockers.len(),
                        "blockers": blockers,
                    }),
                );
                continue;
            }
            // Best-effort self-shutdown. If the call fails (e.g. socket
            // already torn down), the thread just exits and the daemon
            // continues; the next poll cycle would retry, but the
            // process should already be on its way out.
            let _ = shutdown(&endpoint);
            break;
        }
    });
}

/// Progressive per-session migration: default ON, with a kill switch.
///
/// It was shipped dormant "until live-verified through a controlled two-deploy
/// rollout", and that rollout never came — so a handed-off daemon lingered as
/// the preserved owner forever and ownership never converged to the newest
/// daemon. This is the mechanism that releases sessions one at a time as each
/// becomes safe; its per-session predicate (`session_is_migratable_now`) is
/// where "all but the busy few" is decided, and it is the reason the handoff
/// itself must not be gated on session activity.
///
/// Disable with `YGGTERM_ENABLE_PROGRESSIVE_MIGRATION=0` (also `false`/`no`/`off`).
/// See [[finding-hot-update-never-converges-idle-gate]],
/// [[finding-daemon-authoritative-working-state-2945]].
fn progressive_migration_enabled() -> bool {
    match std::env::var("YGGTERM_ENABLE_PROGRESSIVE_MIGRATION") {
        Ok(value) => !matches!(value.trim(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// Only agent CLI sessions are migratable via release+re-resume: their state
/// persists in the agent's own JSONL, so killing the PTY and re-resuming on the
/// newest daemon is lossless (once the migration predicate has ruled out an
/// unsent draft). A plain shell has no such persistence — re-running its launch
/// command yields a fresh shell — so it is never released this way (it stays
/// with the lingering owner, awaiting a future lossless fd-handoff).
fn session_kind_is_migratable_agent(kind: SessionKind) -> bool {
    matches!(
        kind,
        SessionKind::Codex | SessionKind::CodexLiteLlm | SessionKind::ClaudeCode
    )
}

/// One owned session's migration inputs for a single tick.
struct MigrationCandidateRow {
    runtime_key: String,
    re_resumable: bool,
    migratable: bool,
    idle_ms: u64,
}

/// Pick the SINGLE session to migrate this tick — oldest-idle-first (largest
/// idle wins) among sessions that are BOTH re-resumable agents AND currently
/// migratable. `None` when nothing is safe yet. Pacing one-per-tick avoids a
/// post-deploy migration storm (CPU + reveal-storm class). Pure for testing.
fn select_next_migration_candidate(rows: &[MigrationCandidateRow]) -> Option<String> {
    rows.iter()
        .filter(|row| row.re_resumable && row.migratable)
        .max_by_key(|row| row.idle_ms)
        .map(|row| row.runtime_key.clone())
}

/// Drive progressive convergence from a lingering preserved-owner daemon: each
/// tick, if a successor is reachable to adopt, release the oldest-idle safe
/// session so the successor re-resumes and OWNS it. Exit once our hands are
/// empty (the successor owns everything). Sessions that never become migratable
/// (e.g. a plain keep-alive shell) keep us lingering harmlessly.
fn spawn_progressive_session_migration(
    endpoint: ServerEndpoint,
    home_dir: PathBuf,
    runtime: Arc<Mutex<DaemonRuntime>>,
) {
    if !progressive_migration_enabled() {
        return;
    }
    std::thread::spawn(move || {
        // Paced cadence: one release per tick, oldest-idle first.
        const POLL_INTERVAL_MS: u64 = 5_000;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            let owned = {
                let rt = lock_daemon_runtime(&runtime, "progressive_migration_owned");
                rt.terminals.session_keys()
            };
            if owned.is_empty() {
                // Empty hands: the successor owns everything now — retire.
                append_trace_event(
                    &home_dir,
                    "daemon",
                    "lifecycle",
                    "progressive_migration_owner_empty_retire",
                    serde_json::json!({ "current_pid": std::process::id() }),
                );
                let _ = shutdown(&endpoint);
                break;
            }
            // Never release the only live owner: require a reachable successor
            // to adopt the released session, or we would strand it.
            let successor_reachable = !reachable_versioned_daemon_statuses_excluding_endpoint(
                &home_dir, &endpoint,
            )
            .is_empty();
            if !successor_reachable {
                continue;
            }
            let candidate = {
                let rt = lock_daemon_runtime(&runtime, "progressive_migration_select");
                let rows = owned
                    .iter()
                    .map(|key| MigrationCandidateRow {
                        runtime_key: key.clone(),
                        re_resumable: rt
                            .server
                            .live_session_kind(key)
                            .map(session_kind_is_migratable_agent)
                            .unwrap_or(false),
                        migratable: rt.session_is_migratable_now(key),
                        idle_ms: rt.terminals.session_idle_for_ms(key).unwrap_or(0),
                    })
                    .collect::<Vec<_>>();
                select_next_migration_candidate(&rows)
            };
            let Some(runtime_key) = candidate else {
                // Nothing safe to migrate this tick; keep lingering and re-check.
                continue;
            };
            let mut rt = lock_daemon_runtime(&runtime, "progressive_migration_release");
            rt.release_session_for_migration(&home_dir, &runtime_key);
        }
    });
}

/// Kill-switch for the session-preserving self-retire handoff
/// ([[finding-deploy-armed-cold-self-retire-mass-re-resume]]). Setting
/// `YGGTERM_DISABLE_SELF_RETIRE_HANDOFF=1` reverts to the cold-shutdown
/// (re-resume) behaviour with no redeploy, should the handoff ever misbehave
/// live on the most sensitive daemon-lifecycle path.
#[cfg(target_os = "linux")]
fn self_retire_handoff_disabled() -> bool {
    parse_self_retire_handoff_disabled(
        std::env::var("YGGTERM_DISABLE_SELF_RETIRE_HANDOFF").ok().as_deref(),
    )
}

/// Pure parser for the kill-switch env value (set/unset/`0`/`false` = enabled).
/// Split out so it is testable without mutating the shared process environment.
#[cfg(target_os = "linux")]
fn parse_self_retire_handoff_disabled(value: Option<&str>) -> bool {
    match value {
        Some(value) => {
            let value = value.trim();
            !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
        }
        None => false,
    }
}

/// Resolve the replacement binary for a `disk_binary_replaced` retire. Linux
/// appends the literal " (deleted)" suffix to `/proc/self/exe` once the file at
/// our launch path is overwritten, so the un-suffixed path now points at the
/// NEW on-disk binary. Returns `None` when the link is not a replaced-binary
/// link (so the handoff is skipped and the caller cold-shuts-down).
#[cfg(target_os = "linux")]
fn disk_replace_handoff_target(exe_link: &str) -> Option<PathBuf> {
    let path = exe_link.strip_suffix(" (deleted)")?;
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Hand this daemon's live PTYs to a freshly-spawned new-version successor
/// instead of cold-exiting. Routes through the proven `hot_restart` handoff —
/// `force = true` because an autonomous disk-replace retire is an agent-style
/// deploy and we cannot cheaply read the successor's version here (the on-disk
/// binary already IS the new version, so the spawned successor comes up correct).
/// Returns `true` only when the RPC was accepted; the caller then lingers as the
/// preserved owner and must NOT shutdown.
#[cfg(target_os = "linux")]
fn attempt_self_retire_preserving_handoff(
    endpoint: &ServerEndpoint,
    home_dir: &Path,
    exe_link: &str,
    owned: &[String],
) -> bool {
    let Some(new_exe) = disk_replace_handoff_target(exe_link) else {
        return false;
    };
    if !new_exe.is_file() {
        append_trace_event(
            home_dir,
            "daemon",
            "lifecycle",
            "daemon_self_retire_handoff_skipped",
            serde_json::json!({
                "reason": "replacement_binary_missing",
                "new_exe": new_exe.display().to_string(),
            }),
        );
        return false;
    }
    match hot_restart_detailed(
        endpoint,
        &new_exe,
        None,
        None,
        Some("disk_binary_replaced_self_retire"),
        true,
    ) {
        Ok(result) => {
            let outcome = match &result {
                HotRestartResult::Handoff { .. } => "preserved_owner_handoff",
                HotRestartResult::Restarting { .. } => "restart_no_owned_runtime",
            };
            append_trace_event(
                home_dir,
                "daemon",
                "lifecycle",
                "daemon_self_retire_handoff_ok",
                serde_json::json!({
                    "new_exe": new_exe.display().to_string(),
                    "outcome": outcome,
                    "owned_terminal_session_count": owned.len(),
                    "owned_terminal_session_keys": owned,
                    "message": result.message(),
                    "current_version": SERVER_PROTOCOL_VERSION,
                    "current_pid": std::process::id(),
                }),
            );
            true
        }
        Err(error) => {
            append_trace_event(
                home_dir,
                "daemon",
                "lifecycle",
                "daemon_self_retire_handoff_failed",
                serde_json::json!({
                    "new_exe": new_exe.display().to_string(),
                    "error": error.to_string(),
                    "current_pid": std::process::id(),
                }),
            );
            false
        }
    }
}

/// Socket-litter cleanup (run #19): unlink versioned `server-*.sock` files
/// with no listener behind them. ~600 dead sockets had accumulated in
/// ~/.yggterm, and every cross-daemon walk (GUI startup candidate scan,
/// dedup prune, takeover, retire CLI) probed each one. Unlinking a dead
/// socket is safe — a daemon that starts later removes/rebinds its own path.
#[cfg(unix)]
fn cleanup_dead_versioned_server_sockets(home_dir: &Path, excluded: &ServerEndpoint) -> usize {
    let mut removed = 0_usize;
    for path in versioned_server_status_probe_paths_excluding_endpoint(home_dir, excluded) {
        let endpoint = ServerEndpoint::UnixSocket(path.clone());
        if status(&endpoint).is_ok() {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        append_trace_event(
            home_dir,
            "daemon",
            "lifecycle",
            "dead_server_sockets_removed",
            serde_json::json!({ "removed": removed }),
        );
    }
    removed
}

#[cfg(not(unix))]
fn cleanup_dead_versioned_server_sockets(_home_dir: &Path, _excluded: &ServerEndpoint) -> usize {
    0
}

/// GATE #8 startup hook: run the superseded-daemon takeover once, off the
/// accept path, under the runtime lock (see
/// [`DaemonRuntime::takeover_superseded_daemon_state`]), then sweep dead
/// socket litter while the walk is warm.
fn spawn_superseded_daemon_takeover(runtime: Arc<Mutex<DaemonRuntime>>) {
    if let Err(error) = std::thread::Builder::new()
        .name("yggterm-superseded-takeover".to_string())
        .spawn(move || {
            let home_dir = {
                let mut rt = lock_daemon_runtime(&runtime, "superseded_daemon_takeover");
                rt.takeover_superseded_daemon_state();
                rt.store.home_dir().to_path_buf()
            };
            let current_endpoint = default_endpoint(&home_dir);
            let _ = cleanup_dead_versioned_server_sockets(&home_dir, &current_endpoint);
        })
    {
        warn!(error=%error, "failed to spawn superseded daemon takeover");
    }
}

fn spawn_active_terminal_prewarm(
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
    home_dir: PathBuf,
) {
    std::thread::spawn(move || {
        let (active_terminal_path, paths) = {
            let runtime = lock_daemon_runtime(&runtime, "startup_prewarm_active_path");
            let mut paths = Vec::<String>::new();
            let mut active_terminal_path = None::<String>;
            if runtime.server.active_view_mode() == WorkspaceViewMode::Terminal
                && runtime.server.active_session_supports_terminal()
            {
                if let Some(path) = runtime.server.active_session_path().map(ToOwned::to_owned) {
                    active_terminal_path = Some(path.clone());
                    paths.push(path);
                }
            }
            for session in runtime.server.live_sessions() {
                if runtime
                    .server
                    .session_supports_terminal(&session.session_path)
                    && !paths.iter().any(|path| path == &session.session_path)
                {
                    paths.push(session.session_path);
                }
            }
            (active_terminal_path, paths)
        };
        if paths.is_empty() {
            return;
        }
        append_trace_event(
            &home_dir,
            "daemon",
            "startup_prewarm",
            "plan",
            serde_json::json!({
                "active_terminal_path": active_terminal_path.as_deref(),
                "paths": paths.clone(),
            }),
        );
        for path in paths {
            if let Some(reason) = startup_prewarm_skip_reason(&path) {
                append_trace_event(
                    &home_dir,
                    "daemon",
                    "startup_prewarm",
                    "skip",
                    serde_json::json!({
                        "path": path,
                        "reason": reason,
                    }),
                );
                continue;
            }
            let seed_remote_snapshot =
                startup_prewarm_should_seed_remote_snapshot(&path, active_terminal_path.as_deref());
            append_trace_event(
                &home_dir,
                "daemon",
                "startup_prewarm",
                "begin",
                serde_json::json!({
                    "path": path,
                    "seed_remote_snapshot": seed_remote_snapshot,
                }),
            );
            let outcome = {
                let mut runtime = lock_daemon_runtime(&runtime, "startup_prewarm_ensure");
                runtime.ensure_terminal_for_path_for_startup_prewarm(&path, seed_remote_snapshot)
            };
            match outcome {
                Ok(_) => {
                    mark_daemon_activity(last_activity_ms.as_ref());
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "startup_prewarm",
                        "end",
                        serde_json::json!({ "path": path }),
                    );
                }
                Err(error) => {
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "startup_prewarm",
                        "error",
                        serde_json::json!({
                            "path": path,
                            "error": error.to_string(),
                        }),
                    );
                    warn!(path=%path, error=%error, "daemon startup terminal prewarm failed");
                }
            }
        }
    });
}

/// Milliseconds of accumulated suspend time: CLOCK_BOOTTIME minus
/// CLOCK_MONOTONIC. Constant while the machine is awake; jumps by the length
/// of a suspend the moment it ends. See the suspend/wake watcher in
/// `run_daemon`.
#[cfg(target_os = "linux")]
fn suspend_clock_gap_ms() -> Option<i64> {
    fn clock_ms(clock: libc::clockid_t) -> Option<i64> {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        (unsafe { libc::clock_gettime(clock, &mut ts) } == 0)
            .then(|| ts.tv_sec as i64 * 1_000 + ts.tv_nsec as i64 / 1_000_000)
    }
    Some(clock_ms(libc::CLOCK_BOOTTIME)? - clock_ms(libc::CLOCK_MONOTONIC)?)
}

pub fn run_daemon(endpoint: &ServerEndpoint, runtime: GhosttyHostSupport) -> Result<()> {
    // Force both boot-time statics HERE, so uptime measures the process and the running
    // build id is the binary we were launched from — before any deploy can overwrite it
    // on disk underneath us.
    LazyLock::force(&DAEMON_STARTED_AT_MS);
    LazyLock::force(&DAEMON_RUNNING_BUILD_ID);
    let runtime = Arc::new(Mutex::new(DaemonRuntime::load(runtime)?));
    let last_activity_ms = Arc::new(AtomicU64::new(current_millis_u64()));
    let idle_shutdown_ms = daemon_idle_shutdown_ms();
    let terminal_idle_trim_after_ms = daemon_terminal_idle_trim_after_ms();
    let home_dir = {
        let runtime = lock_daemon_runtime(&runtime, "run_daemon_home_dir");
        runtime.store.home_dir().to_path_buf()
    };
    append_trace_event(
        &home_dir,
        "daemon",
        "lifecycle",
        "run_begin",
        serde_json::json!({
            "endpoint": format!("{endpoint:?}"),
        }),
    );
    {
        // Perf incident monitor: catch the RANDOM "fan gets angry" flares the user
        // can't predict. Every 30s, summarize the last 60s of perf telemetry and, if
        // it looks like a load incident (title-regen loop / a span monopolizing /
        // a stall), append a durable snapshot to perf-incidents.jsonl (kept for weeks,
        // debounced 5min) — so the data is still there when reported after the fact.
        let home_dir = home_dir.clone();
        let runtime = runtime.clone();
        std::thread::Builder::new()
            .name("yggterm-perf-incident-monitor".to_string())
            .spawn(move || {
                let mut last_incident_ms = 0u64;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(PERF_INCIDENT_MONITOR_MS));
                    let now_ms = current_millis_u64();
                    let extra = {
                        let runtime = lock_daemon_runtime(&runtime, "perf_incident_extra");
                        let status = runtime.status();
                        serde_json::json!({
                            "owned_terminal_session_count": status.owned_terminal_session_count,
                            "terminal_session_count": status.terminal_session_count,
                        })
                    };
                    last_incident_ms = yggterm_core::record_perf_incident_if_hot(
                        &home_dir,
                        PERF_INCIDENT_WINDOW_MS,
                        now_ms,
                        last_incident_ms,
                        extra,
                    );
                }
            })
            .ok();
    }
    // Suspend/wake watcher: CLOCK_BOOTTIME advances across a suspend while
    // CLOCK_MONOTONIC does not, so a jump in their difference is a precise,
    // dependency-free "the machine just woke" signal. On wake every ssh-carried
    // bridge is dead-but-hung (dead TCP; ServerAlive would take ~45s to notice
    // — the "reconnect after wake is not instant" complaint). Kill + respawn
    // them immediately so recovery costs one ssh handshake instead of a
    // keepalive timeout. See [[finding-sleep-wake-ssh-recovery]].
    #[cfg(target_os = "linux")]
    {
        const WAKE_POLL_MS: u64 = 2_000;
        const WAKE_SUSPEND_THRESHOLD_MS: i64 = 10_000;
        let runtime = runtime.clone();
        let last_activity_ms = last_activity_ms.clone();
        let home_dir = home_dir.clone();
        std::thread::Builder::new()
            .name("yggterm-suspend-wake-watcher".to_string())
            .spawn(move || {
                let mut previous_gap = suspend_clock_gap_ms();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(WAKE_POLL_MS));
                    let Some(gap) = suspend_clock_gap_ms() else {
                        continue;
                    };
                    let jump = previous_gap.map(|prev| gap - prev).unwrap_or(0);
                    previous_gap = Some(gap);
                    if jump < WAKE_SUSPEND_THRESHOLD_MS {
                        continue;
                    }
                    let respawned = {
                        let mut runtime = lock_daemon_runtime(&runtime, "suspend_wake_recovery");
                        runtime.terminals.respawn_ssh_carried_sessions()
                    };
                    mark_daemon_activity(&last_activity_ms);
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "suspend_wake",
                        "bridges_respawned",
                        serde_json::json!({
                            "suspend_ms": jump,
                            "respawned": respawned
                                .iter()
                                .map(|(key, ok)| serde_json::json!({ "path": key, "ok": ok }))
                                .collect::<Vec<_>>(),
                        }),
                    );
                }
            })
            .ok();
    }
    match RemoteRuntimeRegistry::open(&home_dir) {
        Ok(registry) => append_trace_event(
            &home_dir,
            "daemon",
            "remote_runtime",
            "bootstrap",
            serde_json::json!({
                "db_path": registry.paths().db_path.display().to_string(),
                "sessions_dir": registry.paths().sessions_dir.display().to_string(),
            }),
        ),
        Err(error) => warn!(error=%error, "failed to initialize remote runtime registry"),
    }
    // The chore thread ALWAYS runs: the CC title sync inside it is a cheap
    // JSONL read that every daemon (local GUI host and remote host daemons
    // alike) must perform — CC's JSONL is the title SSOT. Only the LLM
    // title/summary GENERATION half stays behind the env opt-in. (The whole
    // chore used to sit behind this env, which nothing set — so the CC title
    // sync shipped as dead code; root cause of "CC titles never update".)
    {
        let generation_enabled = daemon_background_copy_chore_enabled();
        if !generation_enabled {
            append_trace_event(
                &home_dir,
                "daemon",
                "background_copy",
                "generation_disabled",
                serde_json::json!({
                    "env": ENV_YGGTERM_ENABLE_BACKGROUND_COPY_CHORE,
                }),
            );
        }
        let runtime = runtime.clone();
        let last_activity_ms = last_activity_ms.clone();
        std::thread::spawn(move || {
            let mut sleep_ms = BACKGROUND_COPY_CHORE_MS;
            let mut remote_cc_confirmed: HashSet<String> = HashSet::new();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
                run_preserved_owner_revalidation_if_due(&runtime);
                match run_background_copy_chore(
                    &runtime,
                    generation_enabled,
                    &mut remote_cc_confirmed,
                ) {
                    Ok(update_count) => {
                        if update_count > 0 {
                            mark_daemon_activity(&last_activity_ms);
                            sleep_ms = BACKGROUND_COPY_CHORE_MS;
                        } else {
                            sleep_ms =
                                (sleep_ms.saturating_mul(2)).min(BACKGROUND_COPY_MAX_IDLE_CHORE_MS);
                        }
                    }
                    Err(error) => {
                        sleep_ms =
                            (sleep_ms.saturating_mul(2)).min(BACKGROUND_COPY_MAX_IDLE_CHORE_MS);
                        warn!(error=%error, "daemon background copy chore failed");
                    }
                }
            }
        });
    }
    if remote_codex_identity_poll_enabled() {
        let runtime = runtime.clone();
        let last_activity_ms = last_activity_ms.clone();
        std::thread::spawn(move || {
            let mut attempts: HashMap<String, u32> = HashMap::new();
            let mut sleep_ms = REMOTE_CODEX_IDENTITY_POLL_MS;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
                match run_remote_codex_identity_poll_chore(&runtime, &mut attempts) {
                    Ok(rebinds) if rebinds > 0 => {
                        mark_daemon_activity(&last_activity_ms);
                        sleep_ms = REMOTE_CODEX_IDENTITY_POLL_MS;
                    }
                    Ok(_) => {
                        sleep_ms = (sleep_ms.saturating_mul(2))
                            .min(REMOTE_CODEX_IDENTITY_POLL_MAX_IDLE_MS);
                    }
                    Err(error) => {
                        sleep_ms = (sleep_ms.saturating_mul(2))
                            .min(REMOTE_CODEX_IDENTITY_POLL_MAX_IDLE_MS);
                        warn!(error = %error, "daemon remote codex identity poll chore failed");
                    }
                }
            }
        });
    } else {
        append_trace_event(
            &home_dir,
            "daemon",
            "remote_codex_identity_poll",
            "disabled",
            serde_json::json!({
                "env": ENV_YGGTERM_DISABLE_REMOTE_CODEX_IDENTITY_POLL,
            }),
        );
    }
    {
        let runtime = runtime.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(BACKGROUND_COPY_CHORE_MS));
                match trim_terminal_buffers(&runtime, terminal_idle_trim_after_ms) {
                    Ok(summary) if summary.trimmed_sessions > 0 => {
                        let home_dir = {
                            let runtime =
                                lock_daemon_runtime(&runtime, "trim_terminal_buffers_home_dir");
                            runtime.store.home_dir().to_path_buf()
                        };
                        append_trace_event(
                            &home_dir,
                            "daemon",
                            "terminal_buffers",
                            "idle_trim",
                            serde_json::json!({
                                "trimmed_sessions": summary.trimmed_sessions,
                                "reclaimed_bytes": summary.reclaimed_bytes,
                                "before_sessions": summary.before.session_count,
                                "before_chunks": summary.before.retained_chunks,
                                "before_bytes": summary.before.retained_bytes,
                                "after_sessions": summary.after.session_count,
                                "after_chunks": summary.after.retained_chunks,
                                "after_bytes": summary.after.retained_bytes,
                                "idle_trim_after_ms": terminal_idle_trim_after_ms,
                            }),
                        );
                    }
                    Ok(_) => {}
                    Err(error) => warn!(error=%error, "daemon terminal buffer trim chore failed"),
                }
            }
        });
    }

    #[cfg(unix)]
    if let ServerEndpoint::UnixSocket(path) = endpoint {
        let Some(daemon_socket_lock) = try_acquire_daemon_socket_lock(path, &home_dir)? else {
            info!(
                path = %path.display(),
                "another yggterm daemon owns the socket bind lock"
            );
            return Ok(());
        };
        if server_socket_path_lexists(path)
            && ping(&ServerEndpoint::UnixSocket(path.clone())).is_ok()
        {
            append_trace_event(
                &home_dir,
                "daemon",
                "lifecycle",
                "existing_socket_owner_reused",
                serde_json::json!({
                    "endpoint": path.display().to_string(),
                    "pid": std::process::id(),
                }),
            );
            info!(
                path = %path.display(),
                "existing yggterm daemon answered before bind"
            );
            return Ok(());
        }
        if server_socket_path_lexists(path) {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) => {
                    warn!(path=%path.display(), error=%error, "failed to remove stale server socket")
                }
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating server socket dir {}", parent.display()))?;
        }
        let listener = std::os::unix::net::UnixListener::bind(path)
            .with_context(|| format!("binding server socket {}", path.display()))?;
        refresh_legacy_server_socket_aliases(path);
        listener
            .set_nonblocking(true)
            .context("setting daemon unix listener nonblocking")?;
        let host = {
            let runtime = lock_daemon_runtime(&runtime, "run_daemon_unix_host");
            runtime.support.kind.as_str().to_string()
        };
        info!(path=%path.display(), host=%host, "yggterm server daemon listening");
        spawn_active_terminal_prewarm(runtime.clone(), last_activity_ms.clone(), home_dir.clone());
        #[cfg(target_os = "linux")]
        if let Ok(current_exe) = std::env::current_exe() {
            spawn_legacy_linux_daemon_cleanup(endpoint.clone(), current_exe, home_dir.clone());
        }
        // Per [[bug-class-old-daemon-never-retires]]: auto-poll the on-disk
        // binary every 60s. When the running daemon's executable has been
        // replaced (Linux marks /proc/self/exe with the literal "(deleted)"
        // suffix once the inode is unlinked), the on-disk binary is a
        // newer version. Send self a Shutdown RPC so the main loop exits
        // cleanly. The next client connection spawns a fresh daemon from
        // the new disk binary. Without this poll, old daemons accumulate
        // for days until the user runs `server retire-stale-daemons`.
        #[cfg(target_os = "linux")]
        spawn_disk_binary_version_poll(endpoint.clone(), home_dir.clone(), runtime.clone());
        spawn_superseded_daemon_takeover(runtime.clone());
        let (client_outcome_tx, client_outcome_rx) =
            std::sync::mpsc::channel::<Result<DaemonRequestOutcome>>();
        let mut restart_after_exit = None::<PathBuf>;
        loop {
            if drain_unix_client_outcomes(&client_outcome_rx, &mut restart_after_exit) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    spawn_unix_client_handler(
                        stream,
                        runtime.clone(),
                        last_activity_ms.clone(),
                        client_outcome_tx.clone(),
                    );
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if daemon_should_idle_shutdown(
                        &home_dir,
                        endpoint,
                        last_activity_ms.as_ref(),
                        idle_shutdown_ms,
                        lock_daemon_runtime(&runtime, "daemon_idle_terminal_count")
                            .terminals
                            .stats()
                            .session_count,
                    ) {
                        append_trace_event(
                            &home_dir,
                            "daemon",
                            "lifecycle",
                            "idle_shutdown",
                            serde_json::json!({ "idle_shutdown_ms": idle_shutdown_ms }),
                        );
                        info!(path=%path.display(), idle_shutdown_ms, "yggterm daemon idle shutdown");
                        break;
                    }
                    let _ = wait_for_listener_ready(listener.as_raw_fd(), DAEMON_ACCEPT_POLL_MS)?;
                }
                Err(error) => return Err(error).context("accepting daemon client"),
            }
        }
        let restart_executable = restart_after_exit.take();
        append_trace_event(
            &home_dir,
            "daemon",
            "lifecycle",
            "run_end",
            serde_json::json!({
                "endpoint": format!("{endpoint:?}"),
                "transport": "unix",
                "hot_restart": restart_executable.is_some(),
            }),
        );
        drop(listener);
        if let Some(executable) = restart_executable {
            if let Err(error) = fs::remove_file(path)
                && error.kind() != ErrorKind::NotFound
            {
                warn!(
                    path = %path.display(),
                    error = %error,
                    "failed to remove unix socket before hot restart"
                );
            }
            drop(daemon_socket_lock);
            if let Err(error) = spawn_hot_restart_daemon_process(&executable, endpoint) {
                append_trace_event(
                    &home_dir,
                    "daemon",
                    "lifecycle",
                    "hot_restart_spawn_failed",
                    serde_json::json!({
                        "daemon_executable": executable.display().to_string(),
                        "error": error.to_string(),
                    }),
                );
                warn!(
                    daemon_executable = %executable.display(),
                    error = %error,
                    "failed to spawn hot restart daemon"
                );
            }
        } else {
            drop(daemon_socket_lock);
        }
        return Ok(());
    }

    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            bail!(
                "unix sockets are unsupported on this platform: {}",
                path.display()
            )
        }
        ServerEndpoint::Tcp { host, port } => {
            let listener = std::net::TcpListener::bind((host.as_str(), *port))
                .with_context(|| format!("binding server tcp endpoint {}:{}", host, port))?;
            listener
                .set_nonblocking(true)
                .context("setting daemon tcp listener nonblocking")?;
            let host_kind = {
                let runtime = lock_daemon_runtime(&runtime, "run_daemon_tcp_host");
                runtime.support.kind.as_str().to_string()
            };
            info!(host=%host, port, host_kind=%host_kind, "yggterm server daemon listening");
            #[cfg(target_os = "linux")]
            if let Ok(current_exe) = std::env::current_exe() {
                spawn_legacy_linux_daemon_cleanup(endpoint.clone(), current_exe, home_dir.clone());
            }
            spawn_active_terminal_prewarm(
                runtime.clone(),
                last_activity_ms.clone(),
                home_dir.clone(),
            );
            let mut restart_after_exit = None::<PathBuf>;
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let runtime = runtime.clone();
                        let last_activity_ms = last_activity_ms.clone();
                        match handle_tcp_stream(stream, runtime, last_activity_ms) {
                            Ok(outcome) => {
                                if let Some(executable) = outcome.restart_executable {
                                    restart_after_exit = Some(executable);
                                }
                                if outcome.should_shutdown {
                                    break;
                                }
                            }
                            Err(error) => {
                                warn!(error=%format!("{error:#}"), "daemon request failed")
                            }
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if daemon_should_idle_shutdown(
                            &home_dir,
                            endpoint,
                            last_activity_ms.as_ref(),
                            idle_shutdown_ms,
                            lock_daemon_runtime(&runtime, "daemon_idle_terminal_count")
                                .terminals
                                .stats()
                                .session_count,
                        ) {
                            append_trace_event(
                                &home_dir,
                                "daemon",
                                "lifecycle",
                                "idle_shutdown",
                                serde_json::json!({ "idle_shutdown_ms": idle_shutdown_ms }),
                            );
                            info!(host=%host, port, idle_shutdown_ms, "yggterm daemon idle shutdown");
                            break;
                        }
                        #[cfg(unix)]
                        {
                            let _ = wait_for_listener_ready(
                                listener.as_raw_fd(),
                                DAEMON_ACCEPT_POLL_MS,
                            )?;
                        }
                        #[cfg(not(unix))]
                        {
                            std::thread::sleep(std::time::Duration::from_millis(
                                DAEMON_ACCEPT_POLL_MS,
                            ));
                        }
                    }
                    Err(error) => return Err(error).context("accepting daemon client"),
                }
            }
            let restart_executable = restart_after_exit.take();
            append_trace_event(
                &home_dir,
                "daemon",
                "lifecycle",
                "run_end",
                serde_json::json!({
                    "endpoint": format!("{endpoint:?}"),
                    "transport": "tcp",
                    "hot_restart": restart_executable.is_some(),
                }),
            );
            drop(listener);
            if let Some(executable) = restart_executable {
                if let Err(error) = spawn_hot_restart_daemon_process(&executable, endpoint) {
                    append_trace_event(
                        &home_dir,
                        "daemon",
                        "lifecycle",
                        "hot_restart_spawn_failed",
                        serde_json::json!({
                            "daemon_executable": executable.display().to_string(),
                            "error": error.to_string(),
                        }),
                    );
                    warn!(
                        daemon_executable = %executable.display(),
                        error = %error,
                        "failed to spawn hot restart daemon"
                    );
                }
            }
            Ok(())
        }
    }
}

fn lock_daemon_runtime<'a>(
    runtime: &'a Arc<Mutex<DaemonRuntime>>,
    label: &'static str,
) -> MutexGuard<'a, DaemonRuntime> {
    match runtime.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            append_trace_event(
                guard.store.home_dir(),
                "daemon",
                "lifecycle",
                "runtime_lock_poisoned",
                serde_json::json!({
                    "label": label,
                }),
            );
            warn!(label, "daemon runtime lock poisoned; recovering");
            guard
        }
    }
}

fn mark_remote_machine_refresh_queued(
    in_flight: &mut HashSet<String>,
    machine_key: &str,
    should_spawn: bool,
) -> RemoteMachineRefreshQueueStatus {
    if !should_spawn {
        return RemoteMachineRefreshQueueStatus::Loopback;
    }
    if in_flight.insert(machine_key.to_string()) {
        RemoteMachineRefreshQueueStatus::Spawn
    } else {
        RemoteMachineRefreshQueueStatus::AlreadyInFlight
    }
}

fn daemon_queue_remote_machine_refresh(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    machine_key: String,
    request_name: &'static str,
) -> ServerResponse {
    let normalized_machine_key = machine_key.trim().to_string();
    let (target, home_dir, response, should_spawn) = {
        let mut runtime = lock_daemon_runtime(runtime, "queue_remote_machine_refresh");
        if daemon_request_trace_enabled(request_name) {
            append_trace_event(
                runtime.store.home_dir(),
                "daemon",
                "request",
                "begin",
                serde_json::json!({ "request": request_name, "mode": "queued" }),
            );
        }
        let target = match runtime
            .server
            .remote_target_for_machine_key(&normalized_machine_key)
        {
            Ok(target) => target,
            Err(error) => {
                if daemon_request_trace_enabled(request_name) {
                    append_trace_event(
                        runtime.store.home_dir(),
                        "daemon",
                        "request",
                        "end",
                        serde_json::json!({
                            "request": request_name,
                            "mode": "queued",
                            "ok": false,
                            "error": error.to_string(),
                        }),
                    );
                }
                return ServerResponse::Error {
                    message: error.to_string(),
                };
            }
        };
        let should_spawn = !YggtermServer::is_loopback_remote_target(&target);
        let queue_status = mark_remote_machine_refresh_queued(
            &mut runtime.remote_machine_refreshes_in_flight,
            &normalized_machine_key,
            should_spawn,
        );
        let response = runtime.snapshot_response(Some(match queue_status {
            RemoteMachineRefreshQueueStatus::Spawn => {
                format!("queued refresh {normalized_machine_key}")
            }
            RemoteMachineRefreshQueueStatus::AlreadyInFlight => {
                format!("refresh already in progress {normalized_machine_key}")
            }
            RemoteMachineRefreshQueueStatus::Loopback => {
                format!("skipped loopback refresh {normalized_machine_key}")
            }
        }));
        if daemon_request_trace_enabled(request_name) {
            append_trace_event(
                runtime.store.home_dir(),
                "daemon",
                "request",
                "end",
                serde_json::json!({
                    "request": request_name,
                    "mode": "queued",
                    "ok": true,
                    "queue_status": format!("{queue_status:?}"),
                }),
            );
        }
        (
            target,
            runtime.store.home_dir().to_path_buf(),
            response,
            queue_status == RemoteMachineRefreshQueueStatus::Spawn,
        )
    };
    if should_spawn {
        let runtime = Arc::clone(runtime);
        let queued_machine_key = normalized_machine_key.clone();
        std::thread::spawn(move || {
            append_trace_event(
                &home_dir,
                "daemon",
                "remote_machine",
                "background_refresh_begin",
                serde_json::json!({
                    "machine_key": queued_machine_key.clone(),
                    "ssh_target": target.ssh_target.clone(),
                    "prefix": target.prefix.clone(),
                }),
            );
            let scan = YggtermServer::scan_remote_machine_refresh(&target);
            let mut runtime = lock_daemon_runtime(&runtime, "background_remote_machine_refresh");
            let outcome = runtime
                .server
                .apply_remote_machine_refresh_scan(&target, scan);
            if let Err(persist_error) = runtime.persist() {
                warn!(
                    machine_key = %queued_machine_key,
                    error = %persist_error,
                    "failed to persist background remote machine refresh"
                );
            }
            runtime
                .remote_machine_refreshes_in_flight
                .remove(&queued_machine_key);
            match outcome {
                Ok(()) => append_trace_event(
                    runtime.store.home_dir(),
                    "daemon",
                    "remote_machine",
                    "background_refresh_end",
                    serde_json::json!({
                        "machine_key": queued_machine_key.clone(),
                        "ok": true,
                    }),
                ),
                Err(error) => {
                    append_trace_event(
                        runtime.store.home_dir(),
                        "daemon",
                        "remote_machine",
                        "background_refresh_end",
                        serde_json::json!({
                            "machine_key": queued_machine_key.clone(),
                            "ok": false,
                            "error": error.to_string(),
                        }),
                    );
                    warn!(
                        machine_key = %queued_machine_key,
                        error = %error,
                        "background remote machine refresh failed"
                    );
                }
            }
        });
    }
    response
}

fn daemon_request_response(
    runtime: &Arc<Mutex<DaemonRuntime>>,
    request: ServerRequest,
) -> ServerResponse {
    let request_name = server_request_name(&request);
    if let ServerRequest::RefreshRemoteMachine { machine_key } = request {
        return daemon_queue_remote_machine_refresh(runtime, machine_key, request_name);
    }
    let mut runtime = lock_daemon_runtime(runtime, "handle_request");
    let home_dir = runtime.store.home_dir().to_path_buf();
    match panic::catch_unwind(AssertUnwindSafe(|| runtime.handle_request(request))) {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => ServerResponse::Error {
            message: error.to_string(),
        },
        Err(payload) => {
            let panic_message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|value| (*value).to_string())
                })
                .unwrap_or_else(|| "unknown panic payload".to_string());
            append_trace_event(
                &home_dir,
                "daemon",
                "request",
                "panic",
                serde_json::json!({
                    "request": request_name,
                    "panic": panic_message,
                }),
            );
            warn!(request = request_name, panic = %panic_message, "daemon request panicked");
            ServerResponse::Error {
                message: format!("daemon request {request_name} panicked: {panic_message}"),
            }
        }
    }
}

#[cfg(unix)]
fn cleanup_legacy_unix_daemons(endpoint: &ServerEndpoint) -> Result<()> {
    let ServerEndpoint::UnixSocket(current_path) = endpoint else {
        return Ok(());
    };
    let Some(home_dir) = current_path.parent() else {
        return Ok(());
    };
    let entries = match fs::read_dir(home_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading daemon socket dir {}", home_dir.display()));
        }
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("server") || !name.ends_with(".sock") || path == *current_path {
            continue;
        }

        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_) => {
                // A reachable versioned daemon may own live terminal runtimes that the
                // freshly started app has not reclaimed yet. Leave it running; Linux
                // process cleanup applies the stricter ownership checks below.
            }
            Err(_) => {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn daemon_orphan_reap_after_ms() -> u64 {
    std::env::var("YGGTERM_DAEMON_ORPHAN_REAP_AFTER_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_ORPHAN_DAEMON_REAP_AFTER_MS)
}

#[cfg(target_os = "linux")]
fn linux_proc_pid_age_ms(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.split_once(") ")?.1;
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let start_ticks = fields.get(19)?.parse::<u64>().ok()?;
    let uptime = fs::read_to_string("/proc/uptime").ok()?;
    let uptime_secs = uptime.split_whitespace().next()?.parse::<f64>().ok()?;
    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if clk_tck <= 0 {
        return None;
    }
    let uptime_ticks = (uptime_secs * clk_tck as f64).floor() as u64;
    Some(
        uptime_ticks
            .saturating_sub(start_ticks)
            .saturating_mul(1_000)
            / (clk_tck as u64),
    )
}

#[cfg(target_os = "linux")]
fn linux_yggterm_home_from_environ_bytes(environ: &[u8]) -> Option<PathBuf> {
    let mut home = None::<PathBuf>;
    for part in environ
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
    {
        let item = String::from_utf8_lossy(part);
        if let Some(value) = item.strip_prefix("YGGTERM_HOME=") {
            return Some(PathBuf::from(value));
        }
        if let Some(value) = item.strip_prefix("HOME=") {
            home = Some(PathBuf::from(value));
        }
    }
    home.map(|path| path.join(".yggterm"))
}

#[cfg(target_os = "linux")]
fn linux_proc_yggterm_home(proc_path: &Path) -> Option<PathBuf> {
    let environ = fs::read(proc_path.join("environ")).ok()?;
    linux_yggterm_home_from_environ_bytes(&environ)
}

#[cfg(target_os = "linux")]
fn linux_yggterm_server_process_is_live_bridge(parts: &[String]) -> bool {
    parts.first().is_some_and(|argv0| argv0.contains("yggterm"))
        && parts.windows(3).any(|window| {
            window[0] == "server"
                && window[1] == "remote"
                && (window[2] == "resume-codex" || window[2] == "start-codex")
        })
}

#[cfg(target_os = "linux")]
fn linux_home_has_live_bridge_process(home: &Path) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else {
            continue;
        };
        if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let proc_path = entry.path();
        if linux_proc_state(&proc_path) == Some('Z') {
            continue;
        }
        if linux_proc_yggterm_home(&proc_path).as_deref() != Some(home) {
            continue;
        }
        let Ok(bytes) = fs::read(proc_path.join("cmdline")) else {
            continue;
        };
        let parts = bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect::<Vec<_>>();
        if linux_yggterm_server_process_is_live_bridge(&parts) {
            return true;
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn linux_preserved_owner_process_ids_for_home(home: Option<&Path>) -> HashSet<u32> {
    let mut owner_pids = HashSet::new();
    let Some(home) = home else {
        return owner_pids;
    };
    let registry = PreservedTerminalOwnerRegistry::load(home);
    let mut probed_endpoints = HashSet::<String>::new();
    for entry in registry.entries {
        if entry.owner_server_pid > 0 {
            owner_pids.insert(entry.owner_server_pid);
        }
        if !probed_endpoints.insert(entry.endpoint.label()) {
            continue;
        }
        if let Ok(status) = status(&entry.endpoint.to_endpoint()) {
            if status.server_pid > 0 {
                owner_pids.insert(status.server_pid);
            }
        }
    }
    owner_pids
}

#[cfg(target_os = "linux")]
fn linux_preserved_owner_runtime_keys_for_home(home: Option<&Path>) -> HashSet<String> {
    let Some(home) = home else {
        return HashSet::new();
    };
    PreservedTerminalOwnerRegistry::load(home)
        .entries
        .into_iter()
        .map(|entry| entry.runtime_key)
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_daemon_runtime_activity_protected_for_cleanup(
    pid: u32,
    runtime_status: Option<&ServerRuntimeStatus>,
    preserved_owner_pids: &HashSet<u32>,
    preserved_owner_runtime_keys: &HashSet<String>,
    owner_registry_guard_active: bool,
    clean_preserved_owner_available: bool,
    current_version_terminal_keys: &HashSet<String>,
    home_has_live_bridge_process: bool,
) -> bool {
    let is_registered_preserved_owner = preserved_owner_pids.contains(&pid);
    let duplicate_current_runtime = runtime_status.is_some_and(|status| {
        linux_runtime_status_duplicates_current_version(status, current_version_terminal_keys)
    });
    if owner_registry_guard_active {
        if is_registered_preserved_owner {
            return true;
        }
        if duplicate_current_runtime {
            return false;
        }
        if let Some(status) = runtime_status {
            if !status.owned_terminal_session_keys.is_empty() {
                let all_owned_keys_already_represented = status
                    .owned_terminal_session_keys
                    .iter()
                    .all(|key| preserved_owner_runtime_keys.contains(key));
                if clean_preserved_owner_available && all_owned_keys_already_represented {
                    return false;
                }
                return true;
            }
            if status.owned_terminal_session_count > 0 {
                return true;
            }
        }
        if clean_preserved_owner_available {
            return false;
        }
        return runtime_status.is_some_and(|status| {
            if !status.owned_terminal_session_keys.is_empty() {
                status
                    .owned_terminal_session_keys
                    .iter()
                    .any(|key| preserved_owner_runtime_keys.contains(key))
            } else {
                status.owned_terminal_session_count > 0
            }
        });
    }
    if let Some(status) = runtime_status {
        let has_terminal_runtime = status.terminal_session_count > 0
            || !status.terminal_session_keys.is_empty()
            || status.owned_terminal_session_count > 0
            || !status.owned_terminal_session_keys.is_empty();
        if has_terminal_runtime {
            return !duplicate_current_runtime;
        }
    }
    home_has_live_bridge_process
}

#[cfg(target_os = "linux")]
fn linux_runtime_status_duplicates_current_version(
    status: &ServerRuntimeStatus,
    current_version_terminal_keys: &HashSet<String>,
) -> bool {
    status.server_version != SERVER_PROTOCOL_VERSION
        && !status.owned_terminal_session_keys.is_empty()
        && status
            .owned_terminal_session_keys
            .iter()
            .all(|key| current_version_terminal_keys.contains(key))
}

#[cfg(target_os = "linux")]
fn linux_current_version_terminal_keys_for_cleanup(
    runtime_status_by_pid: &HashMap<u32, ServerRuntimeStatus>,
) -> HashSet<String> {
    runtime_status_by_pid
        .values()
        .filter(|status| status.server_version == SERVER_PROTOCOL_VERSION)
        .flat_map(|status| status.owned_terminal_session_keys.iter().cloned())
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_cleanup_has_clean_preserved_owner(
    runtime_status_by_pid: &HashMap<u32, ServerRuntimeStatus>,
    preserved_owner_pids: &HashSet<u32>,
    preserved_owner_runtime_keys: &HashSet<String>,
) -> bool {
    if preserved_owner_runtime_keys.is_empty() {
        return false;
    }
    preserved_owner_pids.iter().any(|pid| {
        let Some(status) = runtime_status_by_pid.get(pid) else {
            return false;
        };
        if status.terminal_session_keys.is_empty() {
            return false;
        }
        let terminal_keys = status
            .terminal_session_keys
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        terminal_keys.len() == preserved_owner_runtime_keys.len()
            && preserved_owner_runtime_keys
                .iter()
                .all(|key| terminal_keys.contains(key))
            && status
                .owned_terminal_session_keys
                .iter()
                .all(|key| preserved_owner_runtime_keys.contains(key))
            && status
                .preserved_terminal_owner_keys
                .iter()
                .all(|key| preserved_owner_runtime_keys.contains(key))
    })
}

#[cfg(target_os = "linux")]
fn linux_cleanup_startup_bridge_sidecar_pid(
    runtime_status_by_pid: &HashMap<u32, ServerRuntimeStatus>,
    preserved_owner_pids: &HashSet<u32>,
    preserved_owner_runtime_keys: &HashSet<String>,
    current_pid: u32,
) -> Option<u32> {
    if preserved_owner_runtime_keys.is_empty() {
        return None;
    }
    if linux_cleanup_has_clean_preserved_owner(
        runtime_status_by_pid,
        preserved_owner_pids,
        preserved_owner_runtime_keys,
    ) {
        return None;
    }
    runtime_status_by_pid
        .iter()
        .filter(|(pid, status)| {
            **pid != current_pid
                && !preserved_owner_pids.contains(pid)
                && status.server_version != SERVER_PROTOCOL_VERSION
                && status.owned_terminal_session_count == 0
                && status.owned_terminal_session_keys.is_empty()
                && !status.terminal_session_keys.is_empty()
                && status
                    .terminal_session_keys
                    .iter()
                    .all(|key| preserved_owner_runtime_keys.contains(key))
                && status
                    .preserved_terminal_owner_keys
                    .iter()
                    .all(|key| preserved_owner_runtime_keys.contains(key))
        })
        .max_by_key(|(pid, status)| {
            (
                daemon_status_version_triplet_for_cleanup(&status.server_version),
                status.server_build_id,
                **pid,
            )
        })
        .map(|(pid, _)| *pid)
}

#[cfg(target_os = "linux")]
fn daemon_status_version_triplet_for_cleanup(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let patch = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    (major, minor, patch)
}

#[cfg(target_os = "linux")]
fn linux_proc_state(proc_path: &Path) -> Option<char> {
    let stat = fs::read_to_string(proc_path.join("stat")).ok()?;
    stat.split_once(") ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|state| state.chars().next())
}

#[cfg(target_os = "linux")]
fn terminate_linux_process(pid: u32) {
    unsafe {
        let _ = libc::kill(pid as i32, libc::SIGTERM);
    }
    std::thread::sleep(std::time::Duration::from_millis(120));
    if Path::new(&format!("/proc/{pid}")).exists() {
        unsafe {
            let _ = libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_socket_inode(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.ino())
}

#[cfg(target_os = "linux")]
fn cleanup_legacy_linux_daemon_processes(
    endpoint: &ServerEndpoint,
    current_exe: &Path,
) -> Result<()> {
    let current_home = daemon_logical_home_for_endpoint(endpoint);
    let reap_after_ms = daemon_orphan_reap_after_ms();
    let preserved_owner_pids = linux_preserved_owner_process_ids_for_home(current_home.as_deref());
    let preserved_owner_runtime_keys =
        linux_preserved_owner_runtime_keys_for_home(current_home.as_deref());
    let owner_registry_guard_active =
        !preserved_owner_pids.is_empty() || !preserved_owner_runtime_keys.is_empty();
    let mut runtime_status_by_pid = HashMap::<u32, ServerRuntimeStatus>::new();
    let mut endpoint_by_pid = HashMap::<u32, ServerEndpoint>::new();
    if let Some(home) = current_home.as_deref() {
        for (status_endpoint, status) in reachable_versioned_daemon_statuses(home) {
            if status.server_pid == 0 {
                continue;
            }
            endpoint_by_pid.insert(status.server_pid, status_endpoint);
            runtime_status_by_pid.insert(status.server_pid, status);
        }
    }
    let startup_bridge_sidecar_pid = linux_cleanup_startup_bridge_sidecar_pid(
        &runtime_status_by_pid,
        &preserved_owner_pids,
        &preserved_owner_runtime_keys,
        std::process::id(),
    );
    let clean_preserved_owner_available = linux_cleanup_has_clean_preserved_owner(
        &runtime_status_by_pid,
        &preserved_owner_pids,
        &preserved_owner_runtime_keys,
    );
    let current_version_terminal_keys =
        linux_current_version_terminal_keys_for_cleanup(&runtime_status_by_pid);
    let current_socket_inode = match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => linux_socket_inode(path),
        ServerEndpoint::Tcp { .. } => None,
    };
    let (same_home_daemon_count, oldest_same_home_pid) = if let Some(home) = current_home.as_deref()
    {
        let mut count = 0usize;
        let mut oldest_pid = None::<(u64, u32)>;
        let entries =
            fs::read_dir("/proc").context("reading /proc for same-home daemon counting")?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid_str) = name.to_str() else {
                continue;
            };
            if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }
            let proc_path = entry.path();
            if linux_proc_state(&proc_path) == Some('Z') {
                continue;
            }
            let Ok(bytes) = fs::read(proc_path.join("cmdline")) else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            let parts = bytes
                .split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).to_string())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                continue;
            }
            let is_daemon = parts[0].contains("yggterm")
                && parts.iter().any(|part| part == "server")
                && parts.iter().any(|part| part == "daemon");
            if !is_daemon {
                continue;
            }
            if linux_proc_yggterm_home(&proc_path).as_deref() == Some(home) {
                count += 1;
                let age_ms =
                    linux_proc_pid_age_ms(pid_str.parse::<u32>().unwrap_or(0)).unwrap_or_default();
                let pid = pid_str.parse::<u32>().unwrap_or(0);
                if pid > 0
                    && oldest_pid.is_none_or(|(oldest_age_ms, oldest_pid_value)| {
                        age_ms > oldest_age_ms
                            || (age_ms == oldest_age_ms && pid < oldest_pid_value)
                    })
                {
                    oldest_pid = Some((age_ms, pid));
                }
            }
        }
        (count, oldest_pid.map(|(_, pid)| pid))
    } else {
        (0, None)
    };
    let mut daemon_candidates = 0usize;
    let mut killed_legacy = 0usize;
    let mut killed_orphan = 0usize;
    let mut killed_duplicate_same_home = 0usize;
    let mut skipped_active_client_legacy = 0usize;
    let mut skipped_runtime_activity_legacy = 0usize;
    let mut skipped_startup_bridge_sidecar_legacy = 0usize;
    let mut skipped_cross_home_orphan = 0usize;
    let proc_entries = fs::read_dir("/proc").context("reading /proc for stale daemon cleanup")?;
    for entry in proc_entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else {
            continue;
        };
        if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let pid = pid_str.parse::<u32>().unwrap_or(0);
        if pid == std::process::id() {
            continue;
        }
        let proc_path = entry.path();
        if linux_proc_state(&proc_path) == Some('Z') {
            continue;
        }
        let proc_exe_target = fs::read_link(proc_path.join("exe")).ok();
        let cmdline_path = entry.path().join("cmdline");
        let Ok(bytes) = fs::read(&cmdline_path) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let parts = bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        let argv0 = &parts[0];
        let is_daemon = argv0.contains("yggterm")
            && parts.iter().any(|part| part == "server")
            && parts.iter().any(|part| part == "daemon");
        if !is_daemon {
            continue;
        }
        daemon_candidates += 1;
        let daemon_home = linux_proc_yggterm_home(&proc_path);
        let candidate_endpoint = endpoint_by_pid
            .get(&pid)
            .cloned()
            .or_else(|| daemon_home.as_deref().map(default_endpoint));
        let has_gui_clients = daemon_home
            .as_deref()
            .zip(candidate_endpoint.as_ref())
            .and_then(|(home, endpoint)| {
                active_client_instance_records_for_endpoint_scope(home, endpoint).ok()
            })
            .is_some_and(|records| !records.is_empty());
        let home_has_live_bridge_process = daemon_home
            .as_deref()
            .is_some_and(linux_home_has_live_bridge_process);
        let has_recoverable_runtime_activity = linux_daemon_runtime_activity_protected_for_cleanup(
            pid,
            runtime_status_by_pid.get(&pid),
            &preserved_owner_pids,
            &preserved_owner_runtime_keys,
            owner_registry_guard_active,
            clean_preserved_owner_available,
            &current_version_terminal_keys,
            home_has_live_bridge_process,
        );
        let is_startup_bridge_sidecar = startup_bridge_sidecar_pid == Some(pid);
        let has_clients =
            has_gui_clients || has_recoverable_runtime_activity || is_startup_bridge_sidecar;
        let age_ms = linux_proc_pid_age_ms(pid).unwrap_or_default();
        let is_legacy_binary =
            legacy_daemon_reap_applies_to_home(current_home.as_deref(), daemon_home.as_deref())
                && daemon_binary_is_legacy(current_exe, argv0, proc_exe_target.as_deref());
        let is_legacy_reapable = is_legacy_binary && !has_clients;
        if is_legacy_binary && has_gui_clients {
            skipped_active_client_legacy += 1;
        }
        if is_legacy_binary && !has_gui_clients && has_recoverable_runtime_activity {
            skipped_runtime_activity_legacy += 1;
        }
        if is_legacy_binary && is_startup_bridge_sidecar {
            skipped_startup_bridge_sidecar_legacy += 1;
        }
        let is_cross_home_orphan_candidate = !has_clients
            && age_ms >= reap_after_ms
            && daemon_home.as_deref() != current_home.as_deref();
        if is_cross_home_orphan_candidate {
            skipped_cross_home_orphan += 1;
        }
        let is_orphan_clientless = !has_clients
            && age_ms >= reap_after_ms
            && orphan_daemon_reap_applies_to_home(current_home.as_deref(), daemon_home.as_deref());
        let is_duplicate_same_home = same_home_daemon_count > 1
            && daemon_home.as_deref() == current_home.as_deref()
            && Some(pid) != oldest_same_home_pid
            && age_ms >= DUPLICATE_SAME_HOME_GRACE_MS
            && !has_clients;
        if is_legacy_reapable || is_orphan_clientless || is_duplicate_same_home {
            terminate_linux_process(pid);
            if is_legacy_reapable {
                killed_legacy += 1;
            } else if is_duplicate_same_home {
                killed_duplicate_same_home += 1;
            } else {
                killed_orphan += 1;
            }
        }
    }
    if let Some(home) = current_home.as_deref() {
        append_trace_event(
            home,
            "daemon",
            "cleanup",
            "linux_daemon_sweep",
            serde_json::json!({
                "candidates": daemon_candidates,
                "killed_legacy": killed_legacy,
                "killed_orphan": killed_orphan,
                "killed_duplicate_same_home": killed_duplicate_same_home,
                "skipped_active_client_legacy": skipped_active_client_legacy,
                "skipped_runtime_activity_legacy": skipped_runtime_activity_legacy,
                "skipped_startup_bridge_sidecar_legacy": skipped_startup_bridge_sidecar_legacy,
                "skipped_cross_home_orphan": skipped_cross_home_orphan,
                "same_home_daemon_count": same_home_daemon_count,
                "oldest_same_home_pid": oldest_same_home_pid,
                "reap_after_ms": reap_after_ms,
                "current_socket_inode": current_socket_inode,
                "preserved_owner_guard_active": owner_registry_guard_active,
                "clean_preserved_owner_available": clean_preserved_owner_available,
                "preserved_owner_pid_count": preserved_owner_pids.len(),
                "preserved_owner_runtime_key_count": preserved_owner_runtime_keys.len(),
                "startup_bridge_sidecar_pid": startup_bridge_sidecar_pid,
            }),
        );
    }
    Ok(())
}

fn expect_snapshot(response: ServerResponse) -> Result<(ServerUiSnapshot, Option<String>)> {
    match response {
        ServerResponse::Snapshot { snapshot, message } => Ok((snapshot, message)),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected snapshot response: {:?}", other),
    }
}

fn expect_ack(response: ServerResponse) -> Result<Option<String>> {
    match response {
        ServerResponse::Ack { message } => Ok(message),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected ack response: {:?}", other),
    }
}

fn load_persisted_state(path: &Path) -> Result<Option<PersistedDaemonState>> {
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(path)
        .with_context(|| format!("reading daemon state {}", path.display()))?;
    let state = serde_json::from_str(&json)
        .with_context(|| format!("parsing daemon state {}", path.display()))?;
    Ok(Some(state))
}

fn write_persisted_state(path: &Path, state: &PersistedDaemonState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating daemon state dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(state).context("serializing daemon state")?;
    if path.exists() {
        let backup_path = path.with_file_name(format!(
            "{}.previous.json",
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("server-state")
        ));
        if let Err(error) = fs::copy(path, &backup_path) {
            tracing::warn!(
                path = %path.display(),
                backup_path = %backup_path.display(),
                error = %error,
                "failed to back up daemon state before overwrite"
            );
        }
    }
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, json)
        .with_context(|| format!("writing daemon state temp file {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .or_else(|_| {
            fs::copy(&temp_path, path)?;
            fs::remove_file(&temp_path)?;
            Ok::<(), std::io::Error>(())
        })
        .with_context(|| format!("writing daemon state {}", path.display()))?;
    Ok(())
}

/// The GUI executable file name that sits beside a headless binary
/// (`yggterm-headless` → `yggterm`, preserving a `.exe` suffix on Windows).
/// Used to point a managed install's `active_executable` (which is the GUI
/// executable) at the hot-update target version.
fn companion_gui_executable_file_name(headless: &Path) -> String {
    let name = headless
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("yggterm-headless");
    if name.ends_with(".exe") {
        "yggterm.exe".to_string()
    } else {
        "yggterm".to_string()
    }
}

fn canonical_hot_restart_executable(raw_path: &str) -> Result<PathBuf> {
    if raw_path.trim().is_empty() {
        bail!("hot restart daemon executable path is empty");
    }
    let path = PathBuf::from(raw_path);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving hot restart executable {}", path.display()))?;
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("reading hot restart executable {}", canonical.display()))?;
    if !metadata.is_file() {
        bail!(
            "hot restart executable is not a regular file: {}",
            canonical.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            bail!(
                "hot restart executable is not marked executable: {}",
                canonical.display()
            );
        }
    }
    Ok(canonical)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonRequestOutcome {
    should_shutdown: bool,
    restart_executable: Option<PathBuf>,
}

/// May a booting daemon cold-restore the LIVE sessions in `server-state.json`?
///
/// A lone daemon: yes — that file is its own state. A daemon booting beside a
/// predecessor that is reachable and still owns terminal runtimes: **no**. The
/// predecessor holds the truth in memory, `server-state.json` may predate the
/// user's most recent closes, and restoring from it resurrects sessions the user
/// deliberately closed — rows with no runtime, which then refuse keep-alive.
/// That is exactly what happened on 2026-07-09 (19 sessions came back).
///
/// The one exception is the daemon the predecessor handed off TO: the retiring
/// daemon writes a fresh update-restart snapshot and names its successor's
/// version in the preserved-owner registry. That successor must restore, or the
/// handoff loses every row.
///
/// [[incident-cli-forked-daemon-resurrected-sessions]]
fn may_cold_restore_live_sessions(
    handoff_target_version: Option<&str>,
    our_version: &str,
    other_daemons: &[(u32, usize)],
) -> bool {
    if handoff_target_version.is_some_and(|version| version == our_version) {
        return true;
    }
    !other_daemons
        .iter()
        .any(|(_pid, owned_terminal_session_count)| *owned_terminal_session_count > 0)
}

fn hot_restart_should_defer_for_session_survival(owned_terminal_session_keys: &[String]) -> bool {
    // The daemon only has to stay alive during a hot update when it owns PTY
    // file descriptors. Preserved-owner entries already point at another
    // daemon, and restored metadata alone is not a runtime that this process
    // can keep alive.
    !owned_terminal_session_keys.is_empty()
}

fn runtime_status_owns_all_keys(status: &ServerRuntimeStatus, runtime_keys: &[String]) -> bool {
    if runtime_keys.is_empty() {
        return false;
    }
    let terminal_keys = status
        .owned_terminal_session_keys
        .iter()
        .collect::<HashSet<_>>();
    runtime_keys
        .iter()
        .all(|runtime_key| terminal_keys.contains(runtime_key))
}

fn hot_restart_duplicate_runtime_owner_status_from_statuses<I>(
    expected_version: Option<&str>,
    owned_terminal_session_keys: &[String],
    current_pid: u32,
    statuses: I,
) -> Option<ServerRuntimeStatus>
where
    I: IntoIterator<Item = ServerRuntimeStatus>,
{
    let expected_version = expected_version?;
    statuses
        .into_iter()
        .filter(|status| {
            status.server_pid != current_pid
                && status.server_version == expected_version
                && runtime_status_owns_all_keys(status, owned_terminal_session_keys)
        })
        .max_by_key(|status| (status.server_build_id, status.server_pid))
}

fn hot_restart_duplicate_runtime_owner_status(
    home: &Path,
    expected_version: Option<&str>,
    owned_terminal_session_keys: &[String],
    current_pid: u32,
) -> Option<ServerRuntimeStatus> {
    hot_restart_duplicate_runtime_owner_status_from_statuses(
        expected_version,
        owned_terminal_session_keys,
        current_pid,
        reachable_versioned_daemon_statuses(home)
            .into_iter()
            .map(|(_, status)| status),
    )
}

fn daemon_request_outcome_for_response(
    request: &ServerRequest,
    response: &ServerResponse,
) -> DaemonRequestOutcome {
    if matches!(response, ServerResponse::Error { .. }) {
        return DaemonRequestOutcome {
            should_shutdown: false,
            restart_executable: None,
        };
    }
    if matches!(response, ServerResponse::HotUpdateHandoff { .. }) {
        return DaemonRequestOutcome {
            should_shutdown: false,
            restart_executable: None,
        };
    }

    match request {
        ServerRequest::Shutdown => DaemonRequestOutcome {
            should_shutdown: true,
            restart_executable: None,
        },
        ServerRequest::RetireDaemon { .. } => DaemonRequestOutcome {
            should_shutdown: true,
            restart_executable: None,
        },
        ServerRequest::HotRestart {
            daemon_executable, ..
        } => DaemonRequestOutcome {
            should_shutdown: true,
            restart_executable: Some(
                canonical_hot_restart_executable(daemon_executable)
                    .unwrap_or_else(|_| PathBuf::from(daemon_executable)),
            ),
        },
        _ => DaemonRequestOutcome {
            should_shutdown: false,
            restart_executable: None,
        },
    }
}

fn write_response<W: Write>(writer: &mut W, response: &ServerResponse) -> Result<()> {
    serde_json::to_writer(&mut *writer, response).context("serializing daemon response")?;
    writer
        .write_all(b"\n")
        .context("writing daemon response terminator")?;
    writer.flush().context("flushing daemon response")?;
    Ok(())
}

fn read_request<R: std::io::Read>(reader: R) -> Result<ServerRequest> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .context("reading daemon request")?;
    if bytes == 0 {
        bail!("daemon client closed connection before sending a request");
    }
    serde_json::from_str(line.trim_end()).context("parsing daemon request")
}

#[cfg(unix)]
fn handle_unix_stream(
    mut stream: std::os::unix::net::UnixStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
) -> Result<DaemonRequestOutcome> {
    let request = read_unix_request_with_timeout(
        &stream,
        std::time::Duration::from_millis(DAEMON_CLIENT_REQUEST_READ_TIMEOUT_MS),
    )?;
    mark_daemon_activity(last_activity_ms.as_ref());
    let response = daemon_request_response(&runtime, request.clone());
    let outcome = daemon_request_outcome_for_response(&request, &response);
    write_response(&mut stream, &response)?;
    trim_process_heap_if_supported();
    Ok(outcome)
}

#[cfg(unix)]
fn read_unix_request_with_timeout(
    stream: &std::os::unix::net::UnixStream,
    timeout: std::time::Duration,
) -> Result<ServerRequest> {
    wait_for_unix_daemon_client_request(stream, timeout)?;
    let read_stream = stream.try_clone().context("cloning unix stream")?;
    configure_unix_daemon_client_read_timeout(&read_stream, timeout)?;
    read_request(read_stream)
}

#[cfg(unix)]
fn configure_unix_daemon_client_read_timeout(
    stream: &std::os::unix::net::UnixStream,
    timeout: std::time::Duration,
) -> Result<()> {
    stream
        .set_read_timeout(Some(timeout))
        .context("setting daemon unix client read timeout")
}

#[cfg(unix)]
fn wait_for_unix_daemon_client_request(
    stream: &std::os::unix::net::UnixStream,
    timeout: std::time::Duration,
) -> Result<()> {
    let mut pollfd = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
        revents: 0,
    };
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    loop {
        // SAFETY: `pollfd` points to valid memory for one connected Unix stream descriptor.
        let rc = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if rc > 0 {
            if pollfd.revents & libc::POLLERR != 0 {
                bail!("daemon client socket reported an error before sending a request");
            }
            if pollfd.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                return Ok(());
            }
            continue;
        }
        if rc == 0 {
            bail!("timed out waiting for daemon client request bytes");
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == ErrorKind::Interrupted {
            continue;
        }
        return Err(error).context("polling daemon client request");
    }
}

fn handle_tcp_stream(
    mut stream: std::net::TcpStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
    last_activity_ms: Arc<AtomicU64>,
) -> Result<DaemonRequestOutcome> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(
            DAEMON_CLIENT_REQUEST_READ_TIMEOUT_MS,
        )))
        .context("setting daemon tcp client read timeout")?;
    let request = read_request(stream.try_clone().context("cloning tcp stream")?)?;
    mark_daemon_activity(last_activity_ms.as_ref());
    let response = daemon_request_response(&runtime, request.clone());
    let outcome = daemon_request_outcome_for_response(&request, &response);
    write_response(&mut stream, &response)?;
    trim_process_heap_if_supported();
    Ok(outcome)
}

#[cfg(target_os = "linux")]
fn trim_process_heap_if_supported() {
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(target_os = "linux"))]
fn trim_process_heap_if_supported() {}

fn send_request(endpoint: &ServerEndpoint, request: &ServerRequest) -> Result<ServerResponse> {
    let mut request_bytes =
        serde_json::to_vec(request).context("serializing daemon request payload")?;
    request_bytes.push(b'\n');
    let io_timeout = Some(std::time::Duration::from_millis(
        daemon_request_io_timeout_ms(request),
    ));
    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            let mut stream = std::os::unix::net::UnixStream::connect(path)
                .with_context(|| format!("connecting to {}", path.display()))?;
            stream
                .set_read_timeout(io_timeout)
                .context("setting daemon request read timeout")?;
            stream
                .set_write_timeout(io_timeout)
                .context("setting daemon request write timeout")?;
            stream
                .write_all(&request_bytes)
                .context("writing daemon request")?;
            stream.flush().context("flushing daemon request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading daemon response")?;
            serde_json::from_str(line.trim_end()).with_context(|| {
                let trimmed = line.trim_end();
                let snippet = if trimmed.len() > 240 {
                    format!("{}...", &trimmed[..240])
                } else {
                    trimmed.to_string()
                };
                format!("parsing daemon response: {:?}", snippet)
            })
        }
        ServerEndpoint::Tcp { host, port } => {
            let mut stream = std::net::TcpStream::connect((host.as_str(), *port))
                .with_context(|| format!("connecting to {}:{}", host, port))?;
            stream
                .set_read_timeout(io_timeout)
                .context("setting daemon request read timeout")?;
            stream
                .set_write_timeout(io_timeout)
                .context("setting daemon request write timeout")?;
            stream
                .write_all(&request_bytes)
                .context("writing daemon request")?;
            stream.flush().context("flushing daemon request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading daemon response")?;
            serde_json::from_str(line.trim_end()).with_context(|| {
                let trimmed = line.trim_end();
                let snippet = if trimmed.len() > 240 {
                    format!("{}...", &trimmed[..240])
                } else {
                    trimmed.to_string()
                };
                format!("parsing daemon response: {:?}", snippet)
            })
        }
    }
}

fn daemon_request_io_timeout_ms(request: &ServerRequest) -> u64 {
    match request {
        ServerRequest::StartSshSession { .. }
        | ServerRequest::StartRemoteCodexSession { .. }
        | ServerRequest::StartRemoteClaudeSession { .. }
        | ServerRequest::OpenRemoteSession { .. }
        | ServerRequest::RefreshPreview { .. }
        | ServerRequest::EnsureRemoteRuntimeCodexSession { .. }
        | ServerRequest::StartRemoteRuntimeCodexSession { .. }
        | ServerRequest::EnsureRemoteRuntimeCcSession { .. }
        | ServerRequest::StartRemoteRuntimeCcSession { .. }
        | ServerRequest::RemoveSession { .. }
        | ServerRequest::TerminalRestart {
            force_remote: true, ..
        }
        // Hot-update handoff writes persisted state for every managed session,
        // writes the preserved-owner registry, and spawns the successor daemon
        // before returning `HotUpdateHandoff`. With many managed sessions this
        // routinely exceeds the 10s default, so the client read timed out at
        // ~10s and surfaced a false "reading daemon response" failure even
        // though the handoff actually completed (new daemon bound, registry
        // written, sessions preserved). Give it the long budget so the success
        // response is read instead of false-negatived. See
        // [[finding-hot-update-interrupts-remote-sessions]].
        | ServerRequest::HotRestart { .. } => DAEMON_LONG_REQUEST_IO_TIMEOUT_MS,
        _ => DAEMON_REQUEST_IO_TIMEOUT_MS,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalWriteStrategy {
    LocalRuntime,
    RemoteDirectFallback,
    LocalRuntimeFallback,
}

fn terminal_write_strategy_for_path(
    path: &str,
    local_runtime_running: bool,
) -> TerminalWriteStrategy {
    if local_runtime_running {
        TerminalWriteStrategy::LocalRuntime
    } else if path.trim_start().starts_with("remote-session://") {
        TerminalWriteStrategy::RemoteDirectFallback
    } else {
        TerminalWriteStrategy::LocalRuntimeFallback
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HOT_RESTART_BLOCKER_RECENTLY_ACTIVE, HOT_RESTART_BLOCKER_WORKING, HotRestartBlocker,
        hot_restart_block_reason_summary,
    };

    fn blocker(key: &str, kind: &str, idle_ms: Option<u64>) -> HotRestartBlocker {
        HotRestartBlocker {
            session_key: key.to_string(),
            kind: kind.to_string(),
            idle_ms,
            threshold_ms: 300_000,
        }
    }

    #[test]
    fn no_blockers_means_nothing_is_deferring_the_restart() {
        assert_eq!(hot_restart_block_reason_summary(&[]), None);
    }

    #[test]
    fn a_working_session_is_named_with_what_it_is_doing() {
        let reason = hot_restart_block_reason_summary(&[blocker(
            "local://abc",
            HOT_RESTART_BLOCKER_WORKING,
            Some(0),
        )])
        .expect("a working session defers the restart");
        assert!(reason.contains("local://abc"), "{reason}");
        assert!(reason.contains("working"), "{reason}");
        assert!(!reason.contains("more session"), "one blocker adds no tail: {reason}");
    }

    #[test]
    fn a_recently_active_session_reports_idle_time_in_seconds_not_raw_ms() {
        let reason = hot_restart_block_reason_summary(&[blocker(
            "local://abc",
            HOT_RESTART_BLOCKER_RECENTLY_ACTIVE,
            Some(42_000),
        )])
        .expect("a recently-active session defers the restart");
        assert!(reason.contains("42s"), "raw milliseconds tell the user nothing: {reason}");
        assert!(reason.contains("300s"), "the idle window must be stated: {reason}");
    }

    #[test]
    fn extra_blockers_are_counted_so_a_deferral_is_never_an_unexplained_wait() {
        // Naming only the FIRST blocker is what made this opaque: the user clears that
        // session, the restart still defers, and the panel names a session it never
        // mentioned. The count is the honesty fix.
        let reason = hot_restart_block_reason_summary(&[
            blocker("local://a", HOT_RESTART_BLOCKER_WORKING, Some(0)),
            blocker("local://b", HOT_RESTART_BLOCKER_WORKING, Some(0)),
            blocker("local://c", HOT_RESTART_BLOCKER_RECENTLY_ACTIVE, Some(1_000)),
        ])
        .expect("three blockers defer the restart");
        assert!(reason.contains("local://a"), "{reason}");
        assert!(reason.contains("+2 more session(s)"), "{reason}");
    }

    // `shutdown()` must never make a pre-2.9.66 daemon type `/exit` into the
    // user's prompt. An unparseable version is treated as OLD — the safe
    // direction, because guessing "new" writes into a live prompt.
    #[test]
    fn legacy_daemons_are_never_asked_to_shutdown_by_typing() {
        use super::daemon_shutdown_writes_into_prompts as types;

        for old in ["2.9.63", "2.9.65", "2.9.0", "2.8.99", "1.99.99"] {
            assert!(types(old), "{old} writes into prompts on Shutdown");
        }
        for safe in ["2.9.66", "2.9.67", "2.9.100", "2.10.0", "3.0.0"] {
            assert!(!types(safe), "{safe} signals instead of writing");
        }
        // 2.9.66 is the boundary itself, and a nonsense version fails safe.
        assert!(!types(super::FIRST_VERSION_WITH_SIGNAL_BASED_SHUTDOWN));
        for junk in ["", "dev", "2.x.3", "not-a-version"] {
            assert!(types(junk), "{junk:?} must fail safe (assume old)");
        }
        // Ordering is numeric, not lexicographic: "2.9.9" < "2.9.66" as text.
        assert!(types("2.9.9"), "2.9.9 really is older than 2.9.66");
    }

    // The guard that stops a booting daemon from resurrecting a live
    // predecessor's sessions out of server-state.json. Placed first because it
    // is the one thing standing between a two-daemon window and the 2026-07-09
    // incident (19 closed sessions came back).
    #[test]
    fn cold_restore_of_live_sessions_is_refused_beside_a_live_owner() {
        use super::may_cold_restore_live_sessions as may;

        // Lone daemon: nothing else is reachable, restore is its own state.
        assert!(may(None, "2.9.66", &[]));
        // A reachable peer that owns NO runtimes is not an owner of the truth.
        assert!(may(None, "2.9.66", &[(4242, 0)]));

        // A reachable predecessor still owning runtimes: refuse.
        assert!(!may(None, "2.9.66", &[(4242, 17)]));
        assert!(!may(None, "2.9.66", &[(4242, 0), (4243, 1)]));

        // ...unless we are the successor it handed off TO. Restoring is then
        // mandatory: the retiring daemon wrote a fresh snapshot for us, and
        // skipping it would drop every row.
        assert!(may(Some("2.9.66"), "2.9.66", &[(4242, 17)]));
        // A registry naming some OTHER version is not our handoff.
        assert!(!may(Some("2.9.63"), "2.9.66", &[(4242, 17)]));
    }

    // Progressive migration is the mechanism that drains a handed-off daemon's
    // sessions "all but the busy few". It shipped dormant and never converged;
    // it is now on by default, with the env var demoted to a kill switch.
    #[test]
    fn progressive_migration_defaults_on_and_honours_the_kill_switch() {
        // SAFETY: single-threaded assertions on one process-wide variable; the
        // value is restored before returning.
        let key = "YGGTERM_ENABLE_PROGRESSIVE_MIGRATION";
        let previous = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        assert!(super::progressive_migration_enabled(), "default is ON");
        for off in ["0", "false", "no", "off"] {
            unsafe { std::env::set_var(key, off) };
            assert!(!super::progressive_migration_enabled(), "{off} disables");
        }
        for on in ["1", "true", "yes", "on", "anything-else"] {
            unsafe { std::env::set_var(key, on) };
            assert!(super::progressive_migration_enabled(), "{on} keeps it on");
        }
        match previous {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    use super::{
        MigratableSignals, MigrationCandidateRow, RemoteMachineRefreshQueueStatus,
        SERVER_PROTOCOL_VERSION, apply_terminal_runtime_truth_to_snapshot,
        daemon_background_copy_chore_enabled_from_env, mark_remote_machine_refresh_queued,
        parse_daemon_version_triple, preserved_owner_candidate_for_runtime_key,
        preserved_owner_saved_session_mismatch_should_detach,
        remove_session_should_detach_keep_alive_runtime, select_next_migration_candidate,
        session_is_migratable, session_kind_is_migratable_agent, terminal_reuse_needs_restart,
        terminal_sidebar_snapshot_from_screen,
    };
    use crate::TerminalManager;
    use std::collections::{BTreeMap, HashMap, HashSet};

    const MIGRATE_IDLE_MS: u64 = 45_000;

    fn migratable_idle_session() -> MigratableSignals {
        // The all-clear baseline: idle long enough, no draft, no foreground
        // command, no working footer.
        MigratableSignals {
            activity_idle_ms: Some(60_000),
            has_pending_draft: Some(false),
            foreground_command_running: Some(false),
            screen_shows_working: false,
        }
    }

    #[test]
    fn migratable_when_all_signals_clear() {
        assert!(session_is_migratable(
            &migratable_idle_session(),
            MIGRATE_IDLE_MS
        ));
    }

    #[test]
    fn not_migratable_when_recently_active() {
        let mut sig = migratable_idle_session();
        sig.activity_idle_ms = Some(1_000);
        assert!(!session_is_migratable(&sig, MIGRATE_IDLE_MS));
    }

    #[test]
    fn not_migratable_when_draft_present() {
        let mut sig = migratable_idle_session();
        sig.has_pending_draft = Some(true);
        assert!(
            !session_is_migratable(&sig, MIGRATE_IDLE_MS),
            "a typed-but-unsent draft must protect the session from release"
        );
    }

    #[test]
    fn not_migratable_when_foreground_command_running() {
        let mut sig = migratable_idle_session();
        sig.foreground_command_running = Some(true);
        assert!(
            !session_is_migratable(&sig, MIGRATE_IDLE_MS),
            "a running tty command (incl. silent ones) must block migration"
        );
    }

    #[test]
    fn not_migratable_when_working_footer_present() {
        let mut sig = migratable_idle_session();
        sig.screen_shows_working = true;
        assert!(!session_is_migratable(&sig, MIGRATE_IDLE_MS));
    }

    #[test]
    fn not_migratable_when_any_signal_unavailable() {
        // Safety bias: every ambiguous/unowned signal blocks migration.
        for mutate in [
            |s: &mut MigratableSignals| s.activity_idle_ms = None,
            |s: &mut MigratableSignals| s.has_pending_draft = None,
            |s: &mut MigratableSignals| s.foreground_command_running = None,
        ] {
            let mut sig = migratable_idle_session();
            mutate(&mut sig);
            assert!(
                !session_is_migratable(&sig, MIGRATE_IDLE_MS),
                "an unavailable signal must never be treated as migratable"
            );
        }
    }

    fn migration_row(key: &str, re: bool, mig: bool, idle_ms: u64) -> MigrationCandidateRow {
        MigrationCandidateRow {
            runtime_key: key.to_string(),
            re_resumable: re,
            migratable: mig,
            idle_ms,
        }
    }

    #[test]
    fn migration_picks_oldest_idle_safe_session() {
        let rows = vec![
            migration_row("remote-cc://dev/a", true, true, 60_000),
            migration_row("remote-cc://dev/b", true, true, 120_000),
            migration_row("remote-cc://dev/c", true, true, 90_000),
        ];
        assert_eq!(
            select_next_migration_candidate(&rows).as_deref(),
            Some("remote-cc://dev/b"),
            "the longest-idle safe session migrates first"
        );
    }

    #[test]
    fn migration_skips_non_re_resumable_and_non_migratable() {
        let rows = vec![
            // Longest idle but a plain shell -> not re-resumable, skipped.
            migration_row("local://shell", false, true, 999_000),
            // Re-resumable but not yet migratable (busy/draft), skipped.
            migration_row("remote-cc://dev/busy", true, false, 500_000),
            // The only safe, re-resumable candidate.
            migration_row("remote-cc://dev/ok", true, true, 50_000),
        ];
        assert_eq!(
            select_next_migration_candidate(&rows).as_deref(),
            Some("remote-cc://dev/ok")
        );
    }

    #[test]
    fn migration_yields_nothing_when_no_safe_candidate() {
        let rows = vec![
            migration_row("local://shell", false, true, 999_000),
            migration_row("remote-cc://dev/busy", true, false, 500_000),
        ];
        assert_eq!(select_next_migration_candidate(&rows), None);
    }

    #[test]
    fn migratable_agent_kinds_exclude_plain_shell() {
        assert!(session_kind_is_migratable_agent(SessionKind::Codex));
        assert!(session_kind_is_migratable_agent(SessionKind::ClaudeCode));
        assert!(session_kind_is_migratable_agent(SessionKind::CodexLiteLlm));
        assert!(!session_kind_is_migratable_agent(SessionKind::Shell));
    }

    #[test]
    fn daemon_version_triple_parses_and_orders() {
        assert_eq!(parse_daemon_version_triple("2.8.9"), Some((2, 8, 9)));
        assert_eq!(parse_daemon_version_triple("2.8"), Some((2, 8, 0)));
        assert_eq!(parse_daemon_version_triple("10.0.1"), Some((10, 0, 1)));
        assert_eq!(parse_daemon_version_triple("garbage"), None);
        // Supersession ordering the idle-shutdown gate relies on.
        assert!(parse_daemon_version_triple("2.8.9") > parse_daemon_version_triple("2.8.8"));
        assert!(parse_daemon_version_triple("2.9.0") > parse_daemon_version_triple("2.8.99"));
        assert!(parse_daemon_version_triple("2.8.6") < parse_daemon_version_triple("2.8.8"));
        // The current daemon version must itself be parseable, or supersession
        // detection silently no-ops.
        assert!(parse_daemon_version_triple(SERVER_PROTOCOL_VERSION).is_some());
    }

    fn runtime_status_with_keys(
        owned: &[&str],
        preserved: &[&str],
    ) -> super::ServerRuntimeStatus {
        let mut terminal: Vec<String> = owned
            .iter()
            .chain(preserved.iter())
            .map(|s| s.to_string())
            .collect();
        terminal.sort();
        terminal.dedup();
        serde_json::from_value(serde_json::json!({
            "server_version": "test",
            "host_kind": "test",
            "host_detail": "test",
            "embedded_surface_supported": false,
            "bridge_enabled": false,
            "owned_terminal_session_keys": owned,
            "owned_terminal_session_count": owned.len(),
            "preserved_terminal_owner_keys": preserved,
            "preserved_terminal_owner_count": preserved.len(),
            "terminal_session_keys": terminal,
            "terminal_session_count": terminal.len(),
        }))
        .expect("test ServerRuntimeStatus")
    }

    // Per [[bug-class-old-daemon-never-retires]]: the jojo 2.8.87 lingering
    // duplicate — a stale daemon holding only remote-session records (real
    // PTYs live on the remote hosts) that the current daemon already covers
    // as preserved owners must be retire-eligible.
    #[test]
    fn stale_daemon_with_only_covered_remote_keys_is_retire_safe() {
        use super::stale_daemon_retire_covered_by_current;
        let remote_a = "remote-session://dev/019ca2da";
        let remote_b = "remote-cc://practice/8247344a";
        let current = runtime_status_with_keys(&["local://aaa"], &[remote_a, remote_b]);
        let stale = runtime_status_with_keys(&[remote_a, remote_b], &[]);
        assert!(stale_daemon_retire_covered_by_current(&current, &stale));
    }

    // A stale daemon holding a LOCAL PTY key the current daemon does not own
    // must never be retired (no fd handoff — retiring kills the PTY).
    #[test]
    fn stale_daemon_with_unowned_local_key_is_never_retired() {
        use super::stale_daemon_retire_covered_by_current;
        let local = "local://019df7b2";
        // Current daemon merely PRESERVES the key (does not own a runtime).
        let current = runtime_status_with_keys(&[], &[local]);
        let stale = runtime_status_with_keys(&[local], &[]);
        assert!(!stale_daemon_retire_covered_by_current(&current, &stale));
    }

    // A local key the current daemon actually OWNS means the stale copy is a
    // superseded duplicate runtime — retire-eligible (duplicate-prune class).
    #[test]
    fn stale_daemon_with_local_key_owned_by_current_is_retire_safe() {
        use super::stale_daemon_retire_covered_by_current;
        let local = "local://019df7b2";
        let current = runtime_status_with_keys(&[local], &[]);
        let stale = runtime_status_with_keys(&[local], &[]);
        assert!(stale_daemon_retire_covered_by_current(&current, &stale));
    }

    // Any stale key the current daemon has NO record of would be lost on
    // retire — keep the stale daemon running.
    #[test]
    fn stale_daemon_with_uncovered_key_is_never_retired() {
        use super::stale_daemon_retire_covered_by_current;
        let current = runtime_status_with_keys(&[], &["remote-session://dev/known"]);
        let stale = runtime_status_with_keys(&["remote-session://dev/unknown"], &[]);
        assert!(!stale_daemon_retire_covered_by_current(&current, &stale));
    }

    // Older daemons may report a session COUNT without listing keys — that
    // cannot be verified key-by-key, so it is never retire-eligible.
    #[test]
    fn stale_daemon_with_count_but_no_keys_is_never_retired() {
        use super::stale_daemon_retire_covered_by_current;
        let current = runtime_status_with_keys(&[], &["remote-session://dev/known"]);
        let mut stale = runtime_status_with_keys(&[], &[]);
        stale.terminal_session_count = 3;
        assert!(!stale_daemon_retire_covered_by_current(&current, &stale));
    }

    // Run #19 squish wiring locks: a restart without a client grid must fall
    // back to the persisted grid (the remote restore/bootstrap restart was
    // the squish entry point), and both local resize paths must forward the
    // grid to the REMOTE daemon's PTY (the implicit SIGWINCH→ssh→bridge
    // chain is unreliable).
    #[test]
    fn terminal_restart_and_resize_carry_grid_to_remote_pty() {
        let source = include_str!("daemon.rs");
        let restart_block = source
            .split("ServerRequest::TerminalRestart {")
            .nth(1)
            .and_then(|suffix| suffix.split("ServerRequest::SyncExternalWindow").next())
            .expect("TerminalRestart handler present");
        assert!(
            restart_block.contains(".or_else(|| self.server.session_pty_grid(&path))"),
            "TerminalRestart must fall back to the persisted session grid before DEFAULT 120x36"
        );
        let resize_block = source
            .split("ServerRequest::TerminalResize { path, cols, rows } =>")
            .nth(1)
            .and_then(|suffix| suffix.split("ServerRequest::TerminalRestart").next())
            .expect("TerminalResize handler present");
        assert!(
            resize_block.contains("forward_remote_pty_resize"),
            "TerminalResize must forward the grid to the remote daemon's PTY"
        );
        let ensure_block = source
            .split("RECORD-ON-CREATE (born-at-correct-size invariant)")
            .nth(1)
            .and_then(|suffix| suffix.split("ensure_session_end").next())
            .expect("ensure record-on-create block present");
        assert!(
            ensure_block.contains("self.forward_remote_pty_resize(path, cols, rows);"),
            "EVERY ensure of a remote session must forward the grid to the remote daemon's PTY"
        );
    }

    // The live jojo 2.8.87 shape: a stale daemon with ZERO owned runtimes
    // holding only preserved-owner records (incl. local:// and live:: keys)
    // carries no process resources — retire-safe once the current daemon
    // covers every key.
    #[test]
    fn stale_daemon_with_preserved_only_records_is_retire_safe() {
        use super::stale_daemon_retire_covered_by_current;
        let keys = [
            "live::186f0205-7f4b-4086-a0b1-8a3ed0b83ed6",
            "local://019e74a0-74db-7fc2-8ae6-00ef153f594e",
            "remote-session://dev/019e0339",
        ];
        let current = runtime_status_with_keys(&[], &keys);
        let stale = runtime_status_with_keys(&[], &keys);
        assert!(stale_daemon_retire_covered_by_current(&current, &stale));
        // …but the same shape with one key the current daemon lacks stays.
        let partial = runtime_status_with_keys(&[], &keys[..2]);
        assert!(!stale_daemon_retire_covered_by_current(&partial, &stale));
    }
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::collect_remote_copy_candidates;
    #[cfg(target_os = "linux")]
    use super::legacy_daemon_reap_applies_to_home;
    #[cfg(target_os = "linux")]
    use super::linux_cleanup_startup_bridge_sidecar_pid;
    #[cfg(target_os = "linux")]
    use super::linux_daemon_runtime_activity_protected_for_cleanup;
    #[cfg(target_os = "linux")]
    use super::linux_yggterm_home_from_environ_bytes;
    use super::load_persisted_state;
    #[cfg(target_os = "linux")]
    use super::orphan_daemon_reap_applies_to_home;
    use super::terminal_launch_command_for_path;
    use super::write_persisted_state;
    use super::{match_codex_identities_to_targets, normalize_cwd_for_identity_match};
    use crate::{LocalAgentCliIdentity, RemoteCodexIdentityPollTarget};

    fn poll_target(key: &str, cwd: &str, current_id: &str) -> RemoteCodexIdentityPollTarget {
        RemoteCodexIdentityPollTarget {
            key: key.to_string(),
            ssh_target: "jojo".to_string(),
            ssh_prefix: None,
            cwd: cwd.to_string(),
            current_id: current_id.to_string(),
        }
    }

    fn codex_identity(session_id: &str, cwd: &str) -> LocalAgentCliIdentity {
        LocalAgentCliIdentity {
            kind: "codex".to_string(),
            session_id: session_id.to_string(),
            cwd: cwd.to_string(),
            storage_path: format!("/home/pi/.codex/sessions/{session_id}.jsonl"),
        }
    }

    #[test]
    fn match_codex_identities_rebinds_uuidv4_row_to_running_transcript() {
        let synth = "11111111-2222-4333-8444-555555555555";
        let real = "019ce5d8-c94c-7b62-ae19-3818ae400b65";
        let targets = vec![poll_target("codex-runtime://abc", "/home/pi", synth)];
        let identities = vec![codex_identity(real, "/home/pi")];
        let rebinds = match_codex_identities_to_targets(&targets, &identities);
        assert_eq!(rebinds.len(), 1);
        assert_eq!(rebinds[0].0, "codex-runtime://abc");
        assert_eq!(rebinds[0].1.session_id, real);
    }

    #[test]
    fn match_codex_identities_ignores_trailing_slash_cwd_difference() {
        let targets = vec![poll_target(
            "k",
            "/home/pi/proj/",
            "11111111-2222-4333-8444-555555555555",
        )];
        let identities = vec![codex_identity("019aaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", "/home/pi/proj")];
        assert_eq!(match_codex_identities_to_targets(&targets, &identities).len(), 1);
    }

    #[test]
    fn match_codex_identities_skips_when_id_already_matches() {
        let id = "019ce5d8-c94c-7b62-ae19-3818ae400b65";
        // Row already carries the real id — nothing to rebind.
        let targets = vec![poll_target("k", "/home/pi", id)];
        let identities = vec![codex_identity(id, "/home/pi")];
        assert!(match_codex_identities_to_targets(&targets, &identities).is_empty());
    }

    #[test]
    fn match_codex_identities_does_not_collapse_two_rows_onto_one_transcript() {
        // Two synthesized rows at the same cwd, two running transcripts: each
        // row must claim a distinct transcript, never the same one twice.
        let targets = vec![
            poll_target("k1", "/home/pi", "11111111-1111-4111-8111-111111111111"),
            poll_target("k2", "/home/pi", "22222222-2222-4222-8222-222222222222"),
        ];
        let identities = vec![
            codex_identity("019aaaaaaaaa-aaaa-aaaa-aaaaaaaaaaaa", "/home/pi"),
            codex_identity("019bbbbbbbbb-bbbb-bbbb-bbbbbbbbbbbb", "/home/pi"),
        ];
        let rebinds = match_codex_identities_to_targets(&targets, &identities);
        assert_eq!(rebinds.len(), 2);
        let bound: HashSet<String> = rebinds.iter().map(|(_, id)| id.session_id.clone()).collect();
        assert_eq!(bound.len(), 2, "two rows must bind to two distinct transcripts");
    }

    #[test]
    fn match_codex_identities_skips_when_cwd_has_no_running_codex() {
        let targets = vec![poll_target(
            "k",
            "/home/pi/other",
            "11111111-2222-4333-8444-555555555555",
        )];
        let identities = vec![codex_identity("019aaaaaaaaa-aaaa-aaaa-aaaaaaaaaaaa", "/home/pi")];
        assert!(match_codex_identities_to_targets(&targets, &identities).is_empty());
    }

    #[test]
    fn match_codex_identities_ignores_claude_code_kind() {
        let targets = vec![poll_target(
            "k",
            "/home/pi",
            "11111111-2222-4333-8444-555555555555",
        )];
        let identities = vec![LocalAgentCliIdentity {
            kind: "claude_code".to_string(),
            session_id: "019aaaaaaaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string(),
            cwd: "/home/pi".to_string(),
            storage_path: "/home/pi/.claude/projects/x/y.jsonl".to_string(),
        }];
        assert!(match_codex_identities_to_targets(&targets, &identities).is_empty());
    }

    #[test]
    fn normalize_cwd_match_handles_root_and_trailing_slashes() {
        assert_eq!(normalize_cwd_for_identity_match("/home/pi/"), "/home/pi");
        assert_eq!(normalize_cwd_for_identity_match("  /home/pi  "), "/home/pi");
        assert_eq!(normalize_cwd_for_identity_match("/"), "/");
        assert_eq!(normalize_cwd_for_identity_match(""), "/");
    }
    use super::{
        REMOTE_ATTACH_STARTUP_GRACE_MS, REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS,
        remote_resume_saved_session_mismatch_requires_restart, remote_resume_stale_attach,
    };
    #[cfg(unix)]
    use super::{
        cleanup_legacy_unix_daemons, configure_unix_daemon_client_read_timeout,
        daemon_binary_is_legacy, default_endpoint, parse_versioned_server_socket_name,
        read_request, read_unix_request_with_timeout, server_version_is_strictly_newer,
        unix_socket_path_fits_platform,
        versioned_server_socket_alias_candidates, versioned_socket_alias_is_legacy,
        versioned_socket_alias_points_to_current, versioned_socket_candidate_is_symlink,
        wait_for_unix_daemon_client_request,
    };
    use crate::{
        GhosttyHostSupport, PersistedDaemonState, PersistedLiveSession, RemoteDeployState,
        RemoteMachineHealth, RemoteMachineSnapshot, RemoteScannedSession, ServerRuntimeStatus,
        ServerUiSnapshot, SessionKind, SessionSource, SnapshotMetadataEntry, SnapshotPreview,
        SnapshotSessionView, TerminalBackend, TerminalLaunchPhase, WorkspaceViewMode,
        YggtermServer, remote_scanned_session_path,
    };
    use yggui_contract::UiTheme;

    #[test]
    fn refresh_preview_request_defaults_to_cache_only_and_can_request_full_payload() {
        let decoded: super::ServerRequest =
            serde_json::from_str(r#"{"kind":"refresh_preview","path":"remote-session://dev/a"}"#)
                .expect("legacy refresh request should decode");
        match decoded {
            super::ServerRequest::RefreshPreview {
                path,
                full_remote_payload,
            } => {
                assert_eq!(path, "remote-session://dev/a");
                assert!(!full_remote_payload);
            }
            other => panic!("unexpected request {other:?}"),
        }

        let encoded = serde_json::to_value(super::ServerRequest::RefreshPreview {
            path: "remote-session://dev/a".to_string(),
            full_remote_payload: true,
        })
        .expect("request should encode");
        assert_eq!(encoded["full_remote_payload"], true);
    }

    #[test]
    fn terminal_stream_responses_default_resize_fence_fields_for_hot_update() {
        let response: super::ServerResponse = serde_json::from_str(
            r#"{
                "kind": "terminal_stream",
                "cursor": 7,
                "chunks": [],
                "running": true,
                "runtime_output_seen": true,
                "eof_without_output": false
            }"#,
        )
        .expect("new clients must deserialize old daemon terminal_stream responses");
        let super::ServerResponse::TerminalStream {
            post_resize_output_seen,
            last_resize_seq,
            ..
        } = response
        else {
            panic!("expected terminal_stream response");
        };
        assert!(!post_resize_output_seen);
        assert_eq!(last_resize_seq, 0);

        let response: super::ServerResponse = serde_json::from_str(
            r#"{
                "kind": "terminal_snapshot",
                "text": "",
                "running": true,
                "runtime_output_seen": true
            }"#,
        )
        .expect("new clients must deserialize old daemon terminal_snapshot responses");
        let super::ServerResponse::TerminalSnapshot {
            post_resize_output_seen,
            last_resize_seq,
            ..
        } = response
        else {
            panic!("expected terminal_snapshot response");
        };
        assert!(!post_resize_output_seen);
        assert_eq!(last_resize_seq, 0);

        let response: super::ServerResponse = serde_json::from_str(
            r#"{
                "kind": "terminal_retained_snapshot",
                "text": "",
                "running": true,
                "runtime_output_seen": true
            }"#,
        )
        .expect("new clients must deserialize old daemon terminal_retained_snapshot responses");
        let super::ServerResponse::TerminalRetainedSnapshot {
            post_resize_output_seen,
            last_resize_seq,
            ..
        } = response
        else {
            panic!("expected terminal_retained_snapshot response");
        };
        assert!(!post_resize_output_seen);
        assert_eq!(last_resize_seq, 0);
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_read_times_out_when_peer_sends_no_request() {
        let (server, _client) = std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        configure_unix_daemon_client_read_timeout(&server, std::time::Duration::from_millis(50))
            .expect("set read timeout");

        let started = std::time::Instant::now();
        let error = read_request(server).expect_err("silent client should time out");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "silent local clients must not block the daemon accept loop indefinitely"
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("reading daemon request"),
            "timeout should be reported as a daemon request read failure: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_poll_times_out_before_request_read() {
        let (server, _client) = std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        let started = std::time::Instant::now();
        let error =
            wait_for_unix_daemon_client_request(&server, std::time::Duration::from_millis(50))
                .expect_err("silent client should time out before read_request");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "silent local clients must not monopolize the synchronous accept loop"
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("timed out waiting for daemon client request bytes"),
            "poll timeout should name the accept-loop request-byte guard: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_poll_accepts_buffered_request_bytes() {
        let (server, mut client) =
            std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        client
            .write_all(b"{\"kind\":\"ping\"}\n")
            .expect("write request bytes");

        wait_for_unix_daemon_client_request(&server, std::time::Duration::from_millis(250))
            .expect("buffered request bytes should be readable");
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_client_partial_request_times_out_after_poll() {
        let (server, mut client) =
            std::os::unix::net::UnixStream::pair().expect("unix stream pair");
        client
            .write_all(b"{\"kind\":\"ping\"")
            .expect("write partial request bytes");

        let started = std::time::Instant::now();
        let error = read_unix_request_with_timeout(&server, std::time::Duration::from_millis(50))
            .expect_err("partial request should time out instead of wedging read_line");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "partial readable requests must not monopolize the synchronous accept loop"
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("reading daemon request"),
            "partial request timeout should be reported as a daemon request read failure: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_daemon_accept_loop_dispatches_clients_off_accept_thread() {
        let source = include_str!("daemon.rs");
        let unix_loop = source
            .split("if let ServerEndpoint::UnixSocket(path) = endpoint {")
            .nth(1)
            .and_then(|suffix| suffix.split("if let ServerEndpoint::Tcp").next())
            .expect("unix daemon accept loop should be present");

        assert!(
            unix_loop.contains("std::sync::mpsc::channel::<Result<DaemonRequestOutcome>>()"),
            "unix accept loop must collect request outcomes asynchronously"
        );
        assert!(
            unix_loop.contains("drain_unix_client_outcomes("),
            "unix accept loop must drain completed client outcomes without blocking accept"
        );
        assert!(
            unix_loop.contains("spawn_unix_client_handler("),
            "unix accept loop must dispatch each accepted client away from the accept thread"
        );
        assert!(
            !unix_loop.contains("match handle_unix_stream(stream"),
            "unix accept loop must not let one partial client monopolize the daemon"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_hot_restart_releases_bind_lock_before_spawning_replacement() {
        let source = include_str!("daemon.rs");
        let unix_loop = source
            .split("if let ServerEndpoint::UnixSocket(path) = endpoint {")
            .nth(1)
            .and_then(|suffix| suffix.split("if let ServerEndpoint::Tcp").next())
            .expect("unix daemon accept loop should be present");
        let drop_lock = unix_loop
            .find("drop(daemon_socket_lock);")
            .expect("hot restart must explicitly release the socket lock");
        let spawn_restart = unix_loop
            .find("spawn_hot_restart_daemon_process(&executable, endpoint)")
            .expect("hot restart spawn should be present");
        assert!(
            drop_lock < spawn_restart,
            "replacement daemon must spawn only after the old daemon releases the bind lock"
        );
    }

    #[test]
    fn explicit_remove_session_drops_local_runtime_before_remote_shutdown() {
        let source = include_str!("daemon.rs");
        let remove_session_block = source
            .split("ServerRequest::RemoveSession { path } => {")
            .nth(1)
            .and_then(|suffix| suffix.split("ServerRequest::DropTerminalRuntime").next())
            .expect("remove session handler should be present");

        let local_drop = remove_session_block
            .find(".remove_session(&runtime_path, stop_command.as_deref())")
            .expect("explicit close should drop the local terminal runtime");
        let remote_shutdown = remove_session_block
            .find("spawn_explicit_remote_session_shutdown(")
            .expect("explicit close should request remote shutdown in the background");
        assert!(
            local_drop < remote_shutdown,
            "remote cleanup must not block local runtime removal or bulk Close All"
        );
        assert!(
            !remove_session_block.contains("terminate_remote_codex_session(machine, session_id)"),
            "explicit close must not synchronously run remote yggterm termination before local close"
        );
        assert!(
            remove_session_block.contains("if removed_terminal {")
                && remove_session_block.contains("format!(\"closed terminal runtime for {path}\")"),
            "runtime-only closes should report as closed so bulk Close All can count them"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refresh_legacy_socket_aliases_skips_self_alias_without_ping_deadlock() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-self-alias-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp socket dir");
        let current = root.join("server-2-1-214.sock");
        let legacy_alias = root.join("server-2-1-213.sock");
        let _listener =
            std::os::unix::net::UnixListener::bind(&current).expect("bind current socket");
        std::os::unix::fs::symlink(&current, &legacy_alias).expect("create self alias");

        assert!(versioned_socket_alias_points_to_current(
            &legacy_alias,
            &current
        ));
        let started = std::time::Instant::now();
        super::refresh_legacy_server_socket_aliases(&current);

        assert!(
            started.elapsed() < std::time::Duration::from_millis(250),
            "startup alias refresh must not ping a symlink that resolves to the current listener"
        );
        assert_eq!(
            fs::read_link(&legacy_alias).expect("self alias should remain"),
            current
        );
        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn refresh_legacy_socket_aliases_retargets_symlink_aliases_without_pinging_target() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-stale-alias-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp socket dir");
        let current = root.join("server-2-1-215.sock");
        let stale = root.join("dead-target.sock");
        let stale_alias = root.join("server-2-1-213.sock");
        let _current_listener =
            std::os::unix::net::UnixListener::bind(&current).expect("bind current socket");
        std::os::unix::fs::symlink(&stale, &stale_alias).expect("create stale alias");

        assert!(versioned_socket_candidate_is_symlink(&stale_alias));
        let started = std::time::Instant::now();
        super::refresh_legacy_server_socket_aliases(&current);

        assert!(
            started.elapsed() < std::time::Duration::from_millis(250),
            "startup alias refresh must retarget symlink aliases without pinging a stale target"
        );
        assert_eq!(
            fs::read_link(&stale_alias).expect("stale alias should be retargeted"),
            current
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn daemon_startup_defers_linux_cleanup_until_after_listener_bind() {
        let source = include_str!("daemon.rs");
        let run_daemon = source
            .split("pub fn run_daemon(endpoint: &ServerEndpoint, runtime: GhosttyHostSupport) -> Result<()> {")
            .nth(1)
            .expect("run_daemon should be present");
        let before_unix_bind = run_daemon
            .split("let listener = std::os::unix::net::UnixListener::bind(path)")
            .next()
            .expect("unix listener bind should be present");

        assert!(
            !before_unix_bind.contains("cleanup_legacy_linux_daemon_processes("),
            "daemon startup must not probe legacy daemon status before binding the current socket"
        );
        assert!(
            run_daemon.contains("spawn_legacy_linux_daemon_cleanup(endpoint.clone()"),
            "legacy daemon cleanup should run only after the daemon has become reachable"
        );

        let headless = include_str!("../../../apps/yggterm/src/bin/yggterm-headless.rs");
        let daemon_entrypoint = headless
            .split("if args.as_slice() == [\"server\", \"daemon\"] {")
            .nth(1)
            .and_then(|suffix| suffix.split("if args.len() >= 3").next())
            .expect("headless server daemon branch should be present");
        assert!(
            !daemon_entrypoint.contains("cleanup_legacy_daemons("),
            "headless daemon entrypoint must not run blocking legacy cleanup before run_daemon"
        );
    }

    #[test]
    fn runtime_load_defers_preserved_owner_deep_reconcile_until_after_socket_bind() {
        let source = include_str!("daemon.rs");
        let load_body = source
            .split("fn load(support: GhosttyHostSupport) -> Result<Self> {")
            .nth(1)
            .and_then(|body| body.split("fn preserved_terminal_owner_keys").next())
            .expect("DaemonRuntime::load body should be present");
        assert!(
            !load_body.contains("restore_missing_preserved_owner_live_sessions("),
            "runtime load must not inspect preserved-owner snapshots before binding the current socket"
        );
        assert!(
            !load_body
                .contains("recover_missing_preserved_owner_live_sessions_from_reachable_daemons("),
            "runtime load must not scan old daemons before binding the current socket"
        );
        assert!(
            !load_body.contains("prune_unrepresented_preserved_owners("),
            "runtime load must not mutate stale owners before binding the current socket"
        );
        assert!(
            !load_body.contains("reachable_versioned_daemon_statuses_excluding_endpoint("),
            "runtime load must not inspect old daemon endpoints before binding the current socket"
        );
        assert!(
            !load_body.contains("retarget_current_alias_entries("),
            "runtime load must not retarget current-alias preserved owners before binding the current socket"
        );
        assert!(
            load_body.contains("preserved_owner_deep_reconcile_deferred_on_load"),
            "runtime load should leave trace evidence for deferred deep reconcile"
        );
    }

    #[test]
    fn force_remote_restart_schedules_daemon_cleanup_after_owner_takeover() {
        let source = include_str!("daemon.rs");
        let branch = source
            .split("ServerRequest::TerminalRestart {\n                path,")
            .nth(1)
            .and_then(|suffix| suffix.split("ServerRequest::SyncExternalWindow").next())
            .expect("terminal restart branch should be present");

        // After the restart_session_with_size refactor, the TerminalRestart
        // branch uses restart_session_with_size(...) not restart_session(...).
        // Accept either to keep the original intent (the handler must call
        // ONE of the restart helpers before cleanup).
        assert!(
            branch.contains("self.terminals.restart_session(")
                || branch.contains("self.terminals.restart_session_with_size("),
            "terminal restart should still restart the requested runtime before cleanup"
        );
        assert!(
            branch.contains("spawn_force_remote_restart_daemon_cleanup("),
            "force-remote restart must rerun daemon cleanup after taking preserved sessions"
        );
        assert!(
            branch.find("self.terminals.restart_session(")
                < branch.find("spawn_force_remote_restart_daemon_cleanup("),
            "cleanup must run after the current daemon has replaced the runtime owner"
        );
    }

    #[test]
    fn update_session_copy_does_not_block_daemon_on_remote_copy_persist() {
        let source = include_str!("daemon.rs");
        let handler = source
            .split("ServerRequest::UpdateSessionCopy {\n                path,")
            .nth(1)
            .and_then(|suffix| suffix.split("ServerRequest::RemoveSshTarget").next())
            .expect("UpdateSessionCopy handler should be present");

        assert!(source.contains("fn spawn_remote_generated_copy_persist("));
        assert!(
            handler.contains("spawn_remote_generated_copy_persist("),
            "remote generated-copy persistence must be spawned away from the daemon request path"
        );
        assert!(
            !handler.contains("persist_remote_generated_copy("),
            "UpdateSessionCopy must not perform remote SSH copy persistence synchronously"
        );
    }

    fn daemon_test_tree() -> yggterm_core::SessionNode {
        yggterm_core::SessionNode {
            kind: yggterm_core::SessionNodeKind::Group,
            name: "sessions".to_string(),
            title: None,
            document_kind: None,
            group_kind: None,
            path: PathBuf::from("/"),
            children: Vec::new(),
            session_id: None,
            cwd: None,
            ..Default::default()
        }
    }

    fn daemon_test_snapshot_session(path: &str, source: SessionSource) -> SnapshotSessionView {
        SnapshotSessionView {
            id: path.rsplit('/').next().unwrap_or(path).to_string(),
            session_path: path.to_string(),
            title: path.rsplit('/').next().unwrap_or(path).to_string(),
            kind: SessionKind::Codex,
            host_label: "dev".to_string(),
            source,
            backend: TerminalBackend::Xterm,
            bridge_available: true,
            launch_phase: TerminalLaunchPhase::Running,
            remote_deploy_state: RemoteDeployState::Ready,
            launch_command: String::new(),
            status_line: String::new(),
            terminal_lines: Vec::new(),
            rendered_sections: Vec::new(),
            preview: SnapshotPreview {
                summary: Vec::new(),
                blocks: Vec::new(),
            },
            metadata: Vec::new(),
            terminal_process_id: None,
            terminal_foreground_active: None,
            terminal_window_id: None,
            terminal_host_token: None,
            terminal_host_mode: crate::GhosttyTerminalHostMode::Unsupported,
            embedded_surface_id: None,
            embedded_surface_detail: None,
            last_launch_error: None,
            last_window_error: None,
            ssh_target: Some("dev".to_string()),
            ssh_prefix: None,
            pty_cols: None,
            pty_rows: None,
            working: None,
        }
    }

    #[test]
    fn preserved_owner_snapshot_restores_missing_live_session_row() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let session_path = remote_scanned_session_path("practice", "241c3f6a");
        let mut owner_session = daemon_test_snapshot_session(&session_path, SessionSource::LiveSsh);
        owner_session.title = "samplers".to_string();
        owner_session.host_label = "practice".to_string();
        owner_session.ssh_target = Some("practice".to_string());
        owner_session.metadata = vec![
            SnapshotMetadataEntry {
                label: "Cwd".to_string(),
                value: "/home/pi/git/samplers".to_string(),
            },
            SnapshotMetadataEntry {
                label: "Runtime Persistence".to_string(),
                value: "keep-alive".to_string(),
            },
            SnapshotMetadataEntry {
                label: "Storage".to_string(),
                value: "/home/pi/.codex/sessions/practice.jsonl".to_string(),
            },
        ];
        let owner_snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(session_path.clone()),
            active_session: Some(owner_session.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: Vec::new(),
            ssh_targets: Vec::new(),
            live_sessions: vec![owner_session],
        };
        let runtime_keys = HashSet::from([session_path.clone()]);

        let restored = super::restore_preserved_owner_live_sessions_from_snapshot(
            &mut server,
            &owner_snapshot,
            &runtime_keys,
        );
        let focused = server.focus_live_session_without_launch_if_active_missing(&session_path);

        assert_eq!(restored, vec![session_path.clone()]);
        assert!(focused);
        assert!(server.represents_terminal_runtime_key(&session_path));
        assert!(server.live_session_keep_alive(&session_path));
        assert_eq!(server.active_session_path(), Some(session_path.as_str()));
        assert_eq!(server.active_view_mode(), WorkspaceViewMode::Terminal);
    }

    #[test]
    fn preserved_owner_recovery_only_restores_kept_or_update_restart_snapshot_rows() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let kept_path = remote_scanned_session_path("practice", "kept");
        let update_path = remote_scanned_session_path("practice", "update");
        let ghost_path = remote_scanned_session_path("practice", "ghost");
        let mut kept = daemon_test_snapshot_session(&kept_path, SessionSource::LiveSsh);
        kept.ssh_target = Some("practice".to_string());
        kept.metadata = vec![SnapshotMetadataEntry {
            label: "Runtime Persistence".to_string(),
            value: "keep-alive".to_string(),
        }];
        let mut update = daemon_test_snapshot_session(&update_path, SessionSource::LiveSsh);
        update.ssh_target = Some("practice".to_string());
        update.metadata = vec![SnapshotMetadataEntry {
            label: "Runtime Restore Reason".to_string(),
            value: crate::UPDATE_RESTART_RESTORE_REASON.to_string(),
        }];
        let mut ghost = daemon_test_snapshot_session(&ghost_path, SessionSource::LiveSsh);
        ghost.ssh_target = Some("practice".to_string());
        let owner_snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: None,
            active_session: None,
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: Vec::new(),
            ssh_targets: Vec::new(),
            live_sessions: vec![kept, update, ghost],
        };
        let runtime_keys =
            HashSet::from([kept_path.clone(), update_path.clone(), ghost_path.clone()]);

        let restored = super::restore_preserved_owner_live_sessions_from_snapshot_with_policy(
            &mut server,
            &owner_snapshot,
            &runtime_keys,
            true,
        );

        assert_eq!(restored, vec![kept_path.clone(), update_path.clone()]);
        assert!(server.represents_terminal_runtime_key(&kept_path));
        assert!(server.represents_terminal_runtime_key(&update_path));
        assert!(
            !server.represents_terminal_runtime_key(&ghost_path),
            "a registry-loss recovery scan must not resurrect unkept closed ghosts"
        );
    }

    #[test]
    fn terminal_read_preserved_owner_path_does_not_run_saved_session_mismatch_probe() {
        let source = include_str!("daemon.rs");
        let terminal_read_branch = source
            .split("ServerRequest::TerminalRead { path, cursor } => {")
            .nth(1)
            .expect("terminal read branch")
            .split("ServerRequest::TerminalSnapshot { path } => {")
            .next()
            .expect("terminal snapshot branch follows terminal read");

        assert!(
            terminal_read_branch.contains("terminal_read(&owner_endpoint, &runtime_path, cursor)"),
            "preserved-owner terminal read must proxy bytes directly"
        );
        assert!(
            !terminal_read_branch.contains("reject_preserved_owner_saved_session_mismatch"),
            "terminal read is the hot loop; mismatch probes belong to ensure/snapshot paths"
        );
    }

    #[test]
    fn preserved_owner_terminal_not_found_error_is_missing_runtime() {
        let error = anyhow::anyhow!(
            "reading daemon response\n\nCaused by:\n    terminal session not found: codex-runtime://019e2ade"
        );

        assert!(
            super::DaemonRuntime::preserved_owner_error_means_missing_runtime(
                "codex-runtime://019e2ade",
                &error
            )
        );
        assert!(
            !super::DaemonRuntime::preserved_owner_error_means_missing_runtime(
                "codex-runtime://other",
                &error
            )
        );
    }

    #[test]
    fn terminal_read_preserved_owner_missing_runtime_falls_back_to_local_ensure() {
        let source = include_str!("daemon.rs");
        let terminal_read_branch = source
            .split("ServerRequest::TerminalRead { path, cursor } => {")
            .nth(1)
            .expect("terminal read branch")
            .split("ServerRequest::TerminalSnapshot { path } => {")
            .next()
            .expect("terminal snapshot branch follows terminal read");
        let owner_error_ix = terminal_read_branch
            .find("self.handle_preserved_owner_request_error(")
            .expect("terminal read handles preserved owner errors");
        let fallback_ix = terminal_read_branch
            .find("let _ = self.ensure_terminal_for_path(&path)?;")
            .expect("terminal read should recover locally after a stale owner is removed");

        assert!(
            owner_error_ix < fallback_ix,
            "preserved-owner read errors must be classified before local fallback"
        );
    }

    #[test]
    fn daemon_background_copy_chore_is_explicit_opt_in() {
        assert!(!daemon_background_copy_chore_enabled_from_env(None));
        assert!(!daemon_background_copy_chore_enabled_from_env(Some("")));
        assert!(!daemon_background_copy_chore_enabled_from_env(Some(
            "false"
        )));
        assert!(daemon_background_copy_chore_enabled_from_env(Some("1")));
        assert!(daemon_background_copy_chore_enabled_from_env(Some("true")));
        assert!(daemon_background_copy_chore_enabled_from_env(Some(" YES ")));
    }

    #[test]
    fn remote_machine_refresh_queue_coalesces_in_flight_targets() {
        let mut in_flight = HashSet::new();
        assert_eq!(
            mark_remote_machine_refresh_queued(&mut in_flight, "dev", true),
            RemoteMachineRefreshQueueStatus::Spawn
        );
        assert_eq!(
            mark_remote_machine_refresh_queued(&mut in_flight, "dev", true),
            RemoteMachineRefreshQueueStatus::AlreadyInFlight
        );
        assert_eq!(
            mark_remote_machine_refresh_queued(&mut in_flight, "local", false),
            RemoteMachineRefreshQueueStatus::Loopback
        );
    }

    #[test]
    fn daemon_snapshot_keeps_remote_agent_rows_across_runtime_gap() {
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let live_sessions = (0..5)
            .map(|ix| {
                daemon_test_snapshot_session(
                    &remote_scanned_session_path("dev", &format!("runtime-{ix}")),
                    SessionSource::LiveSsh,
                )
            })
            .collect::<Vec<_>>();
        let runtime_keys = live_sessions
            .iter()
            .take(4)
            .map(|session| session.session_path.clone())
            .collect::<HashSet<_>>();
        let mut snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(live_sessions[0].session_path.clone()),
            active_session: Some(live_sessions[0].clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: vec![RemoteMachineSnapshot {
                machine_key: "dev".to_string(),
                label: "dev".to_string(),
                ssh_target: "dev".to_string(),
                prefix: None,
                remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: (0..5)
                    .map(|ix| RemoteScannedSession {
                        session_path: remote_scanned_session_path("dev", &format!("runtime-{ix}")),
                        session_id: format!("runtime-{ix}"),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-05-04T00:00:00Z".to_string(),
                        modified_epoch: ix,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 0,
                        title_hint: format!("candidate {ix}"),
                        recent_context: String::new(),
                        cached_precis: None,
                        cached_summary: None,
                        live_runtime: true,
                        storage_path: format!("/home/pi/.codex/sessions/runtime-{ix}.jsonl"),
                    })
                    .collect(),
            }],
            ssh_targets: Vec::new(),
            live_sessions,
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &runtime_keys, &mut snapshot);

        // FIRST-CLASS SESSIONS (jojo 2026-07-08): remote agent rows re-derive
        // from the remote CLI's JSONL, so a row whose runtime is not currently
        // connected is a RECOVERY TARGET, not a husk. All 5 survive the filter;
        // the one without a live runtime (runtime-4) is downgraded to the
        // reconnecting phase rather than dropped from Live Sessions.
        assert_eq!(snapshot.live_sessions.len(), 5);
        let orphan_path = remote_scanned_session_path("dev", "runtime-4");
        let orphan = snapshot
            .live_sessions
            .iter()
            .find(|session| session.session_path == orphan_path)
            .expect("remote agent row without a live runtime is kept as a recovery target");
        assert_eq!(orphan.launch_phase, TerminalLaunchPhase::RemoteBootstrap);
        for session in snapshot
            .live_sessions
            .iter()
            .filter(|session| runtime_keys.contains(&session.session_path))
        {
            assert_eq!(session.launch_phase, TerminalLaunchPhase::Running);
        }
    }

    // Run #16 gate-#5 family: a LOCAL agent CLI row re-derives from the CLI's
    // own JSONL store, so a runtime exit must keep the row (flipped to a
    // recoverable launch phase) instead of erasing it. A non-keep-alive plain
    // shell still dies with its PTY (second-class per the keep-alive spec).
    #[test]
    fn local_agent_rows_survive_runtime_exit_but_plain_shells_do_not() {
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let codex = daemon_test_snapshot_session(
            "local://019df7b2-550e-7090-956b-d4549d4b55e4",
            SessionSource::LiveLocal,
        );
        let cc = {
            let mut session = daemon_test_snapshot_session(
                "local://019e9c28-e46d-7480-9f27-4e5676df61b1",
                // Restored/recovery-created rows carry source=Stored while
                // holding a live local runtime key (2.8.79 lesson; recaught
                // at the 90→91 swap) — the local key must be enough.
                SessionSource::Stored,
            );
            session.kind = SessionKind::ClaudeCode;
            session
        };
        let shell = {
            let mut session =
                daemon_test_snapshot_session("live::plain-shell", SessionSource::LiveLocal);
            session.kind = SessionKind::Shell;
            session
        };
        let mut snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(codex.session_path.clone()),
            active_session: Some(codex.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: Vec::new(),
            ssh_targets: Vec::new(),
            live_sessions: vec![codex.clone(), cc.clone(), shell],
        };

        // Every runtime exited (e.g. daemon swap killed the PTYs).
        apply_terminal_runtime_truth_to_snapshot(&server, &HashSet::new(), &mut snapshot);

        let surviving: Vec<&str> = snapshot
            .live_sessions
            .iter()
            .map(|session| session.session_path.as_str())
            .collect();
        assert_eq!(
            surviving,
            vec![codex.session_path.as_str(), cc.session_path.as_str()],
            "agent rows survive runtime exit; plain shell does not"
        );
        assert!(
            snapshot.live_sessions.iter().all(|session| {
                session.launch_phase == TerminalLaunchPhase::RemoteBootstrap
            }),
            "surviving agent rows flip to a recoverable launch phase"
        );
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.launch_phase),
            Some(TerminalLaunchPhase::RemoteBootstrap),
        );
    }

    #[test]
    fn daemon_snapshot_promotes_stale_pending_phase_when_runtime_owned() {
        // Defect class (jojo 2026-07-09): a remote-cc session's stored
        // launch_phase stuck at RemoteBootstrap while the daemon owned its PTY.
        // The GUI's post-snapshot rearm read the stale phase as "pending
        // launch" and cold-remounted the healthy ACTIVE session after every
        // background snapshot — the sudden-blank-viewport loop. Runtime truth
        // must win in the promote direction too.
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = "remote-cc://dev/stuck-bootstrap".to_string();
        let mut active = daemon_test_snapshot_session(&active_path, SessionSource::LiveSsh);
        active.kind = SessionKind::ClaudeCode;
        active.launch_phase = TerminalLaunchPhase::RemoteBootstrap;
        let mut snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(active_path.clone()),
            active_session: Some(active.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: Vec::new(),
            ssh_targets: Vec::new(),
            live_sessions: vec![active],
        };
        let runtime_keys: HashSet<String> = [active_path.clone()].into_iter().collect();

        apply_terminal_runtime_truth_to_snapshot(&server, &runtime_keys, &mut snapshot);

        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.launch_phase),
            Some(TerminalLaunchPhase::Running),
            "owned runtime promotes the active session's stale pending phase"
        );
        assert_eq!(
            snapshot.live_sessions[0].launch_phase,
            TerminalLaunchPhase::Running,
            "owned runtime promotes the live row's stale pending phase"
        );
    }

    #[test]
    fn daemon_snapshot_preserves_restored_remote_agent_without_runtime() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = remote_scanned_session_path("dev", "missing-runtime");
        let remote_machine = RemoteMachineSnapshot {
            machine_key: "dev".to_string(),
            label: "dev".to_string(),
            ssh_target: "dev".to_string(),
            prefix: None,
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: vec![RemoteScannedSession {
                session_path: active_path.clone(),
                session_id: "missing-runtime".to_string(),
                cwd: "/home/pi".to_string(),
                started_at: "2026-05-04T00:00:00Z".to_string(),
                modified_epoch: 1,
                event_count: 1,
                user_message_count: 1,
                assistant_message_count: 0,
                title_hint: "Recovered preview".to_string(),
                recent_context: "USER: htop".to_string(),
                cached_precis: Some("preview only".to_string()),
                cached_summary: None,
                live_runtime: true,
                storage_path: "/home/pi/.codex/sessions/missing-runtime.jsonl".to_string(),
            }],
        };
        server.apply_snapshot(ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: None,
            active_session: None,
            active_view_mode: WorkspaceViewMode::Rendered,
            remote_machines: vec![remote_machine.clone()],
            ssh_targets: Vec::new(),
            live_sessions: Vec::new(),
        });
        let active = daemon_test_snapshot_session(&active_path, SessionSource::LiveSsh);
        let mut snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(active_path.clone()),
            active_session: Some(active.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: vec![remote_machine],
            ssh_targets: Vec::new(),
            live_sessions: vec![active],
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &HashSet::new(), &mut snapshot);

        // FIRST-CLASS SESSIONS (jojo 2026-07-08): a restored remote agent row
        // without a live runtime re-derives from the remote CLI's JSONL, so it is
        // a RECOVERY TARGET — preserved in Live Sessions as a reconnecting row,
        // not downgraded to a static preview. This is what keeps the user's dev
        // sessions in Live Sessions across a reconnect gap after an agentic
        // restart. Mirrors the keep-alive remote path.
        assert_eq!(snapshot.live_sessions.len(), 1);
        assert_eq!(snapshot.active_view_mode, WorkspaceViewMode::Terminal);
        assert_eq!(
            snapshot.active_session_path.as_deref(),
            Some(active_path.as_str())
        );
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.source),
            Some(SessionSource::LiveSsh)
        );
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.launch_phase),
            Some(TerminalLaunchPhase::RemoteBootstrap)
        );
        assert_eq!(
            snapshot.live_sessions[0].launch_phase,
            TerminalLaunchPhase::RemoteBootstrap
        );
    }

    #[test]
    fn daemon_snapshot_preserves_keep_alive_remote_live_without_runtime_for_restore() {
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = remote_scanned_session_path("dev", "kept-runtime");
        let mut active = daemon_test_snapshot_session(&active_path, SessionSource::LiveSsh);
        active.metadata.push(crate::SnapshotMetadataEntry {
            label: "Runtime Persistence".to_string(),
            value: "keep-alive".to_string(),
        });
        let mut snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(active_path.clone()),
            active_session: Some(active.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: vec![RemoteMachineSnapshot {
                machine_key: "dev".to_string(),
                label: "dev".to_string(),
                ssh_target: "dev".to_string(),
                prefix: None,
                remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: Vec::new(),
            }],
            ssh_targets: Vec::new(),
            live_sessions: vec![active],
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &HashSet::new(), &mut snapshot);

        assert_eq!(
            snapshot.active_session_path.as_deref(),
            Some(active_path.as_str())
        );
        assert_eq!(snapshot.active_view_mode, WorkspaceViewMode::Terminal);
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.source),
            Some(SessionSource::LiveSsh)
        );
        assert_eq!(snapshot.live_sessions.len(), 1);
        assert_eq!(snapshot.live_sessions[0].session_path, active_path);
        assert_eq!(
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.launch_phase),
            Some(TerminalLaunchPhase::RemoteBootstrap)
        );
        assert_eq!(
            snapshot.live_sessions[0].launch_phase,
            TerminalLaunchPhase::RemoteBootstrap
        );
    }

    #[test]
    fn daemon_snapshot_keeps_active_pending_launch_before_runtime_exists() {
        let tree = daemon_test_tree();
        let server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let active_path = "ssh://dev/new-terminal";
        let mut active = daemon_test_snapshot_session(active_path, SessionSource::LiveSsh);
        active.kind = SessionKind::SshShell;
        active.launch_phase = TerminalLaunchPhase::RemoteBootstrap;
        active.remote_deploy_state = RemoteDeployState::Planned;
        // A plain shell with no live PTY is a husk (NOT a recovery target), so it
        // is still dropped by the runtime-truth filter — this is what keeps the
        // test discriminating now that remote agent rows are retained across a
        // runtime gap (see daemon_snapshot_keeps_remote_agent_rows_across_runtime_gap).
        let stale_path = "live::old-shell";
        let mut stale = daemon_test_snapshot_session(stale_path, SessionSource::LiveLocal);
        stale.kind = SessionKind::Shell;
        stale.launch_phase = TerminalLaunchPhase::Running;
        let mut snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(active_path.to_string()),
            active_session: Some(active.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: Vec::new(),
            ssh_targets: Vec::new(),
            live_sessions: vec![active, stale],
        };

        apply_terminal_runtime_truth_to_snapshot(&server, &HashSet::new(), &mut snapshot);

        assert_eq!(snapshot.active_session_path.as_deref(), Some(active_path));
        assert_eq!(snapshot.active_view_mode, WorkspaceViewMode::Terminal);
        assert_eq!(snapshot.live_sessions.len(), 1);
        assert_eq!(snapshot.live_sessions[0].session_path.as_str(), active_path);
    }

    #[test]
    fn live_local_agent_titles_refresh_when_resolver_has_no_real_title() {
        // User bug 5: a yggterm-spawned codex opens with a cwd-derived
        // launch-hint title ("home/pi codex") that the fallback recognizer
        // can't enumerate — under the historical double gate it never
        // refreshed. For live local agents the resolver DB alone gates.
        assert!(super::background_copy_title_missing(
            true,
            "home/pi codex",
            None
        ));
        assert!(super::background_copy_title_missing(
            true,
            "gh/yggterm codex",
            Some("local::0a1b2c3d-0000-4000-8000-000000000000")
        ));
        // A real stored title (e.g. a manual rename) is never clobbered.
        assert!(!super::background_copy_title_missing(
            true,
            "home/pi codex",
            Some("Fix the flaky scheduler test")
        ));
        // Non-live candidates keep the historical double gate: a
        // substantive candidate title blocks regeneration.
        assert!(!super::background_copy_title_missing(
            false,
            "Refactor the parser pipeline",
            None
        ));
        assert!(super::background_copy_title_missing(
            false,
            "yggterm codex",
            None
        ));
    }

    #[test]
    fn live_cc_title_sync_skips_sessions_without_jsonl_and_matching_titles() {
        // collect_live_cc_title_syncs must fail open: no CC JSONL on disk →
        // no update (and never touch non-CC or remote sessions).
        let server = {
            let tree = yggterm_core::SessionNode {
                kind: yggterm_core::SessionNodeKind::Group,
                name: "sessions".to_string(),
                path: std::path::PathBuf::from("/"),
                ..Default::default()
            };
            let mut server = crate::YggtermServer::new(
                &tree,
                false,
                crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
                yggui_contract::UiTheme::ZedLight,
            );
            // A CC session whose id has no ~/.claude JSONL on disk.
            server.start_local_session(
                crate::SessionKind::ClaudeCode,
                Some("/home/pi"),
                Some("home/pi claude"),
            );
            server
        };
        // Even while "working", a CC session with no JSONL on disk yields no
        // update (and never touch non-CC or remote sessions).
        let working: std::collections::HashSet<String> = server
            .live_sessions()
            .iter()
            .map(|s| s.session_path.clone())
            .collect();
        let updates = super::collect_live_cc_title_syncs(&server.live_sessions(), &working);
        assert!(updates.is_empty());
        // And a NON-working CC session is never polled at all (the working
        // indicator is the trigger — spec-title-summary-working-indicator).
        let idle = std::collections::HashSet::new();
        let updates = super::collect_live_cc_title_syncs(&server.live_sessions(), &idle);
        assert!(updates.is_empty());
    }

    #[test]
    fn remote_cc_title_poll_selects_working_and_unconfirmed_rows() {
        // Working remote-cc rows are always polled (renames/ai-titles land on
        // working turns); idle rows are polled only until their title has been
        // confirmed once against the remote JSONL (heals stale launch hints
        // after a restore). Non-CC and non-remote-cc rows are never selected.
        let template = {
            let tree = yggterm_core::SessionNode {
                kind: yggterm_core::SessionNodeKind::Group,
                name: "sessions".to_string(),
                path: std::path::PathBuf::from("/"),
                ..Default::default()
            };
            let mut server = crate::YggtermServer::new(
                &tree,
                false,
                crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
                yggui_contract::UiTheme::ZedLight,
            );
            server.start_local_session(
                crate::SessionKind::ClaudeCode,
                Some("/home/pi"),
                Some("home/pi claude"),
            );
            server.live_sessions()[0].clone()
        };
        let make = |path: &str, id: &str, kind: crate::SessionKind| {
            let mut session = template.clone();
            session.session_path = path.to_string();
            session.id = id.to_string();
            session.kind = kind;
            session
        };
        let sessions = vec![
            make(
                "remote-cc://practice/aaaa",
                "aaaa",
                crate::SessionKind::ClaudeCode,
            ),
            make(
                "remote-cc://practice/bbbb",
                "bbbb",
                crate::SessionKind::ClaudeCode,
            ),
            make("local://cccc", "cccc", crate::SessionKind::ClaudeCode),
            make(
                "remote-session://practice/dddd",
                "dddd",
                crate::SessionKind::Codex,
            ),
        ];
        let working: std::collections::HashSet<String> =
            ["remote-cc://practice/aaaa".to_string()].into();
        let mut confirmed = std::collections::HashSet::new();
        // First tick: both remote-cc rows selected (aaaa working, bbbb unconfirmed).
        let picked = super::remote_cc_title_poll_paths(&sessions, &working, &confirmed);
        let paths: Vec<&str> = picked.iter().map(|(_, _, p)| p.as_str()).collect();
        assert_eq!(
            paths,
            vec!["remote-cc://practice/aaaa", "remote-cc://practice/bbbb"]
        );
        assert_eq!(picked[0].0, "practice");
        assert_eq!(picked[0].1, "aaaa");
        // Once confirmed, an idle row is no longer polled; a working row still is.
        confirmed.insert("remote-cc://practice/aaaa".to_string());
        confirmed.insert("remote-cc://practice/bbbb".to_string());
        let picked = super::remote_cc_title_poll_paths(&sessions, &working, &confirmed);
        let paths: Vec<&str> = picked.iter().map(|(_, _, p)| p.as_str()).collect();
        assert_eq!(paths, vec!["remote-cc://practice/aaaa"]);
    }

    #[test]
    fn live_cc_title_sync_covers_cc_runtime_rows() {
        // cc-runtime:// rows (the host-daemon runtime lane) live on the same
        // machine as ~/.claude/projects, so they ride the same local JSONL
        // sync as local:// rows. With no JSONL on disk this yields no update,
        // but the row must be CONSIDERED (regression lock for the local://
        // -only filter that left host-daemon CC rows stuck on launch hints).
        let session = {
            let tree = yggterm_core::SessionNode {
                kind: yggterm_core::SessionNodeKind::Group,
                name: "sessions".to_string(),
                path: std::path::PathBuf::from("/"),
                ..Default::default()
            };
            let mut server = crate::YggtermServer::new(
                &tree,
                false,
                crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
                yggui_contract::UiTheme::ZedLight,
            );
            server.start_local_session(
                crate::SessionKind::ClaudeCode,
                Some("/home/pi"),
                Some("home/pi claude"),
            );
            let mut session = server.live_sessions()[0].clone();
            session.session_path =
                "cc-runtime://00000000-0000-0000-0000-000000000000".to_string();
            session.id = "00000000-0000-0000-0000-000000000000".to_string();
            session
        };
        let working: std::collections::HashSet<String> =
            [session.session_path.clone()].into();
        // No JSONL exists for the nil UUID → fail open with no update (and no panic).
        let updates = super::collect_live_cc_title_syncs(&[session], &working);
        assert!(updates.is_empty());
    }

    #[test]
    fn live_title_candidates_gate_on_working_indicator() {
        // spec-title-summary-working-indicator: a live agent session becomes
        // a title candidate ONLY while working; idle sessions are never
        // scanned or sent to the LLM.
        let tree = yggterm_core::SessionNode {
            kind: yggterm_core::SessionNodeKind::Group,
            name: "sessions".to_string(),
            path: std::path::PathBuf::from("/"),
            ..Default::default()
        };
        let mut server = crate::YggtermServer::new(
            &tree,
            false,
            crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
            yggui_contract::UiTheme::ZedLight,
        );
        let key = server.start_local_session(
            crate::SessionKind::Codex,
            Some("/home/pi"),
            Some("home/pi codex"),
        );
        // Give the live session a Storage JSONL so it would qualify as a
        // live-local-agent candidate (write a real temp file).
        let dir = std::env::temp_dir().join("yggterm-test-title-gate");
        let _ = std::fs::create_dir_all(&dir);
        let jsonl = dir.join("rollout-test-title-gate.jsonl");
        let _ = std::fs::write(&jsonl, "{}\n");
        if let Some(session) = server.sessions.get_mut(&key) {
            crate::upsert_session_metadata(
                &mut session.metadata,
                "Storage",
                jsonl.to_string_lossy().to_string(),
            );
        }
        let store = yggterm_core::SessionStore::open_or_init().expect("store");
        let mut out = Vec::new();
        // Idle → no candidate.
        let idle = std::collections::HashSet::new();
        super::collect_live_copy_candidates(&store, &server.live_sessions(), &idle, &mut out);
        assert!(
            out.iter().all(|c| c.session_path != key),
            "idle live session must not become a title candidate"
        );
        // Working → candidate appears.
        let mut out = Vec::new();
        let working: std::collections::HashSet<String> = [key.clone()].into_iter().collect();
        super::collect_live_copy_candidates(&store, &server.live_sessions(), &working, &mut out);
        assert!(
            out.iter().any(|c| c.session_path == key && c.live_local_agent),
            "working live agent session must become a title candidate"
        );
        let _ = std::fs::remove_file(&jsonl);
    }

    #[test]
    fn client_close_preserves_sessions_while_an_update_is_staged() {
        // Gate #5: kill -TERM on the GUI during a deploy looked like a user
        // close and removed every non-keep-alive local row through the
        // spec'd client-close path. A staged version difference (or the
        // update-state latch) marks the close as update-related.
        assert!(super::client_close_should_preserve_for_update(
            false,
            Some("2.8.88"),
            "2.8.87"
        ));
        assert!(super::client_close_should_preserve_for_update(
            true, None, "2.8.87"
        ));
        // Same version staged + no latch = a genuine user close: spec says
        // non-keep-alive sessions may be removed.
        assert!(!super::client_close_should_preserve_for_update(
            false,
            Some("2.8.87"),
            "2.8.87"
        ));
        assert!(!super::client_close_should_preserve_for_update(
            false, None, "2.8.87"
        ));
    }

    #[test]
    fn preserved_owner_transport_errors_skip_the_status_reprobe() {
        // Post-swap shadow fix (2026-06-11): one terminal_ensure held the
        // request loop 30.7s probing dead prior-daemon sockets (10s timeout
        // each, then a status() re-probe of the same dead socket). Transport-
        // shaped errors must classify so the re-probe is skipped and the
        // negative cache arms.
        for msg in [
            "reading daemon response",
            "connecting to /home/pi/.yggterm/server-2-8-80.sock",
            "terminal ensure timed out after 5000ms",
        ] {
            assert!(
                super::preserved_owner_error_is_transport_shaped(&anyhow::anyhow!("{msg}")),
                "{msg} must classify as transport-shaped"
            );
        }
        assert!(!super::preserved_owner_error_is_transport_shaped(&anyhow::anyhow!(
            "terminal session not found: remote-session://dev/x"
        )));
    }

    #[test]
    fn remote_start_requests_get_longer_daemon_response_budget() {
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::Status),
            super::DAEMON_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::StartRemoteCodexSession {
                target: "dev".to_string(),
                prefix: None,
                cwd: Some("/home/pi/gh/yggterm".to_string()),
                title_hint: Some("Debug".to_string()),
                terminal_appearance: None,
                insert_after: None,
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::StartSshSession {
                target: "dev".to_string(),
                prefix: None,
                cwd: None,
                title_hint: None,
                terminal_appearance: None,
                insert_after: None,
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::TerminalRestart {
                path: "remote-session://dev/session".to_string(),
                terminal_appearance: Some("dark".to_string()),
                force_remote: true,
                initial_cols: Some(110),
                initial_rows: Some(50),
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::RefreshPreview {
                path: "remote-session://dev/session".to_string(),
                full_remote_payload: true,
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::TerminalRestart {
                path: "remote-session://dev/session".to_string(),
                terminal_appearance: None,
                force_remote: false,
                initial_cols: None,
                initial_rows: None,
            }),
            super::DAEMON_REQUEST_IO_TIMEOUT_MS
        );
        assert_eq!(
            super::daemon_request_io_timeout_ms(&super::ServerRequest::RemoveSession {
                path: "local://slow-close".to_string(),
            }),
            super::DAEMON_LONG_REQUEST_IO_TIMEOUT_MS
        );
    }

    #[test]
    fn hot_restart_detects_duplicate_runtime_owner_before_handoff() {
        fn status_for_hot_restart_test(
            version: &str,
            pid: u32,
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            status_for_hot_restart_test_with_owned(version, pid, terminal_keys, terminal_keys)
        }

        fn status_for_hot_restart_test_with_owned(
            version: &str,
            pid: u32,
            owned_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": pid as u64,
                "server_pid": pid,
                "host_kind": "linux-controlled-dock",
                "host_detail": "test",
                "embedded_surface_supported": false,
                "bridge_enabled": false,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
            }))
            .expect("test status")
        }

        let owned_keys = vec![
            "local://one".to_string(),
            "remote-session://dev/two".to_string(),
        ];
        let duplicate_current = status_for_hot_restart_test(
            "2.2.41",
            410,
            &["local://one", "remote-session://dev/two"],
        );
        let partial_current = status_for_hot_restart_test("2.2.41", 411, &["local://one"]);
        let stale_self = status_for_hot_restart_test(
            "2.2.40",
            400,
            &["local://one", "remote-session://dev/two"],
        );

        let duplicate = super::hot_restart_duplicate_runtime_owner_status_from_statuses(
            Some("2.2.41"),
            &owned_keys,
            400,
            vec![partial_current, duplicate_current.clone(), stale_self],
        )
        .expect("duplicate current owner");

        assert_eq!(duplicate.server_pid, duplicate_current.server_pid);
        assert!(
            super::hot_restart_duplicate_runtime_owner_status_from_statuses(
                Some("2.2.41"),
                &owned_keys,
                400,
                vec![status_for_hot_restart_test("2.2.41", 411, &["local://one"])],
            )
            .is_none(),
            "partial current ownership must not retire the stale owner"
        );
        assert!(
            super::hot_restart_duplicate_runtime_owner_status_from_statuses(
                Some("2.2.41"),
                &owned_keys,
                400,
                vec![status_for_hot_restart_test_with_owned(
                    "2.2.41",
                    412,
                    &["local://one"],
                    &["local://one", "remote-session://dev/two"],
                )],
            )
            .is_none(),
            "preserved/runtime-known keys are not enough; the target daemon must directly own the PTYs"
        );
    }

    #[test]
    fn drop_terminal_runtime_is_local_only_and_does_not_retire_daemon() {
        let request = super::ServerRequest::DropTerminalRuntime {
            runtime_key: "remote-session://dev/current-samplenotes".to_string(),
            reason: Some("duplicate_legacy_owned_runtime_prune".to_string()),
        };
        assert_eq!(
            super::server_request_name(&request),
            "drop_terminal_runtime"
        );

        let outcome = super::daemon_request_outcome_for_response(
            &request,
            &super::ServerResponse::Ack {
                message: Some("dropped terminal runtime".to_string()),
            },
        );

        assert!(
            !outcome.should_shutdown,
            "dropping a duplicate PTY must not be confused with daemon shutdown"
        );
        assert_eq!(
            outcome.restart_executable, None,
            "dropping a duplicate PTY must not start another hot-update chain"
        );
    }

    #[test]
    fn remote_runtime_requests_carry_terminal_appearance() {
        let request = super::ServerRequest::EnsureRemoteRuntimeCodexSession {
            session_id: "abc123".to_string(),
            cwd: Some("/srv/app".to_string()),
            require_existing: true,
            terminal_appearance: Some("dark".to_string()),
            initial_cols: Some(120),
            initial_rows: Some(36),
        };
        let value = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(value["terminal_appearance"], "dark");

        let decoded: super::ServerRequest =
            serde_json::from_value(value).expect("deserialize request");
        match decoded {
            super::ServerRequest::EnsureRemoteRuntimeCodexSession {
                terminal_appearance,
                ..
            } => assert_eq!(terminal_appearance.as_deref(), Some("dark")),
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn remote_session_terminal_input_uses_hot_local_runtime_before_remote_fallback() {
        assert_eq!(
            super::terminal_write_strategy_for_path(
                "remote-session://dev/8931728b-30a7-428a-8b8e-35bee0480444",
                true
            ),
            super::TerminalWriteStrategy::LocalRuntime
        );
        assert_eq!(
            super::terminal_write_strategy_for_path(
                "  remote-session://dev/d98bc22f-91e4-4332-8696-122cca33c71c",
                false
            ),
            super::TerminalWriteStrategy::RemoteDirectFallback
        );
        assert_eq!(
            super::terminal_write_strategy_for_path("ssh://dev/home/pi/gh/yggterm", false),
            super::TerminalWriteStrategy::LocalRuntimeFallback
        );
        assert_eq!(
            super::terminal_write_strategy_for_path("local://shell", true),
            super::TerminalWriteStrategy::LocalRuntime
        );
    }

    #[test]
    fn daemon_request_outcome_restarts_only_after_successful_hot_restart() {
        let request = super::ServerRequest::HotRestart {
            daemon_executable: "/tmp/yggterm-headless-next".to_string(),
            expected_version: Some("2.1.61".to_string()),
            expected_build_id: None,
            reason: Some("test".to_string()),
            force: false,
        };

        let outcome = super::daemon_request_outcome_for_response(
            &request,
            &super::ServerResponse::Ack {
                message: Some("ok".to_string()),
            },
        );
        assert!(outcome.should_shutdown);
        assert_eq!(
            outcome.restart_executable,
            Some(PathBuf::from("/tmp/yggterm-headless-next"))
        );

        let error_outcome = super::daemon_request_outcome_for_response(
            &request,
            &super::ServerResponse::Error {
                message: "no".to_string(),
            },
        );
        assert!(!error_outcome.should_shutdown);
        assert_eq!(error_outcome.restart_executable, None);

        let shutdown_outcome = super::daemon_request_outcome_for_response(
            &super::ServerRequest::Shutdown,
            &super::ServerResponse::Ack {
                message: Some("bye".to_string()),
            },
        );
        assert!(shutdown_outcome.should_shutdown);
        assert_eq!(shutdown_outcome.restart_executable, None);

        let retire_outcome = super::daemon_request_outcome_for_response(
            &super::ServerRequest::RetireDaemon {
                reason: Some("stale covered sidecar".to_string()),
            },
            &super::ServerResponse::Ack {
                message: Some("retire".to_string()),
            },
        );
        assert!(retire_outcome.should_shutdown);
        assert_eq!(retire_outcome.restart_executable, None);

        let handoff_outcome = super::daemon_request_outcome_for_response(
            &request,
            &super::ServerResponse::HotUpdateHandoff {
                message: Some("handoff".to_string()),
                owner_endpoint: "/tmp/server-2-1-60.sock".to_string(),
                owner_server_version: "2.1.60".to_string(),
                owner_server_pid: 123,
                target_server_version: Some("2.1.61".to_string()),
                runtime_keys: vec!["local://kept".to_string()],
            },
        );
        assert!(!handoff_outcome.should_shutdown);
        assert_eq!(handoff_outcome.restart_executable, None);
    }

    #[test]
    fn hot_restart_defers_when_daemon_owns_live_terminal_runtime() {
        let owned_keys = vec!["local://kept".to_string()];
        assert!(super::hot_restart_should_defer_for_session_survival(
            &owned_keys
        ));

        let empty_owned_keys = Vec::<String>::new();
        assert!(!super::hot_restart_should_defer_for_session_survival(
            &empty_owned_keys
        ));
    }

    #[test]
    fn hot_restart_does_not_defer_for_preserved_only_terminal_runtime() {
        let preserved_only_owned_keys = Vec::<String>::new();
        let preserved_only_status: ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.1.61",
                "server_build_id": 0,
                "server_pid": 45,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "restored_live_sessions": 5,
                "owned_terminal_session_count": 0,
                "owned_terminal_session_keys": [],
                "terminal_session_count": 2,
                "terminal_session_keys": ["local://kept-1", "local://kept-2"],
                "preserved_terminal_owner_count": 2,
                "preserved_terminal_owner_keys": ["local://kept-1", "local://kept-2"],
                "managed_session_count": 0,
            }))
            .expect("status");

        assert_eq!(preserved_only_status.owned_terminal_session_count, 0);
        assert_eq!(preserved_only_status.preserved_terminal_owner_count, 2);
        assert!(!super::hot_restart_should_defer_for_session_survival(
            &preserved_only_owned_keys
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_retargets_existing_sidecar_entries_for_handoff_keys() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-owners-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-61.sock"));
        let chained_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-60.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.61",
            "server_build_id": 0,
            "server_pid": 61,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://new-owner"],
        }))
        .expect("status");
        let existing = vec![super::PreservedTerminalOwnerEntry {
            runtime_key: "local://old-owner".to_string(),
            endpoint: super::PreservedOwnerEndpoint::from_endpoint(&chained_endpoint),
            owner_server_version: "2.1.60".to_string(),
            owner_server_build_id: 0,
            owner_server_pid: 60,
            created_at_ms: 1,
        }];

        let registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.62".to_string()),
            vec![
                "local://new-owner".to_string(),
                "local://old-owner".to_string(),
            ],
            existing,
        )
        .expect("write registry");

        assert_eq!(
            registry.keys(),
            vec![
                "local://new-owner".to_string(),
                "local://old-owner".to_string()
            ]
        );
        assert_eq!(
            registry
                .owner_for_key("local://old-owner")
                .expect("old owner")
                .owner_server_pid,
            61
        );
        assert_eq!(
            registry
                .owner_for_key("local://new-owner")
                .expect("new owner")
                .owner_server_pid,
            61
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_retargets_preserved_sidecar_entries() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-retarget-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-61.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.61",
            "server_build_id": 0,
            "server_pid": 61,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["local://kept"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://kept"],
        }))
        .expect("status");
        let mut registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.62".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect("write registry");

        assert!(
            registry
                .retarget_expected_server_version(&home, Some("2.1.63".to_string()))
                .expect("retarget")
        );

        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.expected_server_version.as_deref(), Some("2.1.63"));
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_is_not_pruned_by_key_presence_alone() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-current-owner-preserve-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-7-12.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.7.12",
            "server_build_id": 12,
            "server_pid": 2712,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 2,
            "owned_terminal_session_keys": [
                "remote-session://samplenotes-webapp/current",
                "remote-session://samplenotes-webapp/other"
            ],
            "terminal_session_count": 2,
            "terminal_session_keys": [
                "remote-session://samplenotes-webapp/current",
                "remote-session://samplenotes-webapp/other"
            ],
        }))
        .expect("status");
        let registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.7.13".to_string()),
            vec![
                "remote-session://samplenotes-webapp/current".to_string(),
                "remote-session://samplenotes-webapp/other".to_string(),
            ],
            Vec::new(),
        )
        .expect("write registry");

        assert_eq!(
            registry.keys(),
            vec![
                "remote-session://samplenotes-webapp/current".to_string(),
                "remote-session://samplenotes-webapp/other".to_string()
            ],
            "a current daemon with the same key must not erase preserved-owner state until the duplicate owner is explicitly pruned"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_retargets_current_alias_to_reachable_owner() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-current-alias-retarget-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let current_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-7-18.sock"));
        let current_alias_path = home.join("server-2-1-0.sock");
        if let super::ServerEndpoint::UnixSocket(path) = &current_endpoint {
            fs::write(path, b"current").expect("write current endpoint placeholder");
        }
        std::os::unix::fs::symlink(
            match &current_endpoint {
                super::ServerEndpoint::UnixSocket(path) => path,
                super::ServerEndpoint::Tcp { .. } => unreachable!(),
            },
            &current_alias_path,
        )
        .expect("create current alias");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-7-9.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.7.9",
            "server_build_id": 9,
            "server_pid": 279,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["remote-session://dev/kept"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["remote-session://dev/kept"],
        }))
        .expect("owner status");
        let mut registry = super::PreservedTerminalOwnerRegistry {
            schema_version: super::preserved_terminal_owner_schema_version(),
            expected_server_version: Some("2.7.18".to_string()),
            entries: vec![super::PreservedTerminalOwnerEntry {
                runtime_key: "remote-session://dev/kept".to_string(),
                endpoint: super::PreservedOwnerEndpoint::from_endpoint(
                    &super::ServerEndpoint::UnixSocket(current_alias_path),
                ),
                owner_server_version: "2.7.16".to_string(),
                owner_server_build_id: 16,
                owner_server_pid: 2716,
                created_at_ms: 1,
            }],
        };

        let changed = registry
            .retarget_current_alias_entries(
                &home,
                &current_endpoint,
                &[(owner_endpoint.clone(), owner_status)],
                218,
                "test",
            )
            .expect("retarget current alias");

        assert!(changed);
        let entry = registry
            .owner_for_key("remote-session://dev/kept")
            .expect("retargeted entry");
        assert!(super::server_endpoints_same_target(
            &entry.endpoint.to_endpoint(),
            &owner_endpoint
        ));
        assert_eq!(entry.owner_server_version, "2.7.9");
        assert_eq!(entry.owner_server_pid, 279);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_load_retargets_preserved_owner_registry_without_dropping_keys() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-runtime-load-owner-retarget-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-0-0-1.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "0.0.1",
            "server_build_id": 0,
            "server_pid": 1,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["remote-session://dev/kept-work"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["remote-session://dev/kept-work"],
        }))
        .expect("status");
        let mut registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("0.0.1".to_string()),
            vec!["remote-session://dev/kept-work".to_string()],
            Vec::new(),
        )
        .expect("write registry");

        assert!(
            registry
                .retarget_loaded_registry_for_current_version(&home)
                .expect("retarget loaded registry")
        );

        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(
            loaded.expected_server_version.as_deref(),
            Some(super::SERVER_PROTOCOL_VERSION)
        );
        assert_eq!(
            loaded.keys(),
            vec!["remote-session://dev/kept-work".to_string()]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_refuses_older_target_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-target-regression-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-206.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.206",
            "server_build_id": 0,
            "server_pid": 206,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["local://kept"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://kept"],
        }))
        .expect("status");
        super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.208".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect("write newer registry");

        let error = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.207".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect_err("older target overwrite must be refused");

        assert!(error.to_string().contains("target regression"));
        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.expected_server_version.as_deref(), Some("2.1.208"));
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_refuses_older_retarget() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-retarget-regression-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-206.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.206",
            "server_build_id": 0,
            "server_pid": 206,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["local://kept"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["local://kept"],
        }))
        .expect("status");
        let mut registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.208".to_string()),
            vec!["local://kept".to_string()],
            Vec::new(),
        )
        .expect("write newer registry");

        let error = registry
            .retarget_expected_server_version(&home, Some("2.1.207".to_string()))
            .expect_err("older retarget must be refused");

        assert!(error.to_string().contains("target regression"));
        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.expected_server_version.as_deref(), Some("2.1.208"));
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_prunes_unrepresented_runtime_keys() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-update-prune-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let home = root.join(".yggterm");
        fs::create_dir_all(&home).expect("create temp home");
        let owner_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-61.sock"));
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.1.61",
            "server_build_id": 0,
            "server_pid": 61,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 2,
            "owned_terminal_session_keys": ["local://kept", "local://closed-yesterday"],
            "terminal_session_count": 2,
            "terminal_session_keys": ["local://kept", "local://closed-yesterday"],
        }))
        .expect("status");
        let mut registry = super::PreservedTerminalOwnerRegistry::write_handoff(
            &home,
            &owner_endpoint,
            &owner_status,
            Some("2.1.62".to_string()),
            vec![
                "local://kept".to_string(),
                "local://closed-yesterday".to_string(),
            ],
            Vec::new(),
        )
        .expect("write registry");

        let removed = registry.retain_represented_keys(|key| key == "local://kept");
        registry.save(&home).expect("save pruned registry");

        assert_eq!(removed, vec!["local://closed-yesterday".to_string()]);
        let loaded = super::PreservedTerminalOwnerRegistry::load(&home);
        assert_eq!(loaded.keys(), vec!["local://kept".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn handoff_existing_entries_preserves_reachable_prior_owner_runtime_not_in_current_rows() {
        let endpoint = super::ServerEndpoint::UnixSocket(PathBuf::from("/tmp/yggterm-old.sock"));
        let practice = "remote-session://practice/241c3f6a".to_string();
        let closed = "remote-session://dev/closed".to_string();
        let outgoing = "remote-session://dev/outgoing".to_string();
        let entries = vec![
            super::PreservedTerminalOwnerEntry {
                runtime_key: practice.clone(),
                endpoint: super::PreservedOwnerEndpoint::from_endpoint(&endpoint),
                owner_server_version: "2.6.30".to_string(),
                owner_server_build_id: 0,
                owner_server_pid: 2630,
                created_at_ms: 1,
            },
            super::PreservedTerminalOwnerEntry {
                runtime_key: closed.clone(),
                endpoint: super::PreservedOwnerEndpoint::from_endpoint(&endpoint),
                owner_server_version: "2.6.30".to_string(),
                owner_server_build_id: 0,
                owner_server_pid: 2630,
                created_at_ms: 1,
            },
            super::PreservedTerminalOwnerEntry {
                runtime_key: outgoing.clone(),
                endpoint: super::PreservedOwnerEndpoint::from_endpoint(&endpoint),
                owner_server_version: "2.6.30".to_string(),
                owner_server_build_id: 0,
                owner_server_pid: 2630,
                created_at_ms: 1,
            },
        ];
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.6.30",
            "server_build_id": 0,
            "server_pid": 2630,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": [practice],
            "terminal_session_count": 1,
            "terminal_session_keys": [practice],
        }))
        .expect("status");
        let status_cache = BTreeMap::from([(
            super::PreservedOwnerEndpoint::from_endpoint(&endpoint).label(),
            Some(owner_status),
        )]);
        let outgoing_owned_runtime_keys = HashSet::from([outgoing.clone()]);
        let represented_preserved_owner_keys = HashSet::new();

        let kept = super::preserved_owner_entries_live_for_handoff(
            &entries,
            &outgoing_owned_runtime_keys,
            &represented_preserved_owner_keys,
            &status_cache,
        );

        assert_eq!(
            kept.iter()
                .map(|entry| entry.runtime_key.as_str())
                .collect::<Vec<_>>(),
            vec![practice.as_str()],
            "handoff must keep a reachable prior owner runtime and drop closed/outgoing keys"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hot_update_owner_registry_groups_runtime_keys_by_endpoint() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-hot-update-owner-groups-{}-{}",
            std::process::id(),
            super::current_millis_u64()
        ));
        let first_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-163.sock"));
        let second_endpoint = super::ServerEndpoint::UnixSocket(home.join("server-2-1-190.sock"));
        let registry = super::PreservedTerminalOwnerRegistry {
            schema_version: 1,
            expected_server_version: Some("2.1.191".to_string()),
            entries: vec![
                super::PreservedTerminalOwnerEntry {
                    runtime_key: "remote-session://dev/kept-samplenotes".to_string(),
                    endpoint: super::PreservedOwnerEndpoint::from_endpoint(&first_endpoint),
                    owner_server_version: "2.1.163".to_string(),
                    owner_server_build_id: 0,
                    owner_server_pid: 163,
                    created_at_ms: 1,
                },
                super::PreservedTerminalOwnerEntry {
                    runtime_key: "remote-session://dev/kept-erome".to_string(),
                    endpoint: super::PreservedOwnerEndpoint::from_endpoint(&first_endpoint),
                    owner_server_version: "2.1.163".to_string(),
                    owner_server_build_id: 0,
                    owner_server_pid: 163,
                    created_at_ms: 1,
                },
                super::PreservedTerminalOwnerEntry {
                    runtime_key: "local://new-runtime".to_string(),
                    endpoint: super::PreservedOwnerEndpoint::from_endpoint(&second_endpoint),
                    owner_server_version: "2.1.190".to_string(),
                    owner_server_build_id: 0,
                    owner_server_pid: 190,
                    created_at_ms: 1,
                },
            ],
        };

        let groups = registry.endpoint_groups();

        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0],
            (
                first_endpoint,
                vec![
                    "remote-session://dev/kept-erome".to_string(),
                    "remote-session://dev/kept-samplenotes".to_string(),
                ],
            )
        );
        assert_eq!(
            groups[1],
            (second_endpoint, vec!["local://new-runtime".to_string()])
        );
    }

    #[test]
    fn preserved_owner_runtime_prune_candidates_reject_closed_ghost_keys() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let kept_samplenotes = remote_scanned_session_path("dev", "kept-samplenotes");
        server.restore_live_session(PersistedLiveSession {
            key: kept_samplenotes.clone(),
            id: "kept-samplenotes".to_string(),
            title: "samplenotes".to_string(),
            kind: SessionKind::Codex,
            keep_alive: true,
            ssh_target: "dev".to_string(),
            prefix: None,
            cwd: Some("/home/pi/git/samplenotes".to_string()),
            remote_launch_action: None,
            storage_path: None,
            restore_reason: None,
        });
        let unkept_update_runtime = remote_scanned_session_path("dev", "temporary-update");
        server.restore_live_session(PersistedLiveSession {
            key: unkept_update_runtime.clone(),
            id: "temporary-update".to_string(),
            title: "temporary update".to_string(),
            kind: SessionKind::Codex,
            keep_alive: false,
            ssh_target: "dev".to_string(),
            prefix: None,
            cwd: Some("/home/pi/gh/yggterm".to_string()),
            remote_launch_action: None,
            storage_path: None,
            restore_reason: Some("update-restart".to_string()),
        });
        let owner_registry_keys = HashSet::from([kept_samplenotes.clone()]);
        let all_registry_keys = owner_registry_keys.clone();
        let current_owned_runtime_keys = HashSet::new();

        let stale_runtime_keys = super::unrepresented_preserved_owner_runtime_keys(
            &server,
            &owner_registry_keys,
            &all_registry_keys,
            &current_owned_runtime_keys,
            vec![
                kept_samplenotes,
                unkept_update_runtime.clone(),
                "remote-session://dev/closed-may-6".to_string(),
                "remote-session://dev/closed-generic-a".to_string(),
                "remote-session://dev/closed-generic-b".to_string(),
            ],
        );

        assert!(server.live_session_keep_alive(&remote_scanned_session_path("dev", "kept-samplenotes")));
        assert!(!server.live_session_keep_alive(&unkept_update_runtime));
        assert_eq!(
            stale_runtime_keys,
            vec![
                "remote-session://dev/closed-generic-a".to_string(),
                "remote-session://dev/closed-generic-b".to_string(),
                "remote-session://dev/closed-may-6".to_string(),
            ],
            "owner daemons must drop PTYs that are neither in the hot-update registry nor represented by current live-session truth"
        );
    }

    #[test]
    fn preserved_owner_runtime_scan_recovers_plain_running_rows_as_update_restore() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let running_path = remote_scanned_session_path("practice", "running-unkept");
        let mut running = daemon_test_snapshot_session(&running_path, SessionSource::LiveSsh);
        running.ssh_target = Some("practice".to_string());
        running.metadata = vec![SnapshotMetadataEntry {
            label: "Cwd".to_string(),
            value: "/home/pi/git/samplers".to_string(),
        }];
        let owner_snapshot = ServerUiSnapshot {
            apps: Vec::new(),
            active_session_path: Some(running_path.clone()),
            active_session: Some(running.clone()),
            active_view_mode: WorkspaceViewMode::Terminal,
            remote_machines: Vec::new(),
            ssh_targets: Vec::new(),
            live_sessions: vec![running],
        };
        let runtime_keys = HashSet::from([running_path.clone()]);

        let restored = super::restore_preserved_owner_live_sessions_from_snapshot_with_policy(
            &mut server,
            &owner_snapshot,
            &runtime_keys,
            false,
        );

        assert_eq!(restored, vec![running_path.clone()]);
        assert!(server.represents_terminal_runtime_key(&running_path));
        assert!(!server.live_session_keep_alive(&running_path));
        assert!(
            server.live_session_is_temporary_update_restore(&running_path),
            "a running unkept owner row is protected as an update-restore incident, not silently promoted to Keep Alive"
        );
    }

    #[test]
    fn preserved_owner_prune_never_uses_broad_client_close_fallback() {
        let source = include_str!("daemon.rs");
        let prune_block = source
            .split("fn prune_unrepresented_preserved_owner_runtime_sessions(")
            .nth(1)
            .and_then(|suffix| {
                suffix
                    .split("fn prune_duplicate_legacy_owned_runtime_sessions")
                    .next()
            })
            .expect("preserved-owner prune implementation should be present");

        assert!(
            !prune_block.contains("prepare_client_close(&owner_endpoint)"),
            "runtime pruning must target exact terminal keys; broad PrepareClientClose can kill unrelated running sessions on the old owner"
        );
    }

    #[test]
    fn represented_keep_alive_live_session_still_needs_preserved_owner_without_local_runtime() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let kept_runtime = remote_scanned_session_path("practice", "kept-runtime");
        server.restore_live_session(PersistedLiveSession {
            key: kept_runtime.clone(),
            id: "kept-runtime".to_string(),
            title: "samplers non-data".to_string(),
            kind: SessionKind::Codex,
            keep_alive: true,
            ssh_target: "practice".to_string(),
            prefix: None,
            cwd: Some("/home/pi/git/samplers".to_string()),
            remote_launch_action: None,
            storage_path: None,
            restore_reason: None,
        });
        let terminals = TerminalManager::new();

        assert!(server.represents_terminal_runtime_key(&kept_runtime));
        assert!(
            super::represented_live_session_needs_preserved_owner(
                &server,
                &terminals,
                &kept_runtime,
            ),
            "a restored live row is not enough truth; a keep-alive row still needs the old PTY owner registered"
        );
    }

    #[test]
    fn represented_plain_live_session_does_not_recover_unregistered_owner() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let plain_runtime = remote_scanned_session_path("practice", "plain-runtime");
        server.restore_live_session(PersistedLiveSession {
            key: plain_runtime.clone(),
            id: "plain-runtime".to_string(),
            title: "plain runtime".to_string(),
            kind: SessionKind::Codex,
            keep_alive: false,
            ssh_target: "practice".to_string(),
            prefix: None,
            cwd: Some("/home/pi/git/samplers".to_string()),
            remote_launch_action: None,
            storage_path: None,
            restore_reason: None,
        });
        let terminals = TerminalManager::new();

        assert!(
            !super::represented_live_session_needs_preserved_owner(
                &server,
                &terminals,
                &plain_runtime,
            ),
            "plain stale live rows must not resurrect ghost PTY owners"
        );
    }

    #[test]
    fn preserved_owner_runtime_prune_candidates_reject_duplicate_current_owner_keys() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let kept_samplenotes = remote_scanned_session_path("dev", "kept-samplenotes");
        let duplicate_erome = remote_scanned_session_path("dev", "kept-erome");
        for key in [&kept_samplenotes, &duplicate_erome] {
            server.restore_live_session(PersistedLiveSession {
                key: key.clone(),
                id: key.rsplit('/').next().unwrap_or("session").to_string(),
                title: "remote".to_string(),
                kind: SessionKind::Codex,
                keep_alive: true,
                ssh_target: "dev".to_string(),
                prefix: None,
                cwd: Some("/home/pi/git/samplenotes".to_string()),
                remote_launch_action: None,
                storage_path: None,
                restore_reason: None,
            });
        }
        let owner_registry_keys = HashSet::from([kept_samplenotes.clone()]);
        let all_registry_keys = owner_registry_keys.clone();
        let current_owned_runtime_keys = HashSet::from([duplicate_erome.clone()]);

        let stale_runtime_keys = super::unrepresented_preserved_owner_runtime_keys(
            &server,
            &owner_registry_keys,
            &all_registry_keys,
            &current_owned_runtime_keys,
            vec![kept_samplenotes, duplicate_erome.clone()],
        );

        assert_eq!(
            stale_runtime_keys,
            vec![duplicate_erome],
            "an old preserved owner must drop a runtime once the current daemon already owns that same represented live-session key"
        );
    }

    #[test]
    fn preserved_owner_runtime_prune_candidates_reject_keys_assigned_to_other_owner() {
        let tree = daemon_test_tree();
        let mut server = YggtermServer::new(
            &tree,
            false,
            GhosttyHostSupport::shadow("test".to_string(), false, false),
            UiTheme::ZedLight,
        );
        let kept_samplenotes = remote_scanned_session_path("dev", "kept-samplenotes");
        let reassigned_erome = remote_scanned_session_path("dev", "kept-erome");
        for key in [&kept_samplenotes, &reassigned_erome] {
            server.restore_live_session(PersistedLiveSession {
                key: key.clone(),
                id: key.rsplit('/').next().unwrap_or("session").to_string(),
                title: "remote".to_string(),
                kind: SessionKind::Codex,
                keep_alive: true,
                ssh_target: "dev".to_string(),
                prefix: None,
                cwd: Some("/home/pi/git/samplenotes".to_string()),
                remote_launch_action: None,
                storage_path: None,
                restore_reason: None,
            });
        }
        let owner_registry_keys = HashSet::from([kept_samplenotes.clone()]);
        let all_registry_keys = HashSet::from([kept_samplenotes.clone(), reassigned_erome.clone()]);
        let current_owned_runtime_keys = HashSet::new();

        let stale_runtime_keys = super::unrepresented_preserved_owner_runtime_keys(
            &server,
            &owner_registry_keys,
            &all_registry_keys,
            &current_owned_runtime_keys,
            vec![kept_samplenotes, reassigned_erome.clone()],
        );

        assert_eq!(
            stale_runtime_keys,
            vec![reassigned_erome],
            "an older owner must drop a runtime key once the preserved-owner registry assigns that key to a different owner endpoint"
        );
    }

    #[test]
    fn duplicate_legacy_owned_runtime_prune_candidates_keep_unique_old_runtime_keys() {
        let current_runtime_keys = HashSet::from([
            "remote-session://dev/current-erome".to_string(),
            "remote-session://dev/current-samplenotes".to_string(),
        ]);
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.6.1",
            "server_build_id": 1,
            "server_pid": 261,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 2,
            "owned_terminal_session_keys": [
                "remote-session://dev/current-erome",
                "remote-session://dev/old-only-samplescripts"
            ],
            "terminal_session_count": 2,
            "terminal_session_keys": [
                "remote-session://dev/current-erome",
                "remote-session://dev/old-only-samplescripts"
            ],
        }))
        .expect("status");

        let stale_runtime_keys = super::duplicate_legacy_owned_runtime_keys(
            &current_runtime_keys,
            "2.6.2",
            2,
            262,
            &owner_status,
        );

        assert_eq!(
            stale_runtime_keys,
            vec!["remote-session://dev/current-erome".to_string()],
            "an old daemon with mixed ownership must drop only the runtime already directly owned by the newer daemon"
        );
    }

    #[test]
    fn duplicate_legacy_owned_runtime_prune_candidates_keep_newer_owner_keys() {
        let current_runtime_keys =
            HashSet::from(["remote-session://dev/current-erome".to_string()]);
        let owner_status: ServerRuntimeStatus = serde_json::from_value(serde_json::json!({
            "server_version": "2.6.3",
            "server_build_id": 300,
            "server_pid": 300,
            "host_kind": "local",
            "host_detail": "test",
            "embedded_surface_supported": true,
            "bridge_enabled": true,
            "owned_terminal_session_count": 1,
            "owned_terminal_session_keys": ["remote-session://dev/current-erome"],
            "terminal_session_count": 1,
            "terminal_session_keys": ["remote-session://dev/current-erome"],
        }))
        .expect("status");

        let stale_runtime_keys = super::duplicate_legacy_owned_runtime_keys(
            &current_runtime_keys,
            "2.6.2",
            2,
            262,
            &owner_status,
        );

        assert!(
            stale_runtime_keys.is_empty(),
            "a daemon must not prune duplicate runtime keys from a newer or higher-ranked owner"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn disk_replace_handoff_target_strips_deleted_suffix() {
        // Linux marks an overwritten /proc/self/exe with the " (deleted)"
        // suffix; the un-suffixed path is the NEW on-disk binary to hand off to.
        assert_eq!(
            super::disk_replace_handoff_target("/home/pi/.yggterm/bin/yggterm-headless (deleted)"),
            Some(std::path::PathBuf::from("/home/pi/.yggterm/bin/yggterm-headless")),
        );
        // A live (un-replaced) link must NOT trigger a handoff.
        assert_eq!(
            super::disk_replace_handoff_target("/home/pi/.yggterm/bin/yggterm-headless"),
            None,
        );
        // Defensive: a bare suffix yields no usable path.
        assert_eq!(super::disk_replace_handoff_target(" (deleted)"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn self_retire_handoff_kill_switch_parses_truthy_values() {
        // Default (unset) and explicit off values keep the handoff ENABLED.
        assert!(!super::parse_self_retire_handoff_disabled(None));
        assert!(!super::parse_self_retire_handoff_disabled(Some("")));
        assert!(!super::parse_self_retire_handoff_disabled(Some("0")));
        assert!(!super::parse_self_retire_handoff_disabled(Some("false")));
        assert!(!super::parse_self_retire_handoff_disabled(Some("FALSE")));
        // Any other non-empty value disables it (reverts to cold shutdown).
        assert!(super::parse_self_retire_handoff_disabled(Some("1")));
        assert!(super::parse_self_retire_handoff_disabled(Some("true")));
        assert!(super::parse_self_retire_handoff_disabled(Some(" yes ")));
    }

    #[cfg(unix)]
    #[test]
    fn canonical_hot_restart_executable_requires_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "yggterm-hot-restart-exe-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        fs::create_dir_all(&root).expect("create temp dir");
        let executable = root.join("yggterm-headless");
        fs::write(&executable, b"#!/bin/sh\n").expect("write executable");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("chmod executable");

        let canonical =
            super::canonical_hot_restart_executable(executable.to_str().expect("utf8 temp path"))
                .expect("executable should validate");
        assert_eq!(
            canonical,
            executable.canonicalize().expect("canonical path")
        );

        let non_executable = root.join("not-executable");
        fs::write(&non_executable, b"").expect("write non executable");
        assert!(
            super::canonical_hot_restart_executable(
                non_executable.to_str().expect("utf8 temp path"),
            )
            .is_err()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn parse_versioned_server_socket_name_accepts_semver_socket_names() {
        let parsed = parse_versioned_server_socket_name(Path::new("/tmp/server-2-1-5.sock"));
        assert_eq!(parsed, Some((2, 1, 5)));
        assert_eq!(
            parse_versioned_server_socket_name(Path::new("/tmp/server.sock")),
            None
        );
    }

    #[test]
    fn server_version_strictly_newer_drives_stale_daemon_retire() {
        // The backstop that prevents the split-brain blank-on-update: an older
        // daemon retires when a strictly-newer one is live.
        assert!(server_version_is_strictly_newer("2.8.22", "2.8.21"));
        assert!(server_version_is_strictly_newer("2.8.10", "2.8.9")); // numeric, not lexical
        assert!(server_version_is_strictly_newer("2.9.0", "2.8.99"));
        assert!(server_version_is_strictly_newer("3.0.0", "2.8.22"));
        // Same or older must NOT retire (so the NEWEST daemon never retires itself).
        assert!(!server_version_is_strictly_newer("2.8.22", "2.8.22"));
        assert!(!server_version_is_strictly_newer("2.8.21", "2.8.22"));
        assert!(!server_version_is_strictly_newer("2.8.9", "2.8.10"));
        // Unparseable => never retire (safe default).
        assert!(!server_version_is_strictly_newer("garbage", "2.8.22"));
        assert!(!server_version_is_strictly_newer("2.8.22", "garbage"));
        // Tolerate a pre-release/build suffix.
        assert!(server_version_is_strictly_newer("2.8.22-rc1", "2.8.21"));
    }

    #[cfg(unix)]
    #[test]
    fn default_endpoint_keeps_short_home_socket_in_home() {
        let home = Path::new("/tmp/yggterm-short-home");
        let endpoint = default_endpoint(home);
        let super::ServerEndpoint::UnixSocket(path) = endpoint else {
            panic!("unix builds should use a unix socket endpoint")
        };
        assert!(path.starts_with(home));
        assert!(unix_socket_path_fits_platform(&path));
    }

    #[cfg(unix)]
    #[test]
    fn default_endpoint_uses_runtime_socket_for_long_home() {
        let home = std::env::temp_dir()
            .join("yggterm-long-home")
            .join("nested")
            .join("path")
            .join("that")
            .join("would")
            .join("overflow")
            .join("linux")
            .join("sun-path")
            .join("when")
            .join("the")
            .join("versioned")
            .join("server")
            .join("socket")
            .join("is")
            .join("inside")
            .join("the")
            .join("home");
        let endpoint = default_endpoint(&home);
        let super::ServerEndpoint::UnixSocket(path) = endpoint else {
            panic!("unix builds should use a unix socket endpoint")
        };
        assert!(!path.starts_with(&home));
        assert!(unix_socket_path_fits_platform(&path));
        assert_eq!(
            parse_versioned_server_socket_name(&path),
            parse_versioned_server_socket_name(Path::new(&format!(
                "/tmp/server-{}.sock",
                super::SERVER_PROTOCOL_VERSION.replace('.', "-")
            )))
        );
    }

    #[test]
    fn fallback_daemon_version_picks_highest_reachable() {
        // finding-gui-only-deploy-version-socket-mismatch: a newer GUI whose own
        // socket is absent must fall back to the HIGHEST *reachable* daemon — never
        // a higher-numbered but DEAD socket (a stale symlink from a prior run).
        let probed = [
            ((2, 9, 17), false), // dead stale socket — must be ignored
            ((2, 9, 15), true),  // the live daemon
            ((2, 9, 14), true),  // a back-alias to the same daemon, lower version
            ((2, 1, 5), true),   // protocol-version alias, lower still
        ];
        assert_eq!(
            super::select_fallback_daemon_version(&probed),
            Some((2, 9, 15)),
            "must pick the highest REACHABLE version, skipping the dead 2.9.17 socket"
        );
        // Nothing reachable → None (caller keeps its own endpoint so a daemon spawns).
        let none_reachable = [((2, 9, 15), false), ((2, 1, 5), false)];
        assert_eq!(super::select_fallback_daemon_version(&none_reachable), None);
        assert_eq!(super::select_fallback_daemon_version(&[]), None);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_legacy_unix_daemons_ignores_missing_socket_dir() {
        let endpoint = super::ServerEndpoint::UnixSocket(
            std::env::temp_dir()
                .join(format!(
                    "yggterm-missing-socket-dir-{}-{}",
                    std::process::id(),
                    super::current_millis()
                ))
                .join("server-2-1-35.sock"),
        );
        cleanup_legacy_unix_daemons(&endpoint).expect("missing socket dir should be harmless");
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_legacy_unix_daemons_keeps_reachable_legacy_socket() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-reachable-legacy-socket-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let sockets_dir = root.join("home").join(".yggterm");
        fs::create_dir_all(&sockets_dir).expect("create socket dir");
        let current = sockets_dir.join("server-2-1-48.sock");
        let legacy = sockets_dir.join("server-2-1-47.sock");
        let _listener =
            std::os::unix::net::UnixListener::bind(&legacy).expect("bind legacy socket");
        let endpoint = super::ServerEndpoint::UnixSocket(current);

        cleanup_legacy_unix_daemons(&endpoint).expect("cleanup should keep reachable legacy");

        assert!(
            legacy.exists(),
            "reachable legacy socket must not be unlinked"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_daemon_cleanup_protects_owner_but_not_stale_preserved_sidecar() {
        fn status_for_cleanup_test(
            version: &str,
            pid: u32,
            owned_keys: &[&str],
            preserved_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": 0,
                "server_pid": pid,
                "host_kind": "linux-controlled-dock",
                "host_detail": "test",
                "embedded_surface_supported": false,
                "bridge_enabled": false,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
                "preserved_terminal_owner_count": preserved_keys.len(),
                "preserved_terminal_owner_keys": preserved_keys,
            }))
            .expect("test status")
        }

        let stale_sidecar = status_for_cleanup_test(
            "2.1.170",
            170,
            &[],
            &["local://kept", "local://closed"],
            &["local://kept", "local://closed"],
        );
        let actual_owner = status_for_cleanup_test(
            "2.1.170",
            163,
            &["local://kept", "local://closed"],
            &[],
            &["local://kept", "local://closed"],
        );
        let owned_runtime =
            status_for_cleanup_test("2.1.170", 171, &["local://kept"], &[], &["local://kept"]);
        let unregistered_owned_runtime = status_for_cleanup_test(
            "2.1.170",
            172,
            &["remote-session://practice/new-work"],
            &[],
            &["remote-session://practice/new-work"],
        );
        let ghost_owned_runtime = status_for_cleanup_test(
            "2.1.170",
            177,
            &["local://closed"],
            &[],
            &["local://closed"],
        );
        let current_runtime = status_for_cleanup_test(
            super::SERVER_PROTOCOL_VERSION,
            220,
            &["local://kept"],
            &[],
            &["local://kept"],
        );
        let current_version_terminal_keys =
            HashSet::from(["local://kept".to_string(), "local://closed".to_string()]);
        let preserved_owner_pids = HashSet::from([163]);
        let preserved_owner_runtime_keys = HashSet::from(["local://kept".to_string()]);
        let exact_preserved_owner =
            status_for_cleanup_test("2.1.170", 163, &["local://kept"], &[], &["local://kept"]);

        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                163,
                Some(&exact_preserved_owner),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &HashSet::new(),
                true,
            ),
            "a clean preserved owner that directly owns the registry key must not be killed; the registry is the handoff route, not cleanup permission"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                163,
                Some(&actual_owner),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &HashSet::new(),
                true,
            ),
            "the endpoint in the hot-update owner registry is still a session-survival root"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                163,
                Some(&actual_owner),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &current_version_terminal_keys,
                true,
            ),
            "a registry owner remains the session-survival root even if another daemon reports the same runtime keys"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                170,
                Some(&stale_sidecar),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                false,
                &HashSet::new(),
                true,
            ),
            "a preserved-only sidecar must not inherit protection from unrelated home runtime activity"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                171,
                Some(&owned_runtime),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                false,
                &HashSet::new(),
                false,
            ),
            "a daemon that directly owns a PTY is protected even if it is not the registry endpoint"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                171,
                Some(&owned_runtime),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &HashSet::new(),
                false,
            ),
            "once the registry endpoint is clean, duplicate non-registry owners for the same keys should retire"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                172,
                Some(&unregistered_owned_runtime),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                true,
                &HashSet::new(),
                false,
            ),
            "a stale daemon with a direct PTY key missing from the registry must survive cleanup; session survival beats cleanup"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                177,
                Some(&ghost_owned_runtime),
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                true,
                false,
                &HashSet::new(),
                true,
            ),
            "cleanup cannot prove an unknown direct PTY is closed; explicit close metadata must retire it before cleanup may reap the owner"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                170,
                Some(&stale_sidecar),
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
                &HashSet::new(),
                true,
            ),
            "without current duplicate ownership evidence, cleanup remains conservative for legacy homes"
        );
        assert!(
            !linux_daemon_runtime_activity_protected_for_cleanup(
                163,
                Some(&actual_owner),
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
                &current_version_terminal_keys,
                true,
            ),
            "an old daemon whose terminal keys are already owned by the current daemon must not stay protected"
        );
        assert!(
            linux_daemon_runtime_activity_protected_for_cleanup(
                220,
                Some(&current_runtime),
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
                &current_version_terminal_keys,
                false,
            ),
            "the current daemon that owns live terminal keys is protected"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_daemon_cleanup_keeps_newest_clean_startup_bridge_sidecar() {
        fn status_for_cleanup_test(
            version: &str,
            pid: u32,
            owned_keys: &[&str],
            preserved_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": pid as u64,
                "server_pid": pid,
                "host_kind": "linux-controlled-dock",
                "host_detail": "test",
                "embedded_surface_supported": false,
                "bridge_enabled": false,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
                "preserved_terminal_owner_count": preserved_keys.len(),
                "preserved_terminal_owner_keys": preserved_keys,
            }))
            .expect("test status")
        }

        let preserved_owner_pids = HashSet::from([163]);
        let preserved_owner_runtime_keys =
            HashSet::from(["local://kept-a".to_string(), "local://kept-b".to_string()]);
        let statuses = HashMap::from([
            (
                163,
                status_for_cleanup_test(
                    "2.1.163",
                    163,
                    &[],
                    &[],
                    &["local://kept-a", "local://kept-b", "local://closed"],
                ),
            ),
            (
                182,
                status_for_cleanup_test(
                    "2.1.182",
                    182,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
            (
                184,
                status_for_cleanup_test(
                    "2.1.184",
                    184,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
            (
                185,
                status_for_cleanup_test(
                    SERVER_PROTOCOL_VERSION,
                    185,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
        ]);

        assert_eq!(
            linux_cleanup_startup_bridge_sidecar_pid(
                &statuses,
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                185,
            ),
            Some(184),
            "the newest clean preserved-only sidecar must survive initial cleanup so startup can retarget it instead of handoffing from the old owner with ghost keys"
        );

        let clean_owner_statuses = HashMap::from([
            (
                163,
                status_for_cleanup_test(
                    "2.1.163",
                    163,
                    &["local://kept-a", "local://kept-b"],
                    &[],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
            (
                184,
                status_for_cleanup_test(
                    "2.1.184",
                    184,
                    &[],
                    &["local://kept-a", "local://kept-b"],
                    &["local://kept-a", "local://kept-b"],
                ),
            ),
        ]);
        assert_eq!(
            linux_cleanup_startup_bridge_sidecar_pid(
                &clean_owner_statuses,
                &preserved_owner_pids,
                &preserved_owner_runtime_keys,
                185,
            ),
            None,
            "a preserved-only startup bridge sidecar must retire once the registry's real PTY owner is already clean"
        );
    }

    #[cfg(unix)]
    #[test]
    fn versioned_server_socket_alias_candidates_include_client_instance_versions() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-alias-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let sockets_dir = root.join("home").join(".yggterm");
        let client_instances = sockets_dir.join("client-instances");
        fs::create_dir_all(&client_instances).expect("create client instances dir");
        let current = sockets_dir.join("server-2-1-5.sock");
        // default_endpoint relocates the socket out of `sockets_dir` when the
        // path would exceed SUN_LEN (e.g. a deep $TMPDIR), so create the resolved
        // parent before writing — a no-op when the socket stays in sockets_dir.
        if let Some(parent) = current.parent() {
            fs::create_dir_all(parent).expect("create current socket parent");
        }
        fs::write(&current, b"").expect("write current socket placeholder");
        fs::create_dir_all(client_instances.join("unix--home-pi--yggterm-server-2-1-4-sock"))
            .expect("create old instance dir");

        let candidates = versioned_server_socket_alias_candidates(&current);

        assert!(candidates.contains(&sockets_dir.join("server-2-1-4.sock")));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn versioned_server_status_probe_paths_dedupe_symlink_aliases_before_status() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-status-dedupe-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let sockets_dir = root.join("home").join(".yggterm");
        fs::create_dir_all(&sockets_dir).expect("create sockets dir");
        let super::ServerEndpoint::UnixSocket(current) = super::default_endpoint(&sockets_dir)
        else {
            panic!("unix test requires unix socket endpoint");
        };
        // default_endpoint relocates the socket out of `sockets_dir` when the
        // path would exceed SUN_LEN (e.g. a deep $TMPDIR), so create the resolved
        // parent before writing — a no-op when the socket stays in sockets_dir.
        if let Some(parent) = current.parent() {
            fs::create_dir_all(parent).expect("create current socket parent");
        }
        fs::write(&current, b"").expect("write current socket placeholder");
        std::os::unix::fs::symlink(&current, sockets_dir.join("server-2-1-102.sock"))
            .expect("create 2.1.102 alias");
        std::os::unix::fs::symlink(&current, sockets_dir.join("server-2-1-101.sock"))
            .expect("create 2.1.101 alias");

        let paths = super::versioned_server_status_probe_paths(&sockets_dir);

        assert_eq!(paths, vec![current]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn versioned_server_status_probe_paths_can_skip_current_endpoint_aliases() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-status-exclude-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let sockets_dir = root.join("home").join(".yggterm");
        fs::create_dir_all(&sockets_dir).expect("create sockets dir");
        let super::ServerEndpoint::UnixSocket(current) = super::default_endpoint(&sockets_dir)
        else {
            panic!("unix test requires unix socket endpoint");
        };
        let legacy = sockets_dir.join("server-2-6-2.sock");
        // default_endpoint relocates the socket out of `sockets_dir` when the
        // path would exceed SUN_LEN (e.g. a deep $TMPDIR), so create the resolved
        // parent before writing — a no-op when the socket stays in sockets_dir.
        if let Some(parent) = current.parent() {
            fs::create_dir_all(parent).expect("create current socket parent");
        }
        fs::write(&current, b"").expect("write current socket placeholder");
        fs::write(&legacy, b"").expect("write legacy socket placeholder");
        std::os::unix::fs::symlink(&current, sockets_dir.join("server-2-1-102.sock"))
            .expect("create current alias");

        let excluded = super::ServerEndpoint::UnixSocket(current);
        let paths =
            super::versioned_server_status_probe_paths_excluding_endpoint(&sockets_dir, &excluded);

        assert_eq!(paths, vec![legacy]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn server_endpoint_identity_treats_symlink_alias_as_same_target() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-server-endpoint-identity-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        fs::create_dir_all(&root).expect("create temp dir");
        let current = root.join("server-2-7-18.sock");
        let alias = root.join("server-2-1-0.sock");
        let other = root.join("server-2-7-9.sock");
        fs::write(&current, b"current").expect("write current");
        fs::write(&other, b"other").expect("write other");
        std::os::unix::fs::symlink(&current, &alias).expect("create alias");

        assert!(super::server_endpoints_same_target(
            &super::ServerEndpoint::UnixSocket(alias),
            &super::ServerEndpoint::UnixSocket(current)
        ));
        assert!(!super::server_endpoints_same_target(
            &super::ServerEndpoint::UnixSocket(root.join("server-2-7-18.sock")),
            &super::ServerEndpoint::UnixSocket(other)
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn daemon_socket_lock_is_exclusive_for_socket_path() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-lock-test-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        fs::create_dir_all(&root).expect("create lock root");
        let socket = root.join("server-2-1-148.sock");

        let first = super::try_acquire_daemon_socket_lock(&socket, &root)
            .expect("first lock result")
            .expect("first lock");
        let second =
            super::try_acquire_daemon_socket_lock(&socket, &root).expect("second lock result");
        assert!(
            second.is_none(),
            "second daemon must not acquire the same socket lock"
        );
        drop(first);
        let third =
            super::try_acquire_daemon_socket_lock(&socket, &root).expect("third lock result");
        assert!(third.is_some(), "lock must release when owner exits");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn versioned_socket_alias_policy_only_targets_older_versions() {
        let current = (2, 1, 32);
        assert!(versioned_socket_alias_is_legacy(current, (2, 1, 31)));
        assert!(!versioned_socket_alias_is_legacy(current, current));
        assert!(!versioned_socket_alias_is_legacy(current, (2, 1, 33)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daemon_binary_is_legacy_allows_deleted_current_install_path() {
        let current = Path::new("/home/pi/.yggterm/bin/yggterm");
        assert!(!daemon_binary_is_legacy(
            current,
            "/home/pi/.yggterm/bin/yggterm",
            Some(Path::new("/home/pi/.yggterm/bin/yggterm (deleted)")),
        ));
        assert!(!daemon_binary_is_legacy(
            current,
            "/home/pi/.yggterm/bin/yggterm",
            Some(Path::new("/home/pi/.yggterm/bin/yggterm")),
        ));
        assert!(!daemon_binary_is_legacy(
            current,
            "/home/pi/.yggterm/bin/yggterm-headless",
            Some(Path::new("/home/pi/.yggterm/bin/yggterm-headless")),
        ));
        assert!(daemon_binary_is_legacy(
            current,
            "/tmp/old-yggterm",
            Some(Path::new("/tmp/old-yggterm")),
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_bridge_process_detection_matches_remote_resume_stdio_bridges() {
        let resume_parts = vec![
            "/home/pi/.yggterm/bin/yggterm".to_string(),
            "server".to_string(),
            "remote".to_string(),
            "resume-codex".to_string(),
            "019dbddf".to_string(),
            "/home/pi/gh/yggterm".to_string(),
            "--require-existing".to_string(),
        ];
        assert!(super::linux_yggterm_server_process_is_live_bridge(
            &resume_parts
        ));
        let start_parts = vec![
            "/home/pi/.yggterm/bin/yggterm".to_string(),
            "server".to_string(),
            "remote".to_string(),
            "start-codex".to_string(),
            "019dbddf".to_string(),
            "/home/pi/gh/yggterm".to_string(),
        ];
        assert!(super::linux_yggterm_server_process_is_live_bridge(
            &start_parts
        ));
        let short_lived_probe = vec![
            "/home/pi/.yggterm/bin/yggterm".to_string(),
            "server".to_string(),
            "remote".to_string(),
            "saved-codex-session-exists".to_string(),
            "019dbddf".to_string(),
        ];
        assert!(!super::linux_yggterm_server_process_is_live_bridge(
            &short_lived_probe
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn legacy_daemon_reap_only_applies_within_same_home() {
        let current_home = Path::new("/home/pi/.yggterm");
        let other_home = Path::new("/tmp/yggterm-other-home");
        assert!(legacy_daemon_reap_applies_to_home(
            Some(current_home),
            Some(current_home)
        ));
        assert!(!legacy_daemon_reap_applies_to_home(
            Some(current_home),
            Some(other_home)
        ));
        assert!(!legacy_daemon_reap_applies_to_home(
            Some(current_home),
            None
        ));
        assert!(!legacy_daemon_reap_applies_to_home(
            None,
            Some(current_home)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn orphan_daemon_reap_only_applies_within_same_home() {
        let current_home = Path::new("/home/pi/.yggterm");
        let other_home = Path::new("/tmp/yggterm-other-home");
        assert!(orphan_daemon_reap_applies_to_home(
            Some(current_home),
            Some(current_home)
        ));
        assert!(!orphan_daemon_reap_applies_to_home(
            Some(current_home),
            Some(other_home)
        ));
        assert!(!orphan_daemon_reap_applies_to_home(
            Some(current_home),
            None
        ));
        assert!(!orphan_daemon_reap_applies_to_home(
            None,
            Some(current_home)
        ));
    }

    #[test]
    fn write_persisted_state_creates_parent_and_round_trips() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-state-roundtrip-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let state_path = root.join("nested").join("server-state.json");
        let expected = PersistedDaemonState {
            active_session_path: Some("remote-session://jojo/demo".to_string()),
            active_view_mode: super::WorkspaceViewMode::Terminal,
            ssh_targets: Vec::new(),
            remote_machines: Vec::new(),
            stored_sessions: Vec::new(),
            live_sessions: vec![PersistedLiveSession {
                key: "remote-session://jojo/demo".to_string(),
                id: "demo".to_string(),
                title: "Demo".to_string(),
                kind: super::SessionKind::Codex,
                keep_alive: true,
                ssh_target: "jojo".to_string(),
                prefix: None,
                cwd: Some("/home/pi".to_string()),
                remote_launch_action: None,
                storage_path: None,
                restore_reason: None,
            }],
            session_pty_grids: Vec::new(),
        };

        write_persisted_state(&state_path, &expected).expect("write daemon state");
        let loaded = load_persisted_state(&state_path)
            .expect("load daemon state")
            .expect("persisted daemon state");
        assert_eq!(loaded.active_session_path, expected.active_session_path);
        assert_eq!(loaded.live_sessions, expected.live_sessions);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_persisted_state_keeps_previous_copy_before_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-daemon-state-backup-{}-{}",
            std::process::id(),
            super::current_millis()
        ));
        let state_path = root.join("server-state.json");
        let first = PersistedDaemonState {
            active_session_path: Some("remote-session://dev/first".to_string()),
            active_view_mode: super::WorkspaceViewMode::Terminal,
            ssh_targets: Vec::new(),
            remote_machines: Vec::new(),
            stored_sessions: Vec::new(),
            live_sessions: Vec::new(),
            session_pty_grids: Vec::new(),
        };
        let second = PersistedDaemonState {
            active_session_path: Some("remote-session://dev/second".to_string()),
            active_view_mode: super::WorkspaceViewMode::Rendered,
            ssh_targets: Vec::new(),
            remote_machines: Vec::new(),
            stored_sessions: Vec::new(),
            live_sessions: Vec::new(),
            session_pty_grids: Vec::new(),
        };

        write_persisted_state(&state_path, &first).expect("write first daemon state");
        write_persisted_state(&state_path, &second).expect("write second daemon state");

        let backup_path = root.join("server-state.previous.json");
        let backup = load_persisted_state(&backup_path)
            .expect("load backup daemon state")
            .expect("backup daemon state");
        let current = load_persisted_state(&state_path)
            .expect("load current daemon state")
            .expect("current daemon state");
        assert_eq!(backup.active_session_path, first.active_session_path);
        assert_eq!(current.active_session_path, second.active_session_path);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_prewarm_is_opt_in_and_skips_when_disabled() {
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("local://shell", false),
            Some("disabled_by_env")
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("codex://session", false),
            Some("disabled_by_env")
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag(
                "live::fc99adfa-d0b1-4e50-a27d-cbb3a1602b4b",
                true,
            ),
            Some("runtime_owned_live_session")
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("local://shell", true),
            None
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("codex://session", true),
            None
        );
        assert_eq!(
            super::startup_prewarm_skip_reason_with_flag("remote-session://dev/session", true),
            None
        );
    }

    #[test]
    fn startup_prewarm_seeds_active_remote_snapshot_only() {
        assert!(super::startup_prewarm_should_seed_remote_snapshot(
            "remote-session://dev/session",
            Some("remote-session://dev/session")
        ));
        assert!(!super::startup_prewarm_should_seed_remote_snapshot(
            "remote-session://dev/background",
            Some("remote-session://dev/session")
        ));
        assert!(!super::startup_prewarm_should_seed_remote_snapshot(
            "remote-session://dev/background",
            None
        ));
        assert!(!super::startup_prewarm_should_seed_remote_snapshot(
            "local://shell",
            Some("local://shell")
        ));
    }

    #[test]
    fn terminal_ensure_skips_remote_snapshot_seed_for_background_latency_budget() {
        assert!(!super::terminal_ensure_should_seed_remote_snapshot(
            "remote-session://dev/session",
            false
        ));
        assert!(super::terminal_ensure_should_seed_remote_snapshot(
            "remote-session://dev/session",
            true
        ));
        assert!(!super::terminal_ensure_should_seed_remote_snapshot(
            "local://shell",
            true
        ));
    }

    #[test]
    fn terminal_sidebar_snapshot_from_screen_prefers_non_empty_tail() {
        let snapshot = "\npi@dev:~/gh/yggterm$ sleep 3\npi@dev:~/gh/yggterm$ \n";
        let (status_line, terminal_lines) =
            terminal_sidebar_snapshot_from_screen(snapshot).expect("screen snapshot");
        assert_eq!(status_line, "pi@dev:~/gh/yggterm$");
        assert_eq!(
            terminal_lines,
            vec![
                "pi@dev:~/gh/yggterm$ sleep 3".to_string(),
                "pi@dev:~/gh/yggterm$".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_sidebar_snapshot_from_screen_returns_none_for_blank_surface() {
        assert_eq!(terminal_sidebar_snapshot_from_screen("\n  \n\r\n"), None);
    }

    #[test]
    fn collect_remote_copy_candidates_do_not_clone_machine_session_lists() {
        let machines = vec![RemoteMachineSnapshot {
            machine_key: "jojo".to_string(),
            label: "jojo".to_string(),
            ssh_target: "jojo".to_string(),
            prefix: Some("sudo -u pi".to_string()),
            remote_binary_expr: Some("$HOME/.yggterm/bin/yggterm".to_string()),
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: vec![
                RemoteScannedSession {
                    session_path: remote_scanned_session_path("jojo", "one"),
                    session_id: "one".to_string(),
                    cwd: "/home/pi".to_string(),
                    started_at: "2026-04-01T00:00:00Z".to_string(),
                    modified_epoch: 1,
                    event_count: 1,
                    user_message_count: 1,
                    assistant_message_count: 0,
                    title_hint: "One".to_string(),
                    recent_context: "USER: one".to_string(),
                    cached_precis: None,
                    cached_summary: None,
                    live_runtime: false,
                    storage_path: "/home/pi/.codex/sessions/one.jsonl".to_string(),
                },
                RemoteScannedSession {
                    session_path: remote_scanned_session_path("jojo", "two"),
                    session_id: "two".to_string(),
                    cwd: "/srv/app".to_string(),
                    started_at: "2026-04-01T00:01:00Z".to_string(),
                    modified_epoch: 2,
                    event_count: 2,
                    user_message_count: 1,
                    assistant_message_count: 1,
                    title_hint: "Two".to_string(),
                    recent_context: String::new(),
                    cached_precis: Some("precis".to_string()),
                    cached_summary: Some("summary".to_string()),
                    live_runtime: false,
                    storage_path: "/home/pi/.codex/sessions/two.jsonl".to_string(),
                },
            ],
        }];

        let candidates = collect_remote_copy_candidates(&machines);

        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            let machine = candidate
                .remote_machine
                .expect("candidate should keep machine routing");
            assert_eq!(machine.machine_key, "jojo");
            assert!(machine.sessions.is_empty());
            assert_eq!(machine.prefix.as_deref(), Some("sudo -u pi"));
        }
        assert_eq!(machines[0].sessions.len(), 2);
    }

    #[test]
    fn collect_remote_copy_candidates_preserve_machine_order_across_duplicates() {
        let machines = vec![
            RemoteMachineSnapshot {
                machine_key: "jojo".to_string(),
                label: "jojo".to_string(),
                ssh_target: "jojo".to_string(),
                prefix: None,
                remote_binary_expr: None,
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![
                    RemoteScannedSession {
                        session_path: remote_scanned_session_path("jojo", "one"),
                        session_id: "one".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-04-01T00:00:00Z".to_string(),
                        modified_epoch: 1,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 0,
                        title_hint: "One".to_string(),
                        recent_context: String::new(),
                        cached_precis: None,
                        cached_summary: None,
                        live_runtime: false,
                        storage_path: "/home/pi/.codex/sessions/one.jsonl".to_string(),
                    },
                    RemoteScannedSession {
                        session_path: remote_scanned_session_path("jojo", "two"),
                        session_id: "two".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-04-01T00:01:00Z".to_string(),
                        modified_epoch: 2,
                        event_count: 2,
                        user_message_count: 1,
                        assistant_message_count: 1,
                        title_hint: "Two".to_string(),
                        recent_context: String::new(),
                        cached_precis: None,
                        cached_summary: None,
                        live_runtime: false,
                        storage_path: "/home/pi/.codex/sessions/two.jsonl".to_string(),
                    },
                ],
            },
            RemoteMachineSnapshot {
                machine_key: "oc".to_string(),
                label: "oc".to_string(),
                ssh_target: "oc".to_string(),
                prefix: None,
                remote_binary_expr: None,
                remote_deploy_state: RemoteDeployState::Ready,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![RemoteScannedSession {
                    session_path: remote_scanned_session_path("oc", "three"),
                    session_id: "three".to_string(),
                    cwd: "/root".to_string(),
                    started_at: "2026-04-01T00:02:00Z".to_string(),
                    modified_epoch: 3,
                    event_count: 3,
                    user_message_count: 2,
                    assistant_message_count: 1,
                    title_hint: "Three".to_string(),
                    recent_context: String::new(),
                    cached_precis: None,
                    cached_summary: None,
                    live_runtime: false,
                    storage_path: "/root/.codex/sessions/three.jsonl".to_string(),
                }],
            },
        ];

        let candidates = collect_remote_copy_candidates(&machines);
        let keys = candidates
            .into_iter()
            .map(|candidate| {
                candidate
                    .remote_machine
                    .expect("remote machine")
                    .machine_key
            })
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec!["jojo".to_string(), "jojo".to_string(), "oc".to_string()]
        );
    }

    #[test]
    fn terminal_launch_command_for_remote_session_keeps_server_owned_resume_command() {
        let stored =
            "ssh oc '$HOME/.yggterm/bin/yggterm' server remote resume-codex 019c --require-existing"
                .to_string();
        let legacy = Some("ssh oc legacy-direct-attach 019c".to_string());

        let selected =
            terminal_launch_command_for_path("remote-session://oc/019c", stored.clone(), legacy);

        assert_eq!(selected, stored);
    }

    #[test]
    fn remote_resume_stale_attach_ignores_semantic_mismatch_without_breakage() {
        assert!(!remote_resume_stale_attach(
            true,
            3_500,
            false,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(remote_resume_stale_attach(
            false,
            901,
            false,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(!remote_resume_stale_attach(
            false,
            3_500,
            false,
            REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(remote_resume_stale_attach(
            true,
            3_500,
            true,
            REMOTE_START_CODEX_ATTACH_STARTUP_GRACE_MS
        ));
    }

    #[test]
    fn remote_resume_saved_session_mismatch_does_not_auto_restart_live_runtime() {
        assert!(!remote_resume_saved_session_mismatch_requires_restart(
            true,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS - 1,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(!remote_resume_saved_session_mismatch_requires_restart(
            true,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(!remote_resume_saved_session_mismatch_requires_restart(
            true,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS * 30,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(!remote_resume_saved_session_mismatch_requires_restart(
            false,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(!remote_resume_saved_session_mismatch_requires_restart(
            true,
            false,
            REMOTE_ATTACH_STARTUP_GRACE_MS,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
        assert!(!remote_resume_saved_session_mismatch_requires_restart(
            true,
            true,
            REMOTE_ATTACH_STARTUP_GRACE_MS,
            false,
            REMOTE_ATTACH_STARTUP_GRACE_MS
        ));
    }

    #[test]
    fn protected_remote_runtime_blocks_semantic_restart_while_still_running() {
        let (needs_restart, blocked) =
            terminal_reuse_needs_restart(true, true, true, true, true, true, false);

        assert!(
            !needs_restart,
            "a still-running protected remote runtime must not be replaced because early output or launch spec looks stale"
        );
        assert!(
            blocked,
            "the trace must expose that restart was requested but blocked by session survival"
        );
    }

    #[test]
    fn non_keep_alive_remote_runtime_can_still_restart_after_recovery_failure() {
        let (needs_restart, blocked) =
            terminal_reuse_needs_restart(true, true, false, true, false, false, true);

        assert!(needs_restart);
        assert!(!blocked);
    }

    #[test]
    fn protected_remote_runtime_restarts_only_after_process_exit() {
        let (needs_restart, blocked) =
            terminal_reuse_needs_restart(false, true, true, false, false, false, true);

        assert!(needs_restart);
        assert!(
            !blocked,
            "session survival protects a live runtime; if the runtime is already gone, restart remains the recovery path"
        );
    }

    #[test]
    fn terminal_read_recovers_missing_runtime_owned_session_before_read() {
        let terminals = TerminalManager::new();
        assert_eq!(
            super::terminal_read_runtime_recovery_reason(
                "remote-session://practice/019e3648",
                "remote-session://practice/019e3648",
                &terminals,
            ),
            Some("missing_runtime_before_read")
        );
        assert_eq!(
            super::terminal_read_runtime_recovery_reason(
                "local://shell",
                "local://shell",
                &terminals,
            ),
            None
        );
    }

    #[test]
    fn keep_alive_preserved_owner_mismatch_stays_attached() {
        assert!(
            !preserved_owner_saved_session_mismatch_should_detach(
                "remote-session://practice/019e2ade",
                true,
                false,
                true,
            ),
            "saved-session text mismatch is recovery evidence for keep-alive, not detach permission"
        );
        assert!(preserved_owner_saved_session_mismatch_should_detach(
            "remote-session://practice/019e2ade",
            false,
            false,
            true,
        ));
        assert!(
            !preserved_owner_saved_session_mismatch_should_detach(
                "remote-session://practice/019e2ade",
                false,
                true,
                true,
            ),
            "update-handoff protected runtimes must stay attached even when early text heuristics look stale"
        );
        assert!(preserved_owner_saved_session_mismatch_should_detach(
            "local://shell",
            true,
            false,
            true,
        ));
        assert!(!preserved_owner_saved_session_mismatch_should_detach(
            "remote-session://practice/019e2ade",
            true,
            false,
            false,
        ));
    }

    #[test]
    fn remove_session_close_terminates_explicit_keep_alive_runtime() {
        assert!(
            !remove_session_should_detach_keep_alive_runtime(true),
            "ordinary close/remove is a destructive terminal close even for explicit keep-alive runtimes; app close is the detach boundary"
        );
        assert!(
            !remove_session_should_detach_keep_alive_runtime(false),
            "non-kept live sessions use ordinary close as a destructive runtime close"
        );
    }

    #[test]
    fn reachable_preserved_owner_candidate_prefers_live_owner_for_runtime_key() {
        fn status(
            version: &str,
            build_id: u64,
            pid: u32,
            owned_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": build_id,
                "server_pid": pid,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
            }))
            .expect("runtime status")
        }

        let runtime_key = "remote-session://practice/019e2ade";
        let statuses = vec![
            (
                super::ServerEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: 26101,
                },
                status("2.6.19", 19, 190, &[], &[runtime_key]),
            ),
            (
                super::ServerEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: 26100,
                },
                status("2.6.18", 18, 180, &[runtime_key], &[runtime_key]),
            ),
            (
                super::ServerEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: 26102,
                },
                status("2.6.20", 20, 200, &[], &["remote-session://dev/other"]),
            ),
        ];

        let (endpoint, owner_status) =
            preserved_owner_candidate_for_runtime_key(statuses, runtime_key, 200)
                .expect("candidate");

        assert_eq!(owner_status.server_pid, 180);
        assert_eq!(
            endpoint,
            super::ServerEndpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 26100,
            }
        );
    }

    /// Regression (telemetry campaign 2026-07-17): a daemon that merely
    /// REPRESENTS a runtime key (terminal_session_keys) without OWNING it
    /// (owned_terminal_session_keys) is a hollow bridge — it re-preserves the
    /// key toward an even older daemon and cannot serve a byte of it. It must
    /// never be selected as a preserved-owner candidate: 8 such entries kept
    /// `hot_update_handoff_active` pinned true for four days on the live host.
    #[test]
    fn preserved_owner_candidate_requires_actual_ownership() {
        fn status(
            version: &str,
            build_id: u64,
            pid: u32,
            owned_keys: &[&str],
            terminal_keys: &[&str],
        ) -> ServerRuntimeStatus {
            serde_json::from_value(serde_json::json!({
                "server_version": version,
                "server_build_id": build_id,
                "server_pid": pid,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": owned_keys.len(),
                "owned_terminal_session_keys": owned_keys,
                "terminal_session_count": terminal_keys.len(),
                "terminal_session_keys": terminal_keys,
            }))
            .expect("runtime status")
        }

        let runtime_key = "local://217cb4a6-hollow-chain";
        let represented_only = status("2.11.1", 11, 111, &[], &[runtime_key]);
        assert!(!super::preserved_owner_status_serves_runtime_key(
            &represented_only,
            runtime_key
        ));
        let owned = status("2.11.1", 11, 112, &[runtime_key], &[runtime_key]);
        assert!(super::preserved_owner_status_serves_runtime_key(
            &owned,
            runtime_key
        ));

        let statuses = vec![
            (
                super::ServerEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: 26110,
                },
                represented_only,
            ),
            (
                super::ServerEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: 26111,
                },
                status("2.11.0", 10, 110, &[], &[runtime_key]),
            ),
        ];
        assert!(
            preserved_owner_candidate_for_runtime_key(statuses, runtime_key, 999).is_none(),
            "daemons that only represent (never own) the key must not be adopted as owners"
        );
    }

    #[cfg(unix)]
    #[test]
    fn daemon_idle_shutdown_is_blocked_by_owned_terminal_sessions() {
        let home =
            std::env::temp_dir().join(format!("yggterm-idle-terminal-{}", std::process::id()));
        fs::create_dir_all(&home).expect("create temp home");
        let endpoint = super::ServerEndpoint::UnixSocket(home.join("server-test.sock"));
        let last_activity = std::sync::atomic::AtomicU64::new(0);

        assert!(
            !super::daemon_should_idle_shutdown(&home, &endpoint, &last_activity, 1, 1),
            "a daemon that still owns a terminal PTY must not idle-exit just because no GUI/client record is present"
        );

        let _ = fs::remove_dir_all(home);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_yggterm_home_from_environ_prefers_explicit_override() {
        let env = b"HOME=/home/pi\0YGGTERM_HOME=/tmp/yggterm-test\0";
        assert_eq!(
            linux_yggterm_home_from_environ_bytes(env),
            Some(std::path::PathBuf::from("/tmp/yggterm-test"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_yggterm_home_from_environ_falls_back_to_default_home_dir() {
        let env = b"HOME=/home/pi\0DISPLAY=:10\0";
        assert_eq!(
            linux_yggterm_home_from_environ_bytes(env),
            Some(std::path::PathBuf::from("/home/pi/.yggterm"))
        );
    }

    /// The source text of `pub enum Name {` through its matching close brace.
    fn extract_enum_block(source: &str, opener: &str) -> String {
        let start = source
            .find(opener)
            .unwrap_or_else(|| panic!("{opener} not found in daemon.rs"));
        let mut depth = 0usize;
        for (offset, ch) in source[start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return source[start..=start + offset].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced braces after {opener}");
    }

    /// The wire-protocol SHAPE STAMP — the repo-side replacement for the
    /// runtime build-id compatibility gate (deploy semantics 2026-07-17).
    ///
    /// Protocol compatibility across machines is now version-ordered, which is
    /// only sound if every wire change ships under a NEW version. This test
    /// enforces that at the moment of the change: it hashes the source text of
    /// `ServerRequest` + `ServerResponse` and compares against a stamp taken
    /// when the protocol last changed. If you edited either enum, this fails —
    /// bump the workspace version (Cargo.toml) AND re-stamp BOTH constants
    /// below in the SAME commit. Re-stamping without bumping defeats the fleet
    /// deploy semantics (two builds of one version with different wire shapes),
    /// so the stamped version must be the bumped one.
    ///
    /// Formatting/comment edits inside the enums re-trigger this; that
    /// over-trigger is deliberate — a spare version bump is cheap, a silent
    /// wire divergence is the lost-PTY latch storm of 2026-07-17.
    #[test]
    fn protocol_shape_stamp_forces_version_bump() {
        const STAMPED_AT_VERSION: &str = "2.11.4";
        const STAMPED_SHAPE_HASH: u64 = 0xb513bed5b007ef21;
        let source = include_str!("daemon.rs");
        let shape = format!(
            "{}\n{}",
            extract_enum_block(source, "pub enum ServerRequest {"),
            extract_enum_block(source, "pub enum ServerResponse {"),
        );
        let hash = crate::fnv1a_build_id(shape.as_bytes());
        assert_eq!(
            hash, STAMPED_SHAPE_HASH,
            "ServerRequest/ServerResponse source changed (computed hash {hash:#018x}). \
             Bump the workspace version in Cargo.toml and update STAMPED_AT_VERSION + \
             STAMPED_SHAPE_HASH to the new version and this hash IN THE SAME COMMIT.",
        );
        let stamped = parse_daemon_version_triple(STAMPED_AT_VERSION).expect("stamp parses");
        let current =
            parse_daemon_version_triple(SERVER_PROTOCOL_VERSION).expect("current parses");
        assert!(
            stamped <= current,
            "STAMPED_AT_VERSION {STAMPED_AT_VERSION} is ahead of CARGO_PKG_VERSION \
             {SERVER_PROTOCOL_VERSION} — the stamp must be taken at (not beyond) the shipped version",
        );
    }
}
