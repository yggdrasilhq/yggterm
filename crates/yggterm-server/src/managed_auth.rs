//! Managed-auth launch wiring — the daemon side of the ygg-auth `exec` seam.
//!
//! The auth-consumer lane (ygg4/auth-consumer, f075de21) ships
//! `.agents/skills/yggterm-agent-fleet/ygg-auth.py exec`: a preflight receipt
//! (`--preflight-only --json`, read-only, stdout) and a same-pid exec
//! replacement (`exec --harness <H> --json -- <native argv…>`, receipt one
//! line on stderr, then the native CLI owns the terminal). This module is the
//! daemon wiring for that seam and nothing more:
//!
//! - **Off (the default)** composes byte-identically to the pre-wiring law —
//!   a host that never heard of managed auth must launch exactly as before.
//! - **Observe** runs the bounded preflight, emits one `auth_preflight` trace
//!   event, and launches normally. The receipt records; it never gates.
//! - **Enforce** refuses the launch unless the preflight says `ready`. The
//!   refusal is a VISIBLE shell refusal line (the reason on stderr, exit 86),
//!   not a composer `Err` — the composer's error channel is the
//!   layout-resolution fallback, and every call site degrades an `Err` to
//!   `legacy_agent_launch_command`. An `Err` refusal would therefore
//!   downgrade to exactly the unauthenticated launch enforce exists to
//!   prevent; a composed refusal line launches nothing and shows why.
//!
//! ⛔ **No policy activation lives here.** The mode is read from the settings
//! store (`managed_auth_mode`), whose default is `off`; nothing in this module
//! enables enforcement, touches a live account, or runs a refresh. The
//! preflight is the seam's own read-only verb — the ONE optional network touch
//! (`--measure`) is deliberately never passed.
//!
//! The harness mapping follows the auth-consumer capability matrix: codex
//! (and its litellm twin), claude, and zcode have an auth plane; the other
//! registered CLIs have none and are never gated, in any mode.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::managed_cli::ManagedCliTool;

/// Settings key carrying the mode string ("off" | "observe" | "enforce").
pub const MANAGED_AUTH_MODE_SETTINGS_KEY: &str = "managed_auth_mode";
/// Settings key carrying an explicit ygg-auth.py path (empty ⇒ canonical
/// dispersed location). Config-explicit wins; a configured-but-missing path is
/// a refusal in Enforce, never a fallback guess.
pub const MANAGED_AUTH_SCRIPT_SETTINGS_KEY: &str = "managed_auth_script";
/// The dispersed canonical location `ygg-disperse` maintains.
pub const DISPERSED_YGG_AUTH_PATH: &str = ".yggterm/bin/ygg/ygg-auth.py";
/// The preflight is a local receipt read (plus the seam's own store reads);
/// anything slower than this is a wedged helper, and the launch decision may
/// not wait on a wedged helper.
pub const PREFLIGHT_TIMEOUT: Duration = Duration::from_millis(2000);

/// How the daemon treats the auth plane for a managed-CLI launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedAuthMode {
    /// Byte-identical composition. The default and the activation gate.
    Off,
    /// Preflight runs, the receipt is traced, the launch is untouched.
    Observe,
    /// Preflight must say `ready` or the launch is refused with the reason.
    Enforce,
}

impl ManagedAuthMode {
    /// Unreadable or unknown settings values are Off — the gate fails closed
    /// toward the pre-wiring behavior, never toward enforcement.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "observe" => ManagedAuthMode::Observe,
            "enforce" => ManagedAuthMode::Enforce,
            _ => ManagedAuthMode::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ManagedAuthMode::Off => "off",
            ManagedAuthMode::Observe => "observe",
            ManagedAuthMode::Enforce => "enforce",
        }
    }
}

/// The auth-plane harness name a managed CLI launches under, per the
/// auth-consumer capability matrix. `None` ⇒ no auth plane: never gated,
/// never prefixed, in any mode.
pub fn auth_harness_for(tool: ManagedCliTool) -> Option<&'static str> {
    match tool {
        ManagedCliTool::Codex | ManagedCliTool::CodexLiteLlm => Some("codex"),
        ManagedCliTool::ClaudeCode => Some("claude"),
        ManagedCliTool::ZcodeTui => Some("zcode"),
        _ => None,
    }
}

