//! The ARM MATRIX — every agent CLI × every locality, in one table
//! (`docs/spec-agent-cli-harness.md` §6 A3, §8 phase 2).
//!
//! **The problem this exists for.** The harness's per-arm decisions —
//! which scheme names the row, which key names the runtime, what the resume
//! command is, where a keystroke goes when the local runtime is gone — are made
//! in four different modules. A change that "obviously" applies to every arm
//! has repeatedly been applied to one: `terminal_write_strategy_for_path`
//! matched `remote-session://` only, so a `remote-cc://` session aimed its
//! keystrokes at a runtime that did not exist (live on jojo 2026-07-23,
//! `docs/pending-bugs.md`), and §7.2 lists eleven more of the same shape. Those
//! holes are invisible in a per-function test, because a per-function test asks
//! only about the arm its author had in mind.
//!
//! So this module asks about ALL of them at once: one row per arm, every row
//! answered by the same accessors. A phase that changes one arm's answer
//! changes exactly one visible cell — and the cells that MUST agree across
//! arms (the store layout, the invocation, the write strategy's fork on
//! locality alone) are asserted against the arm's twin rather than against a
//! constant, so "unified" is checked instead of transcribed.
//!
//! **The spec calls it the "four-arm matrix" and that undercounts what ships:**
//! `AGENT_CLIS` carries THREE CLIs (codex, codex-litellm, claude-code), so the
//! matrix is six arms. That is not a defect in the spec so much as evidence for
//! why the enumeration must be derived — [`every_registered_cli_has_both_arms`]
//! fails if a CLI is registered without arms, which is the A6 new-CLI drill's
//! first gate.
//!
//! **Recorded forks** (§7 divergences that are still real) live in
//! [`RECORDED_ARM_FORKS`] and are locked in BOTH directions, the same contract
//! as phase 0's `KNOWN_PREDICATE_HOLES`: a fork that stops reproducing fails the
//! test until its row is deleted, so the table can never go stale-green.

use yggterm_core::SessionKind;
use yggterm_core::agent_cli::{AGENT_CLIS, agent_cli_descriptor};

use crate::daemon::{TerminalWriteStrategy, terminal_write_strategy_for_path};
use crate::{
    agent_launch_command, local_live_runtime_key, persistent_agent_resume_command,
    remote_agent_resume_subcommand, remote_agent_start_subcommand, remote_cc_session_path,
    remote_runtime_agent_session_key, remote_scanned_session_path,
};

/// Where a session's PTY lives relative to the machine holding the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Locality {
    Local,
    Remote,
}

/// One arm of the matrix: a CLI on a locality.
struct Arm {
    kind: SessionKind,
    locality: Locality,
    /// The scheme that names a ROW on this arm.
    row_scheme: &'static str,
    /// The scheme that names the RUNTIME on this arm (`None` where the row
    /// scheme is also the runtime key — `local://` is both, which the scheme
    /// registry models rather than papers over).
    runtime_scheme: Option<&'static str>,
    /// The wrapper subcommand the remote transport invokes (`None` for local,
    /// which spawns directly — the one transport seam, spec §2 corollary 2).
    remote_resume_subcommand: Option<&'static str>,
    remote_start_subcommand: Option<&'static str>,
    /// Where a keystroke goes when this arm's local runtime is NOT running.
    /// The cell that was wrong for remote-cc and cost the user a live session.
    write_strategy_without_local_runtime: TerminalWriteStrategy,
    /// The executable this arm invokes, per the registry.
    binary: &'static str,
    /// The token that names an existing session on this arm's resume line.
    resume_selector_token: &'static str,
    /// Whether the resume line re-roots the CLI with `-C "$PWD"`.
    re_roots_with_cwd: bool,
    /// The CLI's own store, as the scanner globs it.
    store_globs: &'static [&'static str],
}

const ARM_SESSION_ID: &str = "11111111-2222-3333-4444-555555555555";
const ARM_CWD: &str = "/home/user/gh/yggterm";
const ARM_MACHINE: &str = "dev";

