use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use time::OffsetDateTime;
use yggterm_core::SessionStore;

const TERMINAL_ENV_REMOVALS: &[&str] = &["NO_COLOR"];

/// `backend` while the daemon-owned shell is being obtained. ⛔ Not a result —
/// it exists so the file never claims an outcome that has not happened yet
/// ([[finding-a-set-is-not-a-fill]]): the old code wrote `daemon-shell`
/// *before* the attach was attempted, so a session that fell back to a plain
/// shell still vouched for persistence it did not have.
pub const ATTACH_BACKEND_PENDING: &str = "pending";
/// The daemon owns the PTY. This shell SURVIVES the terminal closing — the one
/// thing the product sells.
pub const ATTACH_BACKEND_DAEMON_SHELL: &str = "daemon-shell";
/// ⛔ A plain login shell in THIS process. It dies with its terminal.
pub const ATTACH_BACKEND_PLAIN_SHELL: &str = "plain-shell";
/// Nothing is attached: the daemon-owned shell was unavailable and the
/// substitution was refused.
pub const ATTACH_BACKEND_NONE: &str = "none";

/// The stable token a caller greps for on the terminal stream when the attach
/// downgraded. It goes to STDOUT, not stderr, because stdout is the stream the
/// PTY carries to every viewer — a phone streaming `server attach` over SSH
/// sees stdout and never sees stderr.
pub const PLAIN_SHELL_DOWNGRADE_MARKER: &str = "yggterm-attach-downgraded:";
/// The stable token on the refusal error. The caller's real signal is the
/// non-zero exit, but the token makes the reason machine-readable too.
pub const PLAIN_SHELL_REFUSED_MARKER: &str = "yggterm-attach-refused:";

/// Opt in to the non-persistent substitute on the `server attach` command line.
pub const PLAIN_SHELL_FALLBACK_FLAG: &str = "--allow-plain-shell-fallback";
/// Opt in for a caller that cannot edit the command line — notably a session
/// row whose `launch_command` was persisted by an older build.
pub const PLAIN_SHELL_FALLBACK_ENV: &str = "YGGTERM_ATTACH_ALLOW_PLAIN_SHELL_FALLBACK";

/// Whether [`run_attach`] may substitute a NON-PERSISTENT login shell when the
/// daemon-owned one cannot be obtained.
///
/// ⛔ **The default is [`PlainShellFallback::Refuse`], and the default IS the
/// fix.** `server attach` exists to hand back a shell the host daemon owns, so
/// that closing the terminal — or an iPhone suspending the app 20 s after it
/// backgrounds — does not end the session. Substituting a plain shell delivers
/// the exact thing the session is differentiated against, and the old code did
/// it on any error while announcing it with one `eprintln!`. On a phone stderr
/// is invisible, so the user learned about it by losing work
/// ([[bug-class-silent-downgrade-looks-like-a-correct-refusal]]).
///
/// A caller that genuinely wants "a shell, persistent or not" can still say so,
/// but it must say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlainShellFallback {
    #[default]
    Refuse,
    Allow,
}

/// What [`run_attach`] does once the daemon-owned shell has proven unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachDowngradeDecision {
    /// Exec a plain, NON-PERSISTENT login shell — recorded and announced first.
    SubstitutePlainShell,
    /// ⛔ Refuse, and fail the command so the caller's exit status carries it.
    Refuse,
}

/// Why `backend` is not [`ATTACH_BACKEND_DAEMON_SHELL`].
///
/// `backend` answers *what is holding this shell*; this answers *what was asked
/// for and could not be delivered, and when*. It is the programmatic half of
/// the signal: a client that cannot read the terminal stream (or that wants to
/// check after the fact) reads this out of `session.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonShellUnavailable {
    pub at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachMetadata {
    pub uuid: String,
    pub backend: String,
    pub shell: String,
    pub hostname: String,
    pub cwd: String,
    pub attach_count: u64,
    pub started_at: String,
    pub last_attached_at: String,
    /// Absent on a healthy attach — and **cleared** by one, so a session that
    /// downgraded once never keeps vouching for a failure it has since
    /// recovered from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_shell_unavailable: Option<DaemonShellUnavailable>,
}