/// Resolve the ygg-auth.py script: an explicit settings path first, then the
/// dispersed canonical path. Absent means absent — the caller refuses in
/// Enforce and reports unavailable in Observe. No third location is guessed,
/// because a guessed script is an untrusted program run before every launch.
pub fn resolve_ygg_auth_script(explicit: &str) -> Option<PathBuf> {
    let trimmed = explicit.trim();
    if !trimmed.is_empty() {
        let path = PathBuf::from(trimmed);
        return path.is_file().then_some(path);
    }
    let dispersed = dirs::home_dir()?.join(DISPERSED_YGG_AUTH_PATH);
    dispersed.is_file().then_some(dispersed)
}

/// `<python3> <script> exec --harness <H> --preflight-only --json` — the
/// read-only receipt. `--measure` is never added here: the one optional
/// network touch is a meter's decision, not a launch gate's.
pub fn preflight_argv(script: &Path, harness: &str) -> Vec<String> {
    vec![
        "python3".to_string(),
        script.display().to_string(),
        "exec".to_string(),
        "--harness".to_string(),
        harness.to_string(),
        "--preflight-only".to_string(),
        "--json".to_string(),
    ]
}

/// Wrap a composed shell invocation in the exec seam. Persistent rows compose
/// a leading `exec ` on the INNER command; the outer exec already replaces
/// the shell, so the inner one is stripped to avoid `exec exec python3 …`.
/// The script path is single-quoted; the inner invocation is already a
/// composed shell string (its tokens were quoted at composition).
pub fn wrap_invocation(script: &Path, harness: &str, inner: &str) -> String {
    let stripped = inner.strip_prefix("exec ").unwrap_or(inner);
    format!(
        "exec python3 {} exec --harness {} --json -- {}",
        crate::shell_single_quote(&script.display().to_string()),
        harness,
        stripped
    )
}

/// The receipt the exec seam prints. Parsing is deliberately lenient — the
/// daemon reads `outcome`, the error triple, and the grant's record hash, and
/// leaves every other field to the auth plane's own contract.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuthPreflightReceipt {
    pub outcome: String,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub error: Option<AuthPreflightError>,
    #[serde(default)]
    pub grant_binding: Option<AuthGrantBinding>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuthPreflightError {
    #[serde(default)]
    pub code: Option<i64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuthGrantBinding {
    #[serde(default)]
    pub proven: Option<bool>,
    #[serde(default)]
    pub record_hash: Option<String>,
}

impl AuthPreflightReceipt {
    pub fn is_ready(&self) -> bool {
        self.outcome == "ready"
    }

    /// The refusal reason an Enforce launch surfaces: the seam's own reason
    /// when it has one, the code when that is all it has.
    pub fn refusal_reason(&self) -> String {
        if let Some(error) = &self.error {
            if let Some(reason) = &error.reason {
                return reason.clone();
            }
            if let Some(code) = error.code {
                return format!("auth error {code}");
            }
        }
        format!("outcome {}", self.outcome)
    }
}

/// Parse the preflight receipt from stdout. The seam prints ONE line of JSON;
/// a chatty PYTHONPATH printer ahead of it must not turn into a parse
/// failure, so the first line that opens with `{` is the candidate.
pub fn parse_receipt(stdout: &str) -> Option<AuthPreflightReceipt> {
    let trimmed = stdout.trim();
    if let Ok(receipt) = serde_json::from_str(trimmed) {
        return Some(receipt);
    }
    stdout
        .lines()
        .find_map(|line| serde_json::from_str(line.trim()).ok())
}

/// What the preflight probe established. `Unavailable` carries the reason the
/// daemon could not obtain a receipt at all (spawn failure, timeout, parse
/// failure) — distinct from a receipt that says `error`, because "the gate
/// could not run" and "the gate refused" are different facts a launch refusal
/// must not conflate.
#[derive(Debug, Clone, PartialEq)]
pub enum PreflightProbe {
    Ready(AuthPreflightReceipt),
    Blocked(AuthPreflightReceipt),
    Unavailable(String),
}

/// Run the bounded preflight. Never panics on a wedged helper: the deadline
/// abandons it, and the abandoned child is left to the reaper — killing a
/// process from the launch path is not this module's call.
pub fn run_preflight(script: &Path, harness: &str) -> PreflightProbe {
    let argv = preflight_argv(script, harness);
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return PreflightProbe::Unavailable(format!("spawn failed: {error}")),
    };
    let deadline = Instant::now() + PREFLIGHT_TIMEOUT;
    let mut stdout = Vec::new();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    return PreflightProbe::Unavailable("preflight timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return PreflightProbe::Unavailable(format!("wait failed: {error}")),
        }
    }
    if let Some(mut pipe) = child.stdout.take() {
        use std::io::Read;
        let _ = pipe.read_to_end(&mut stdout);
    }
    match parse_receipt(&String::from_utf8_lossy(&stdout)) {
        Some(receipt) if receipt.is_ready() => PreflightProbe::Ready(receipt),
        Some(receipt) => PreflightProbe::Blocked(receipt),
        None => PreflightProbe::Unavailable("preflight printed no receipt JSON".to_string()),
    }
}