const ARMS: &[Arm] = &[
    Arm {
        kind: SessionKind::Codex,
        locality: Locality::Local,
        row_scheme: "local://",
        runtime_scheme: None,
        remote_resume_subcommand: None,
        remote_start_subcommand: None,
        write_strategy_without_local_runtime: TerminalWriteStrategy::LocalRuntimeFallback,
        binary: "codex",
        resume_selector_token: "resume",
        re_roots_with_cwd: true,
        store_globs: &[".codex/sessions/**/rollout-*.jsonl"],
    },
    Arm {
        kind: SessionKind::Codex,
        locality: Locality::Remote,
        row_scheme: "remote-session://",
        runtime_scheme: Some("codex-runtime://"),
        remote_resume_subcommand: Some("resume-codex"),
        remote_start_subcommand: Some("start-codex"),
        write_strategy_without_local_runtime: TerminalWriteStrategy::RemoteDirectFallback,
        binary: "codex",
        resume_selector_token: "resume",
        re_roots_with_cwd: true,
        store_globs: &[".codex/sessions/**/rollout-*.jsonl"],
    },
    Arm {
        kind: SessionKind::CodexLiteLlm,
        locality: Locality::Local,
        row_scheme: "local://",
        runtime_scheme: None,
        remote_resume_subcommand: None,
        remote_start_subcommand: None,
        write_strategy_without_local_runtime: TerminalWriteStrategy::LocalRuntimeFallback,
        binary: "codex-litellm",
        resume_selector_token: "resume",
        // ⚠ RECORDED FORK: codex re-roots here, codex-litellm does not.
        // See RECORDED_ARM_FORKS.
        re_roots_with_cwd: false,
        store_globs: &[".codex-litellm/sessions/**/rollout-*.jsonl"],
    },
    Arm {
        kind: SessionKind::CodexLiteLlm,
        locality: Locality::Remote,
        row_scheme: "remote-session://",
        runtime_scheme: Some("codex-runtime://"),
        remote_resume_subcommand: Some("resume-codex"),
        remote_start_subcommand: Some("start-codex"),
        write_strategy_without_local_runtime: TerminalWriteStrategy::RemoteDirectFallback,
        binary: "codex-litellm",
        resume_selector_token: "resume",
        re_roots_with_cwd: false,
        store_globs: &[".codex-litellm/sessions/**/rollout-*.jsonl"],
    },
    Arm {
        kind: SessionKind::ClaudeCode,
        locality: Locality::Local,
        row_scheme: "local://",
        runtime_scheme: None,
        remote_resume_subcommand: None,
        remote_start_subcommand: None,
        write_strategy_without_local_runtime: TerminalWriteStrategy::LocalRuntimeFallback,
        binary: "claude",
        resume_selector_token: "--resume",
        re_roots_with_cwd: false,
        store_globs: &[".claude/projects/*/*.jsonl"],
    },
    Arm {
        kind: SessionKind::ClaudeCode,
        locality: Locality::Remote,
        row_scheme: "remote-cc://",
        runtime_scheme: Some("cc-runtime://"),
        remote_resume_subcommand: Some("resume-cc"),
        remote_start_subcommand: Some("start-cc"),
        write_strategy_without_local_runtime: TerminalWriteStrategy::RemoteDirectFallback,
        binary: "claude",
        resume_selector_token: "--resume",
        re_roots_with_cwd: false,
        store_globs: &[".claude/projects/*/*.jsonl"],
    },
];

/// A §7 divergence that is STILL REAL, recorded so it is a decision rather than
/// an accident — and locked in both directions so the row cannot outlive it.
struct RecordedArmFork {
    /// What forks, in the vocabulary of the spec's inventory.
    concern: &'static str,
    /// When it was recorded / last re-verified against main.
    recorded: &'static str,
    /// Why it has not been unified yet. A fork with no reason is a bug.
    reason: &'static str,
}

const RECORDED_ARM_FORKS: &[RecordedArmFork] = &[
    RecordedArmFork {
        concern: "resume_re_roots_with_cwd: Codex re-roots with -C \"$PWD\", CodexLiteLlm does not",
        recorded: "2026-07-25",
        reason: "inherited from the pre-descriptor builder, which gated re-rooting on \
                 SessionKind::Codex alone. Whether that was intent or oversight is still \
                 unverified — settling it needs a live codex-litellm session resumed from a \
                 DIFFERENT cwd than it was born in, which nothing in the fleet runs today.",
    },
    RecordedArmFork {
        concern: "codex home env var forks by locality: YGGTERM_CODEX_HOME locally, \
                  CODEX_HOME on the remote scan path",
        recorded: "2026-07-25",
        reason: "the remote scan runs INSIDE the CLI's own environment, where CODEX_HOME is \
                 the CLI's variable and yggterm's would be meaningless; locally yggterm owns \
                 the override so it must not collide with the user's own CODEX_HOME. Unifying \
                 changes which sessions a host finds, so it belongs to a phase that can prove \
                 the change live.",
    },
];