pub fn run_attach(uuid: &str, cwd: Option<&str>, fallback: PlainShellFallback) -> Result<()> {
    let store = SessionStore::open_or_init()?;
    let session_dir = store.home_dir().join("runtime").join("attach").join(uuid);
    fs::create_dir_all(&session_dir)
        .with_context(|| format!("creating attach dir {}", session_dir.display()))?;

    let resolved_cwd = resolve_attach_cwd(cwd);
    if let Some(cwd) = resolved_cwd.as_deref() {
        std::env::set_current_dir(cwd).with_context(|| format!("setting attach cwd to {cwd}"))?;
    }

    let metadata_path = session_dir.join("session.json");
    let mut metadata = load_metadata(&metadata_path).unwrap_or_else(|| AttachMetadata {
        uuid: uuid.to_string(),
        backend: "daemon-shell".to_string(),
        shell: shell_program(),
        hostname: host_label(),
        cwd: std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .display()
            .to_string(),
        attach_count: 0,
        started_at: timestamp_now(),
        last_attached_at: timestamp_now(),
        daemon_shell_unavailable: None,
    });
    metadata.attach_count += 1;
    metadata.last_attached_at = timestamp_now();
    metadata.shell = shell_program();
    metadata.hostname = host_label();
    metadata.cwd = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .display()
        .to_string();
    // ⛔ NOT `daemon-shell` — nothing has been attached yet. Clearing any record
    // left by an earlier attach is the same honesty running the other way.
    metadata.backend = ATTACH_BACKEND_PENDING.to_string();
    metadata.daemon_shell_unavailable = None;
    write_metadata(&metadata_path, &metadata)?;

    // The host daemon owns/persists the shell PTY (it IS the multiplexer per
    // [[spec-decentralized-host-daemon]]). Bridge stdio to a daemon-owned,
    // resumable shell session — no external multiplexer (tmux/screen) anywhere.
    //
    // ⚠ TWO STAGES, AND ONLY THE FIRST IS ELIGIBLE FOR A SUBSTITUTE. Obtaining
    // the daemon-owned session can fail before anything exists, and a plain
    // shell is at least a defensible answer to that. A failure while BRIDGING
    // an already-created session is different in kind: that session exists, is
    // holding the user's work, and dropping into a fresh plain shell would
    // strand it while looking like a normal prompt. The old code ran both
    // stages under one `match` and substituted for either.
    let unavailable = match crate::ensure_daemon_shell_session_for_attach(
        uuid,
        resolved_cwd.as_deref(),
    ) {
        Ok((endpoint, key)) => {
            metadata.backend = ATTACH_BACKEND_DAEMON_SHELL.to_string();
            write_metadata(&metadata_path, &metadata)?;
            return crate::bridge_remote_runtime_session_stdio(&endpoint, &key);
        }
        Err(error) => format!("{error:#}"),
    };

    match record_daemon_shell_unavailable(&metadata_path, &mut metadata, fallback, &unavailable)? {
        AttachDowngradeDecision::SubstitutePlainShell => {
            announce_plain_shell_downgrade(&mut std::io::stdout(), &unavailable);
            exec_shell(resolved_cwd.as_deref())
        }
        AttachDowngradeDecision::Refuse => Err(anyhow::anyhow!(refusal_message(&unavailable))),
    }
}

/// Everything that must happen when the daemon-owned shell cannot be obtained,
/// EXCEPT exec'ing the shell — which never returns and therefore cannot be
/// exercised in-process. The seam exists so a test can feed this the very thing
/// it guards against (a daemon error) and read the outcome.
///
/// The record is written on BOTH branches. A refusal is as much a fact a client
/// needs as a substitution is, and `backend` tells them apart without a second
/// field that could disagree with it.
fn record_daemon_shell_unavailable(
    metadata_path: &Path,
    metadata: &mut AttachMetadata,
    fallback: PlainShellFallback,
    reason: &str,
) -> Result<AttachDowngradeDecision> {
    let decision = match fallback {
        PlainShellFallback::Allow => AttachDowngradeDecision::SubstitutePlainShell,
        PlainShellFallback::Refuse => AttachDowngradeDecision::Refuse,
    };
    metadata.backend = match decision {
        AttachDowngradeDecision::SubstitutePlainShell => ATTACH_BACKEND_PLAIN_SHELL,
        AttachDowngradeDecision::Refuse => ATTACH_BACKEND_NONE,
    }
    .to_string();
    metadata.daemon_shell_unavailable = Some(DaemonShellUnavailable {
        at: timestamp_now(),
        reason: reason.to_string(),
    });
    write_metadata(metadata_path, metadata)?;
    Ok(decision)
}

/// ⛔ STDOUT, deliberately. The old announcement went to stderr, which the phone
/// this protects never sees: `server attach` runs under SSH on a PTY and the
/// viewer reads the PTY stream. Writing it here also puts it directly above the
/// substitute shell's first prompt, where a human cannot miss it either.
fn announce_plain_shell_downgrade(out: &mut impl Write, reason: &str) {
    let _ = writeln!(out, "{}", plain_shell_downgrade_line(reason));
    let _ = out.flush();
}