/// The launch decision, as data — so the gating rule is unit-testable without
/// spawning anything, and the composer applies whatever the rule returned.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthLaunchPlan {
    /// Compose as before (Off, or a CLI with no auth plane).
    PassThrough,
    /// Prefix the composed invocation with the exec seam.
    Wrap {
        script: PathBuf,
        harness: &'static str,
    },
    /// Compose a visible refusal line instead of the launch: the reason lands
    /// on stderr and the shell exits 86. Never a composer `Err` — see the
    /// module doc for why an `Err` refusal would silently downgrade.
    Refuse { reason: String },
}

/// The composed refusal line. `86` is deliberately not a CLI exit code a
/// caller would mistake for the CLI's own verdict — the row shows the reason
/// and stops.
pub fn refusal_invocation(reason: &str) -> String {
    format!(
        "printf '%s\\n' {} >&2; exit 86",
        crate::shell_single_quote(&format!(
            "yggterm managed-auth refused this launch: {reason}"
        ))
    )
}

/// The gating rule. Every branch is decided here so the composer carries no
/// auth logic of its own.
pub fn plan_launch(
    mode: ManagedAuthMode,
    tool: ManagedCliTool,
    probe: Option<&PreflightProbe>,
    script: Option<&Path>,
) -> AuthLaunchPlan {
    let Some(harness) = auth_harness_for(tool) else {
        return AuthLaunchPlan::PassThrough;
    };
    match mode {
        ManagedAuthMode::Off => AuthLaunchPlan::PassThrough,
        ManagedAuthMode::Observe => AuthLaunchPlan::PassThrough,
        ManagedAuthMode::Enforce => {
            let Some(script) = script else {
                return AuthLaunchPlan::Refuse {
                    reason: "managed-auth enforce: ygg-auth.py script unavailable".to_string(),
                };
            };
            match probe {
                Some(PreflightProbe::Ready(_receipt)) => AuthLaunchPlan::Wrap {
                    script: script.to_path_buf(),
                    harness,
                },
                Some(PreflightProbe::Blocked(receipt)) => AuthLaunchPlan::Refuse {
                    reason: format!(
                        "managed-auth enforce: preflight refused the launch ({})",
                        receipt.refusal_reason()
                    ),
                },
                Some(PreflightProbe::Unavailable(reason)) => AuthLaunchPlan::Refuse {
                    reason: format!("managed-auth enforce: preflight unavailable ({reason})"),
                },
                None => AuthLaunchPlan::Refuse {
                    reason: "managed-auth enforce: no preflight was run".to_string(),
                },
            }
        }
    }
}

/// One trace event per preflight, on the CLI plane: which harness, which
/// mode, and what the receipt said (digest-grade facts only — the receipt
/// never carries credentials, and this payload must not start).
pub fn emit_preflight_event(
    kind: yggterm_core::SessionKind,
    mode: ManagedAuthMode,
    probe: &PreflightProbe,
) {
    let payload = match probe {
        PreflightProbe::Ready(receipt) => serde_json::json!({
            "mode": mode.as_str(),
            "outcome": "ready",
            "harness": receipt.harness,
            "record_hash": receipt.grant_binding.as_ref().and_then(|grant| grant.record_hash.clone()),
        }),
        PreflightProbe::Blocked(receipt) => serde_json::json!({
            "mode": mode.as_str(),
            "outcome": "blocked",
            "reason": receipt.refusal_reason(),
        }),
        PreflightProbe::Unavailable(reason) => serde_json::json!({
            "mode": mode.as_str(),
            "outcome": "unavailable",
            "reason": reason,
        }),
    };
    yggterm_core::perf::ytrace_emit_event(
        "daemon",
        yggterm_core::cli_plane::CLI_PLANE_CATEGORY,
        "auth_preflight",
        payload,
    );
}

/// The configured mode, off the ONE settings store. A store that cannot be
/// opened is Off — a settings-read failure must never turn into enforcement.
pub fn configured_mode() -> ManagedAuthMode {
    crate::settings_string_field(MANAGED_AUTH_MODE_SETTINGS_KEY)
        .map(|raw| ManagedAuthMode::parse(&raw))
        .unwrap_or(ManagedAuthMode::Off)
}