impl Arm {
    fn name(&self) -> String {
        format!("{:?}/{:?}", self.kind, self.locality)
    }

    /// The row path this arm produces for the fixture session.
    fn row_path(&self) -> String {
        match (self.kind, self.locality) {
            (_, Locality::Local) => local_live_runtime_key(ARM_SESSION_ID),
            (SessionKind::ClaudeCode, Locality::Remote) => {
                remote_cc_session_path(ARM_MACHINE, ARM_SESSION_ID)
            }
            (_, Locality::Remote) => remote_scanned_session_path(ARM_MACHINE, ARM_SESSION_ID),
        }
    }
}

#[test]
fn every_registered_cli_has_both_arms() {
    for descriptor in AGENT_CLIS {
        for locality in [Locality::Local, Locality::Remote] {
            let matches = ARMS
                .iter()
                .filter(|arm| arm.kind == descriptor.kind && arm.locality == locality)
                .count();
            assert_eq!(
                matches, 1,
                "{:?} on {locality:?} must appear EXACTLY once in the arm matrix — a CLI \
                 registered without arms is a CLI whose harness decisions nothing checks \
                 (spec §6 A6, the new-CLI drill)",
                descriptor.kind,
            );
        }
    }
    assert_eq!(
        ARMS.len(),
        AGENT_CLIS.len() * 2,
        "the matrix must be exactly the registry crossed with locality — an extra arm means \
         an arm for a CLI that no longer ships",
    );
}

#[test]
fn every_arm_names_its_row_and_runtime_with_the_registered_scheme() {
    for arm in ARMS {
        let row_path = arm.row_path();
        assert!(
            row_path.starts_with(arm.row_scheme),
            "{}: row path {row_path} does not use its declared scheme {}",
            arm.name(),
            arm.row_scheme,
        );
        match (arm.locality, arm.runtime_scheme) {
            (Locality::Remote, Some(scheme)) => {
                let key = remote_runtime_agent_session_key(arm.kind, ARM_SESSION_ID)
                    .unwrap_or_else(|| {
                        panic!("{}: agent kind must have a runtime key", arm.name())
                    });
                assert!(
                    key.starts_with(scheme),
                    "{}: runtime key {key} does not use its declared scheme {scheme}",
                    arm.name(),
                );
            }
            (Locality::Local, None) => {
                // `local://` is BOTH row identity and runtime key for a local
                // agent — one string, two roles (spec §7.1).
                assert_eq!(row_path, local_live_runtime_key(ARM_SESSION_ID));
            }
            (locality, scheme) => panic!(
                "{}: {locality:?} with runtime scheme {scheme:?} is not a shape the matrix models",
                arm.name(),
            ),
        }
    }
}

/// The tail of a built command: everything after the environment preamble,
/// i.e. the CLI invocation itself. The preamble carries the cwd walk and the
/// terminal-appearance exports, both of which legitimately vary with the
/// build (`TERM_PROGRAM_VERSION` is the crate version) and with ambient env
/// (`YGGTERM_CC_EXTRA_ARGS`) — a byte-for-byte lock on the whole string would
/// fail on every version bump, which is a lock nobody keeps.
fn invocation_tail(command: &str) -> &str {
    let tail = command
        .rsplit_once("&& ")
        .map(|(_, tail)| tail)
        .unwrap_or(command)
        .trim();
    tail.strip_prefix("exec ").unwrap_or(tail)
}

/// Whether `tokens` appear in `text` in that order.
fn contains_in_order(text: &str, tokens: &[&str]) -> bool {
    let mut cursor = 0usize;
    for token in tokens {
        match text[cursor..].find(token) {
            Some(at) => cursor += at + token.len(),
            None => return false,
        }
    }
    true
}