fn plain_shell_downgrade_line(reason: &str) -> String {
    format!(
        "{PLAIN_SHELL_DOWNGRADE_MARKER} backend={ATTACH_BACKEND_PLAIN_SHELL} persistent=false \
         this shell dies when the terminal closes; the daemon-owned one was unavailable: {reason}"
    )
}

fn refusal_message(reason: &str) -> String {
    format!(
        "{PLAIN_SHELL_REFUSED_MARKER} the daemon-owned (persistent) shell was unavailable: \
         {reason}. Refusing to substitute a plain shell that would NOT survive the terminal \
         closing. Pass {PLAIN_SHELL_FALLBACK_FLAG} (or set {PLAIN_SHELL_FALLBACK_ENV}=1) to \
         accept a non-persistent shell."
    )
}

/// Resolve the fallback policy. The flag wins over the environment, and the
/// absence of both is [`PlainShellFallback::Refuse`] — silence must never be
/// read as consent to lose the persistence guarantee.
pub fn resolve_plain_shell_fallback(
    flag_present: bool,
    env_value: Option<&str>,
) -> PlainShellFallback {
    if flag_present {
        return PlainShellFallback::Allow;
    }
    match env_value.map(str::trim) {
        Some(value)
            if matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "allow"
            ) =>
        {
            PlainShellFallback::Allow
        }
        _ => PlainShellFallback::Refuse,
    }
}

/// Parse the argument tail of `server attach <uuid> …` into `(cwd, policy)`.
///
/// ⛔ The flag is filtered OUT before the positional scan. A naive `args.get(3)`
/// reads `--allow-plain-shell-fallback` as the cwd, and `resolve_attach_cwd`
/// then silently lands the shell in `$HOME` — the flag would break the very
/// thing it opts into.
pub fn parse_attach_args(tail: &[String]) -> (Option<String>, PlainShellFallback) {
    let flag_present = tail.iter().any(|arg| arg == PLAIN_SHELL_FALLBACK_FLAG);
    let cwd = tail
        .iter()
        .find(|arg| !arg.starts_with("--") && !arg.trim().is_empty())
        .cloned();
    let env_value = std::env::var(PLAIN_SHELL_FALLBACK_ENV).ok();
    (
        cwd,
        resolve_plain_shell_fallback(flag_present, env_value.as_deref()),
    )
}

fn write_metadata(path: &Path, metadata: &AttachMetadata) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(metadata)?)
        .with_context(|| format!("writing attach metadata {}", path.display()))
}

fn resolve_attach_cwd(cwd: Option<&str>) -> Option<String> {
    let requested = cwd.map(str::trim).filter(|value| !value.is_empty())?;
    let requested_path = PathBuf::from(requested);
    if requested_path.is_dir() {
        return Some(requested.to_string());
    }
    if let Some(existing_parent) = requested_path.ancestors().find(|path| path.is_dir()) {
        return Some(existing_parent.display().to_string());
    }
    std::env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| PathBuf::from(value).is_dir())
}

fn load_metadata(path: &PathBuf) -> Option<AttachMetadata> {
    let json = fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

fn timestamp_now() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

fn host_label() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn shell_program() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string())
}

fn exec_shell(cwd: Option<&str>) -> Result<()> {
    let shell = shell_program();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(&shell);
        for key in TERMINAL_ENV_REMOVALS {
            command.env_remove(key);
        }
        command.arg("-i");
        if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
            if uses_bash_shell(&shell) {
                command.env("YGGTERM_START_CWD", cwd);
                command.env(
                    "PROMPT_COMMAND",
                    r#"cd -- "$YGGTERM_START_CWD"; unset PROMPT_COMMAND"#,
                );
            }
        }
        let error = command.exec();
        Err(anyhow::anyhow!("failed to exec shell {shell}: {error}"))
    }

    #[cfg(not(unix))]
    {
        let mut command = Command::new(&shell);
        for key in TERMINAL_ENV_REMOVALS {
            command.env_remove(key);
        }
        command.arg("-i");
        if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
            if uses_bash_shell(&shell) {
                command.env("YGGTERM_START_CWD", cwd);
                command.env(
                    "PROMPT_COMMAND",
                    r#"cd -- "$YGGTERM_START_CWD"; unset PROMPT_COMMAND"#,
                );
            }
        }
        let status = command
            .status()
            .with_context(|| format!("running shell {shell}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("shell exited with status {status}"))
        }
    }
}