/// The configured script path (explicit setting, else the dispersed canonical
/// location), resolved to existence.
pub fn configured_script() -> Option<PathBuf> {
    let explicit =
        crate::settings_string_field(MANAGED_AUTH_SCRIPT_SETTINGS_KEY).unwrap_or_default();
    resolve_ygg_auth_script(&explicit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_or_empty_mode_string_is_off() {
        assert_eq!(ManagedAuthMode::parse(""), ManagedAuthMode::Off);
        assert_eq!(ManagedAuthMode::parse("garbage"), ManagedAuthMode::Off);
        assert_eq!(ManagedAuthMode::parse("Enforce "), ManagedAuthMode::Enforce);
        assert_eq!(ManagedAuthMode::parse("OBSERVE"), ManagedAuthMode::Observe);
    }

    #[test]
    fn only_the_capability_matrix_clis_have_an_auth_harness() {
        assert_eq!(auth_harness_for(ManagedCliTool::Codex), Some("codex"));
        assert_eq!(
            auth_harness_for(ManagedCliTool::CodexLiteLlm),
            Some("codex")
        );
        assert_eq!(auth_harness_for(ManagedCliTool::ClaudeCode), Some("claude"));
        assert_eq!(auth_harness_for(ManagedCliTool::ZcodeTui), Some("zcode"));
        assert_eq!(auth_harness_for(ManagedCliTool::Pi), None);
        assert_eq!(auth_harness_for(ManagedCliTool::Muse), None);
        assert_eq!(auth_harness_for(ManagedCliTool::Mimo), None);
    }

    #[test]
    fn preflight_argv_is_the_read_only_receipt_call() {
        let argv = preflight_argv(Path::new("/s/ygg-auth.py"), "codex");
        assert_eq!(argv[0], "python3");
        assert!(argv.contains(&"--preflight-only".to_string()));
        assert!(argv.contains(&"--json".to_string()));
        assert!(!argv.contains(&"--measure".to_string()));
    }

    #[test]
    fn wrap_strips_the_inner_exec_and_quotes_the_script() {
        let wrapped = wrap_invocation(
            Path::new("/opt/my auth/ygg-auth.py"),
            "codex",
            "codex --model x",
        );
        assert!(wrapped.starts_with(
            "exec python3 '/opt/my auth/ygg-auth.py' exec --harness codex --json -- "
        ));
        assert!(wrapped.ends_with("codex --model x"));
        let persistent = wrap_invocation(
            Path::new("/s/ygg-auth.py"),
            "claude",
            "exec claude --resume abc",
        );
        assert!(!persistent.contains("exec exec"));
        assert!(persistent.ends_with("claude --resume abc"));
    }

    #[test]
    fn the_refusal_line_quotes_arbitrary_reasons_and_exits_86() {
        let line = refusal_invocation("grant_unproven; rm -rf $HOME");
        assert!(line.ends_with("; exit 86"));
        assert!(line.contains("grant_unproven; rm -rf $HOME"));
        assert!(
            line.starts_with("printf '%s\\n' '"),
            "the reason must be single-quoted, never interpolated"
        );
        // A reason containing a single quote must not break out of the quoting.
        let quoted = refusal_invocation("it's blocked");
        assert!(
            quoted.contains("'\\''"),
            "embedded single quotes escape as '\\''"
        );
    }

    #[test]
    fn a_receipt_parses_whether_or_not_chatter_precedes_it() {
        let ready: &str = r#"{"schema_version": 1, "outcome": "ready", "harness": "codex", "grant_binding": {"proven": true, "record_hash": "abc123"}}"#;
        let receipt = parse_receipt(ready).expect("ready receipt parses");
        assert!(receipt.is_ready());
        assert_eq!(
            receipt.grant_binding.unwrap().record_hash.as_deref(),
            Some("abc123")
        );

        let chatty = "PYTHONPATH warning\n".to_owned() + ready;
        assert!(parse_receipt(&chatty).is_some());

        let blocked: &str = r#"{"outcome": "error", "error": {"code": 5, "reason": "grant_unproven", "message": "capture first"}}"#;
        let receipt = parse_receipt(blocked).expect("blocked receipt parses");
        assert!(!receipt.is_ready());
        assert_eq!(receipt.refusal_reason(), "grant_unproven");

        assert!(parse_receipt("not json at all").is_none());
    }

    #[test]
    fn enforce_refuses_unavailable_and_blocked_but_never_conflates_them() {
        let ready = PreflightProbe::Ready(AuthPreflightReceipt {
            outcome: "ready".to_string(),
            harness: Some("codex".to_string()),
            error: None,
            grant_binding: None,
        });
        let blocked = PreflightProbe::Blocked(AuthPreflightReceipt {
            outcome: "error".to_string(),
            harness: None,
            error: Some(AuthPreflightError {
                code: Some(5),
                reason: Some("grant_unproven".to_string()),
                message: None,
            }),
            grant_binding: None,
        });
        let unavailable = PreflightProbe::Unavailable("preflight timed out".to_string());

        let script = Path::new("/s/ygg-auth.py");
        match plan_launch(
            ManagedAuthMode::Enforce,
            ManagedCliTool::Codex,
            Some(&ready),
            Some(script),
        ) {
            AuthLaunchPlan::Wrap { harness, .. } => assert_eq!(harness, "codex"),
            other => panic!("ready must wrap, got {other:?}"),
        }
        match plan_launch(
            ManagedAuthMode::Enforce,
            ManagedCliTool::Codex,
            Some(&blocked),
            Some(script),
        ) {
            AuthLaunchPlan::Refuse { reason } => assert!(reason.contains("grant_unproven")),
            other => panic!("blocked must refuse with the receipt's reason, got {other:?}"),
        }
        match plan_launch(
            ManagedAuthMode::Enforce,
            ManagedCliTool::Codex,
            Some(&unavailable),
            Some(script),
        ) {
            AuthLaunchPlan::Refuse { reason } => assert!(reason.contains("timed out")),
            other => panic!("unavailable must refuse with its own reason, got {other:?}"),
        }
        match plan_launch(
            ManagedAuthMode::Enforce,
            ManagedCliTool::Codex,
            Some(&ready),
            None,
        ) {
            AuthLaunchPlan::Refuse { reason } => assert!(reason.contains("unavailable")),
            other => panic!("missing script must refuse, got {other:?}"),
        }
        // A preflight that never ran is a refusal, not a pass.
        match plan_launch(
            ManagedAuthMode::Enforce,
            ManagedCliTool::Codex,
            None,
            Some(script),
        ) {
            AuthLaunchPlan::Refuse { reason } => assert!(reason.contains("no preflight")),
            other => panic!("missing probe must refuse, got {other:?}"),
        }
    }

    #[test]
    fn observe_and_off_pass_through_even_when_the_preflight_blocked() {
        let blocked = PreflightProbe::Blocked(AuthPreflightReceipt {
            outcome: "error".to_string(),
            harness: None,
            error: None,
            grant_binding: None,
        });
        assert_eq!(
            plan_launch(
                ManagedAuthMode::Observe,
                ManagedCliTool::Codex,
                Some(&blocked),
                None
            ),
            AuthLaunchPlan::PassThrough
        );
        assert_eq!(
            plan_launch(
                ManagedAuthMode::Off,
                ManagedCliTool::Codex,
                Some(&blocked),
                None
            ),
            AuthLaunchPlan::PassThrough
        );
    }

    #[test]
    fn a_cli_without_an_auth_plane_is_never_gated_in_any_mode() {
        for mode in [
            ManagedAuthMode::Off,
            ManagedAuthMode::Observe,
            ManagedAuthMode::Enforce,
        ] {
            assert_eq!(
                plan_launch(mode, ManagedCliTool::Muse, None, None),
                AuthLaunchPlan::PassThrough,
                "{:?} must not gate a CLI with no auth plane",
                mode
            );
        }
    }

    #[test]
    fn an_explicit_script_path_that_does_not_exist_resolves_to_none() {
        // ⛔ Not /tmp — the scratchpad is the repo's sanctioned scratch root.
        let missing = scratch_fixture_path("no-such-script.py");
        assert_eq!(
            resolve_ygg_auth_script(&missing.display().to_string()),
            None
        );
    }

    #[test]
    fn an_existing_explicit_path_wins_over_the_dispersed_default() {
        let existing = scratch_fixture_path("real-script.py");
        std::fs::write(&existing, "#!/bin/sh\nexit 0\n").expect("fixture script writes");
        let resolved = resolve_ygg_auth_script(&existing.display().to_string());
        std::fs::remove_file(&existing).ok();
        assert_eq!(resolved, Some(existing));
    }

    fn scratch_fixture_path(name: &str) -> PathBuf {
        let dir = dirs::home_dir()
            .expect("home dir resolves in test env")
            .join(".yggterm/scratchpad/ygg4-managed-auth-tests");
        std::fs::create_dir_all(&dir).expect("scratch fixture dir creates");
        dir.join(format!("{}-{name}", std::process::id()))
    }
}