#[test]
fn every_arm_builds_the_invocation_its_descriptor_declares() {
    let quoted_id = format!("'{ARM_SESSION_ID}'");
    for arm in ARMS {
        let descriptor = agent_cli_descriptor(arm.kind).expect("registered CLI");
        assert_eq!(
            descriptor.binary_name,
            arm.binary,
            "{}: the arm and the registry disagree about the executable",
            arm.name(),
        );

        let resume = persistent_agent_resume_command(arm.kind, Some(ARM_CWD), ARM_SESSION_ID);
        let tail = invocation_tail(&resume);
        assert!(
            contains_in_order(tail, &[arm.binary, arm.resume_selector_token, &quoted_id]),
            "{}: resume invocation must name the binary, then the selector, then the quoted \
             session id — got {tail}",
            arm.name(),
        );
        assert_eq!(
            tail.contains("-C \"$PWD\""),
            arm.re_roots_with_cwd,
            "{}: re-rooting drifted from the matrix. If this is intended, the RECORDED_ARM_FORKS \
             row for re-rooting must change in the same commit — got {tail}",
            arm.name(),
        );
        assert!(
            resume.contains(ARM_CWD),
            "{}: the command must establish the session's cwd (spec §5.3 — click = ssh + cd + \
             resume)",
            arm.name(),
        );

        let launch = agent_launch_command(arm.kind, Some(ARM_CWD), None);
        let launch_tail = invocation_tail(&launch);
        assert!(
            launch_tail.starts_with(arm.binary),
            "{}: launch invocation must start with the binary — got {launch_tail}",
            arm.name(),
        );
        assert!(
            !launch_tail.contains(ARM_SESSION_ID),
            "{}: a FRESH launch must not carry a session id (CC's born-identity `--session-id` \
             is added by its own birth site, not by the generic launch builder)",
            arm.name(),
        );
    }
}

/// A terminal identity pinned for the duration of a test, so the invocation
/// under comparison is a fixed string instead of whatever the host happens to
/// export.
///
/// Two separate defects made this necessary, and both are worth naming because
/// the module doc above promises this table "can never go stale-green":
///
///  - **It was flaky.** These builders read the palette from process-global env,
///    which `codex_cli`'s own tests clear and rewrite. Comparing an arm against
///    its twin means calling the SAME function with the SAME arguments twice, so
///    an interleaved write between the two calls produced a diff and the test
///    reported it as "launch command forks on locality" — a locality fork that
///    did not exist. It failed only on a host that HAS a palette exported, i.e.
///    inside a yggterm session.
///  - **It was vacuous.** On a bare host with no palette, both sides were
///    colourless and the assertion compared two commands that carried none of
///    the identity it exists to protect. It would have passed with the palette
///    dropped from the remote arm entirely.
///
/// Pinning fixes both: the comparison is deterministic, and it is made against a
/// palette that is known to be present.
#[cfg(test)]
fn pinned_terminal_identity() -> crate::codex_cli::TerminalIdentityColorProfile {
    crate::codex_cli::TerminalIdentityColorProfile {
        foreground: "#e5e5e5".to_string(),
        background: "#262a33".to_string(),
        palette: (0..16).map(|i| format!("#{i:02x}{i:02x}{i:02x}")).collect(),
    }
}

#[test]
fn locality_does_not_fork_the_invocation() {
    // Spec §2 corollary 2: local and remote differ only INSIDE the transport
    // (direct spawn vs login-shell-wrapped ssh). The command itself must be the
    // same string on both arms — if it is not, some caller above the transport
    // seam is branching on locality.
    let _env = crate::codex_cli::env_test_guard();

    for arm in ARMS {
        compare_arm_against_twin("resume", arm, |kind| {
            persistent_agent_resume_command(kind, Some(ARM_CWD), ARM_SESSION_ID)
        });
        compare_arm_against_twin("launch", arm, |kind| {
            agent_launch_command(kind, Some(ARM_CWD), None)
        });
    }
}