fn uses_bash_shell(shell: &str) -> bool {
    PathBuf::from(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("bash"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The daemon error the guard exists to catch. Feeding the guard the thing
    /// it guards against is the only way to falsify it.
    const DAEMON_ERROR: &str = "connecting to daemon socket: No such file or directory";

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yggterm-attach-downgrade-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn metadata_for_test() -> AttachMetadata {
        AttachMetadata {
            uuid: "0199fc00-0000-7000-8000-00000000abcd".to_string(),
            backend: ATTACH_BACKEND_PENDING.to_string(),
            shell: "/bin/bash".to_string(),
            hostname: "test-host".to_string(),
            cwd: "/tmp".to_string(),
            attach_count: 1,
            started_at: "1970-01-01T00:00:00Z".to_string(),
            last_attached_at: "1970-01-01T00:00:00Z".to_string(),
            daemon_shell_unavailable: None,
        }
    }

    /// ⛔ THE DEFAULT IS THE FIX. Before this change `run_attach` had no refusal
    /// branch at all: any daemon error substituted a non-persistent shell and
    /// said so once on stderr, which a phone never reads.
    #[test]
    fn a_caller_that_says_nothing_gets_a_refusal_not_a_silent_downgrade() {
        assert_eq!(PlainShellFallback::default(), PlainShellFallback::Refuse);
        assert_eq!(
            resolve_plain_shell_fallback(false, None),
            PlainShellFallback::Refuse
        );
        assert_eq!(
            resolve_plain_shell_fallback(false, Some("")),
            PlainShellFallback::Refuse,
            "an empty env value is not consent"
        );
        assert_eq!(
            resolve_plain_shell_fallback(false, Some("0")),
            PlainShellFallback::Refuse
        );
        assert_eq!(
            resolve_plain_shell_fallback(true, None),
            PlainShellFallback::Allow
        );
        for value in ["1", "true", "YES", " allow "] {
            assert_eq!(
                resolve_plain_shell_fallback(false, Some(value)),
                PlainShellFallback::Allow,
                "env value {value:?} should opt in"
            );
        }
    }

    /// The refusal must reach the caller as a FAILURE (so the exit status
    /// carries it) and must say, in machine-readable form, what was refused.
    #[test]
    fn a_refused_attach_records_that_nothing_is_attached() {
        let dir = scratch_dir("refuse");
        let path = dir.join("session.json");
        let mut metadata = metadata_for_test();

        let decision =
            record_daemon_shell_unavailable(&path, &mut metadata, PlainShellFallback::Refuse, DAEMON_ERROR)
                .expect("record the refusal");

        assert_eq!(decision, AttachDowngradeDecision::Refuse);
        // Read it back off disk: that is the client's actual read path, and a
        // struct in memory proves nothing about what a phone can see.
        let stored: AttachMetadata =
            serde_json::from_str(&fs::read_to_string(&path).expect("read metadata"))
                .expect("parse metadata");
        assert_eq!(
            stored.backend, ATTACH_BACKEND_NONE,
            "a refused attach must not claim a backend it does not have"
        );
        let unavailable = stored
            .daemon_shell_unavailable
            .expect("the refusal must be recorded, not only printed");
        assert_eq!(unavailable.reason, DAEMON_ERROR);

        let message = refusal_message(DAEMON_ERROR);
        assert!(message.starts_with(PLAIN_SHELL_REFUSED_MARKER), "{message}");
        assert!(message.contains(DAEMON_ERROR), "{message}");
        assert!(
            message.contains(PLAIN_SHELL_FALLBACK_FLAG),
            "the refusal must name its own override: {message}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// An ALLOWED downgrade is still not a silent one: it is recorded as
    /// non-persistent where a client can read it, and announced on stdout —
    /// the stream a phone streaming the PTY actually receives.
    #[test]
    fn an_allowed_downgrade_is_recorded_and_announced_as_non_persistent() {
        let dir = scratch_dir("allow");
        let path = dir.join("session.json");
        let mut metadata = metadata_for_test();

        let decision =
            record_daemon_shell_unavailable(&path, &mut metadata, PlainShellFallback::Allow, DAEMON_ERROR)
                .expect("record the downgrade");

        assert_eq!(decision, AttachDowngradeDecision::SubstitutePlainShell);
        let stored: AttachMetadata =
            serde_json::from_str(&fs::read_to_string(&path).expect("read metadata"))
                .expect("parse metadata");
        assert_eq!(
            stored.backend, ATTACH_BACKEND_PLAIN_SHELL,
            "the file must name the shell that is REALLY running, not the one asked for"
        );
        assert_eq!(
            stored
                .daemon_shell_unavailable
                .as_ref()
                .map(|record| record.reason.as_str()),
            Some(DAEMON_ERROR)
        );

        let mut announced = Vec::<u8>::new();
        announce_plain_shell_downgrade(&mut announced, DAEMON_ERROR);
        let announced = String::from_utf8(announced).expect("utf8 announcement");
        assert!(
            announced.starts_with(PLAIN_SHELL_DOWNGRADE_MARKER),
            "a client greps for the marker at the start of the line: {announced:?}"
        );
        assert!(
            announced.contains("persistent=false"),
            "the announcement must state the thing that was lost: {announced:?}"
        );
        assert!(announced.contains(DAEMON_ERROR), "{announced:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    /// The other direction of the same honesty: a session that downgraded once
    /// must not keep vouching for a failure it has recovered from.
    #[test]
    fn a_healthy_reattach_clears_a_stale_unavailable_record() {
        let dir = scratch_dir("clear");
        let path = dir.join("session.json");
        let mut metadata = metadata_for_test();
        record_daemon_shell_unavailable(&path, &mut metadata, PlainShellFallback::Allow, DAEMON_ERROR)
            .expect("record the downgrade");

        // What `run_attach` does at the top of a fresh attach.
        metadata.backend = ATTACH_BACKEND_PENDING.to_string();
        metadata.daemon_shell_unavailable = None;
        write_metadata(&path, &metadata).expect("rewrite metadata");
        // ...and what it does once the daemon-owned session is in hand.
        metadata.backend = ATTACH_BACKEND_DAEMON_SHELL.to_string();
        write_metadata(&path, &metadata).expect("rewrite metadata");

        let stored: AttachMetadata =
            serde_json::from_str(&fs::read_to_string(&path).expect("read metadata"))
                .expect("parse metadata");
        assert_eq!(stored.backend, ATTACH_BACKEND_DAEMON_SHELL);
        assert!(
            stored.daemon_shell_unavailable.is_none(),
            "a healthy attach must clear the old failure record"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// ⛔ The old parse was `args.get(3)`. Handed the opt-in flag it would read
    /// it as a cwd, `resolve_attach_cwd` would reject the non-directory and land
    /// the shell in `$HOME` — the flag silently moving the session.
    #[test]
    fn the_fallback_flag_is_never_read_as_a_cwd() {
        let flag = PLAIN_SHELL_FALLBACK_FLAG.to_string();
        let (cwd, fallback) = parse_attach_args(std::slice::from_ref(&flag));
        assert_eq!(cwd, None);
        assert_eq!(fallback, PlainShellFallback::Allow);

        let tail = vec![flag.clone(), "/tmp".to_string()];
        let (cwd, fallback) = parse_attach_args(&tail);
        assert_eq!(cwd.as_deref(), Some("/tmp"));
        assert_eq!(fallback, PlainShellFallback::Allow);
    }

    /// The `AttachMetadata` a pre-fix build wrote has no `daemon_shell_unavailable`
    /// key. It must still parse — a phone reading an older host's file gets
    /// "no failure recorded", not a parse error.
    #[test]
    fn metadata_written_by_an_older_build_still_parses() {
        let legacy = r#"{
            "uuid": "0199fc00-0000-7000-8000-00000000abcd",
            "backend": "daemon-shell",
            "shell": "/bin/bash",
            "hostname": "test-host",
            "cwd": "/tmp",
            "attach_count": 3,
            "started_at": "1970-01-01T00:00:00Z",
            "last_attached_at": "1970-01-01T00:00:00Z"
        }"#;
        let parsed: AttachMetadata = serde_json::from_str(legacy).expect("parse legacy metadata");
        assert!(parsed.daemon_shell_unavailable.is_none());
    }

    #[test]
    fn resolve_attach_cwd_reuses_existing_directory() {
        let cwd = std::env::temp_dir();
        let cwd = cwd.display().to_string();
        assert_eq!(resolve_attach_cwd(Some(&cwd)), Some(cwd));
    }

    #[test]
    fn resolve_attach_cwd_falls_back_to_existing_parent() {
        let root = std::env::temp_dir().join(format!(
            "yggterm-attach-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let existing = root.join("existing");
        fs::create_dir_all(&existing).expect("create existing parent");
        let missing = existing.join("missing").join("child");
        assert_eq!(
            resolve_attach_cwd(Some(&missing.display().to_string())),
            Some(existing.display().to_string())
        );
        let _ = fs::remove_dir_all(&root);
    }
}