/// Build one arm's command and its twin's from the SAME pinned identity, and
/// assert they agree.
///
/// The awkward shape here is not defensiveness, it is the only honest way to
/// test these builders. They read the terminal palette out of process-global
/// env **by design** — the daemon needs one identity for every child PTY it
/// spawns — and applying a theme anywhere in the crate re-syncs that env from
/// itself, clearing the palette when it cannot read a complete one back. cargo
/// runs tests as threads of one process, so a theme sync on another thread can
/// land between these two calls and has: the failure that led here had the
/// pinned palette on one side and none on the other, which reads as
/// "CodexLiteLlm/Remote: resume command forks on locality" when nothing forked.
///
/// Serializing does not fix it, because the clearing reaches this env through
/// PRODUCTION code from tests that have no reason to know this guard exists.
/// So: only compare a pair that was demonstrably built from the same identity,
/// and re-pin and retry when it was not. A genuine locality fork carries the
/// pinned identity on BOTH sides and therefore fails on the first attempt and
/// every retry — the assertion is as strong as it ever was. What retrying
/// removes is only the false red.
#[cfg(test)]
fn compare_arm_against_twin(label: &str, arm: &Arm, build: impl Fn(SessionKind) -> String) {
    let twin = ARMS
        .iter()
        .find(|other| other.kind == arm.kind && other.locality != arm.locality)
        .expect("every arm has a locality twin");
    let profile = pinned_terminal_identity();

    for attempt in 1..=8 {
        crate::codex_cli::sync_terminal_identity_appearance_with_profile("dark", Some(&profile));
        let left = build(arm.kind);
        let right = build(twin.kind);

        // Both sides must carry the identity we pinned, or the inputs were not
        // equal and the comparison would be measuring the theft, not the arms.
        if left.contains(&profile.background) && right.contains(&profile.background) {
            assert_eq!(
                left,
                right,
                "{}: {label} command forks on locality",
                arm.name(),
            );
            return;
        }
        assert!(
            attempt < 8,
            "{}: could not build the {label} pair from one identity in 8 attempts — \
             something is clearing the terminal palette continuously, which is a real \
             defect rather than a flake. left carries it: {}, right carries it: {}",
            arm.name(),
            left.contains(&profile.background),
            right.contains(&profile.background),
        );
    }
}

/// The equality above compares an arm against its twin, which passes trivially
/// if BOTH sides carry no terminal identity at all — and on a host that exports
/// no palette, that is exactly what happens. So prove separately, and with the
/// shortest possible window between pinning and reading, that the identity
/// really does reach the built command on every arm. Without this the matrix
/// would go green with the palette dropped from the remote arm entirely, which
/// is the stale-green the module doc says this table cannot have.
#[test]
fn every_arm_carries_the_pinned_terminal_identity_into_its_invocation() {
    let _env = crate::codex_cli::env_test_guard();
    let profile = pinned_terminal_identity();

    for arm in ARMS {
        // Same retry as the twin comparison, for the same reason: a theme sync
        // on another thread can clear the palette between the pin and the build.
        // Eight consecutive thefts is not a flake, and the message says so.
        let mut carried = false;
        for _ in 0..8 {
            crate::codex_cli::sync_terminal_identity_appearance_with_profile(
                "dark",
                Some(&profile),
            );
            if agent_launch_command(arm.kind, Some(ARM_CWD), None).contains(&profile.background) {
                carried = true;
                break;
            }
        }
        assert!(
            carried,
            "{}: the launch command dropped the pinned terminal identity on every one of \
             8 attempts",
            arm.name(),
        );
    }
}

#[test]
fn every_remote_arm_names_its_wrapper_subcommands() {
    for arm in ARMS {
        match arm.locality {
            Locality::Remote => {
                assert_eq!(
                    Some(remote_agent_resume_subcommand(arm.kind)),
                    arm.remote_resume_subcommand,
                    "{}: remote resume subcommand drifted",
                    arm.name(),
                );
                assert_eq!(
                    Some(remote_agent_start_subcommand(arm.kind)),
                    arm.remote_start_subcommand,
                    "{}: remote start subcommand drifted",
                    arm.name(),
                );
            }
            Locality::Local => {
                assert!(
                    arm.remote_resume_subcommand.is_none() && arm.remote_start_subcommand.is_none(),
                    "{}: a local arm crosses no transport seam and must name no wrapper \
                     subcommand (spec §2 corollary 2)",
                    arm.name(),
                );
            }
        }
    }
}

#[test]
fn every_arm_routes_a_keystroke_the_same_way_as_its_locality_twin() {
    for arm in ARMS {
        let row_path = arm.row_path();
        assert_eq!(
            terminal_write_strategy_for_path(&row_path, false),
            arm.write_strategy_without_local_runtime,
            "{}: a keystroke with no local runtime goes somewhere the matrix does not expect. \
             This is the cell that was wrong for remote-cc — RemoteDirectFallback for EVERY \
             remote agent scheme, or the keystroke is aimed at a runtime that does not exist",
            arm.name(),
        );
        assert_eq!(
            terminal_write_strategy_for_path(&row_path, true),
            TerminalWriteStrategy::LocalRuntime,
            "{}: a running local runtime takes the keystroke on every arm",
            arm.name(),
        );
    }

    // The invariant behind the per-arm cells, stated once: locality decides the
    // fallback, the CLI never does. If a future arm breaks this, the matrix
    // above will disagree with it and both tests fail — which is the point.
    for arm in ARMS {
        let expected = match arm.locality {
            Locality::Local => TerminalWriteStrategy::LocalRuntimeFallback,
            Locality::Remote => TerminalWriteStrategy::RemoteDirectFallback,
        };
        assert_eq!(
            arm.write_strategy_without_local_runtime,
            expected,
            "{}: write strategy must fork on LOCALITY only, never on CLI",
            arm.name(),
        );
    }
}

#[test]
fn every_arm_scans_the_store_its_descriptor_declares() {
    for arm in ARMS {
        let descriptor = agent_cli_descriptor(arm.kind)
            .unwrap_or_else(|| panic!("{}: every arm's kind is a registered CLI", arm.name()));
        assert_eq!(
            descriptor.session_store_globs,
            arm.store_globs,
            "{}: the arm and the registry disagree about where this CLI keeps its sessions",
            arm.name(),
        );
        // Both localities read the SAME store layout — the transport differs,
        // the layout does not. This is what phase 1b's ARGV-passed globs buy:
        // the remote scan script cannot spell a second encoding.
        let twin = ARMS
            .iter()
            .find(|other| other.kind == arm.kind && other.locality != arm.locality)
            .unwrap_or_else(|| panic!("{}: every arm has a locality twin", arm.name()));
        assert_eq!(
            arm.store_globs,
            twin.store_globs,
            "{}: store layout must not fork on locality",
            arm.name(),
        );
    }
}

#[test]
fn recorded_forks_still_reproduce_and_none_is_unrecorded() {
    let codex = agent_cli_descriptor(SessionKind::Codex).expect("codex is registered");
    let litellm =
        agent_cli_descriptor(SessionKind::CodexLiteLlm).expect("codex-litellm is registered");

    // Fork 1 — re-rooting. Locked in BOTH directions: if someone unifies it,
    // this fails and the RECORDED_ARM_FORKS row must be deleted in the same
    // commit, so the ledger cannot go stale-green.
    assert!(
        codex.resume_re_roots_with_cwd && !litellm.resume_re_roots_with_cwd,
        "the codex/codex-litellm re-rooting fork no longer reproduces — delete its row from \
         RECORDED_ARM_FORKS (and this assertion) in the same commit that unifies it",
    );

    // Fork 2 — the codex home env var. The LOCAL override is the descriptor's;
    // the REMOTE scan reads the CLI's own variable. Both halves asserted, so
    // unifying either one fails here.
    assert_eq!(
        codex.store_home_env_override,
        Some(yggterm_core::ENV_YGGTERM_CODEX_HOME),
        "the local codex home override moved — re-check the remote half before re-stamping",
    );
    assert!(
        include_str!("lib.rs").contains("std::env::var_os(\"CODEX_HOME\")"),
        "the remote codex home lookup no longer reads CODEX_HOME — if the two halves have been \
         unified, delete the fork row",
    );

    assert_eq!(
        RECORDED_ARM_FORKS.len(),
        2,
        "a fork was added or removed without updating this count — every recorded fork needs a \
         reason, and every reason needs an assertion above that proves it still reproduces",
    );
    for fork in RECORDED_ARM_FORKS {
        assert!(
            !fork.reason.trim().is_empty() && !fork.recorded.trim().is_empty(),
            "a recorded fork without a reason or a date is an accident wearing a table row: {}",
            fork.concern,
        );
    }
}
