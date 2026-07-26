//! The SHELL-side ARM MATRIX — phase 2b of the harness spec
//! (`docs/spec-agent-cli-harness.md` §8 phase 2, the "2b, NOT built" note).
//!
//! **What this is a twin of.** `yggterm_server::agent_arm_matrix` locks the
//! DAEMON-side axes: identity, invocation, store layout, keystroke routing. It
//! deliberately stops at the transport seam. Everything the GUI decides *after*
//! a row exists — whether the session gets the remote-resume readiness path,
//! what seeds its viewport on attach, how its mount is identified across a
//! reveal — lives in `yggterm-shell` and had no table at all. The spec calls
//! those out as §7.3 (readiness/overlay), §7.6 (attach seed) and §7.10
//! (mount/reveal), and names 2b a prerequisite for phase 3 rather than a
//! nicety, because the `remote-cc` readiness hole is one of them.
//!
//! **The invariant this table states, once.** Every axis below is a property of
//! WHERE the PTY lives, never of WHICH CLI is talking. A remote Claude Code
//! session is remote for exactly the same reasons a remote codex session is.
//! So the honest shape of each cell is `locality`, and any cell that reads the
//! CLI instead is a hole — the same class as `terminal_write_strategy_for_path`
//! matching `remote-session://` only, which cost the user a typable session.
//!
//! **Today four of these axes ARE holes, and this table says so out loud.**
//! They are recorded in [`RECORDED_SHELL_ARM_HOLES`] and locked in BOTH
//! directions, the same contract as phase 0's `KNOWN_PREDICATE_HOLES` and the
//! server matrix's `RECORDED_ARM_FORKS`: each hole is asserted to STILL
//! reproduce, so the day phase 3 fixes one, this test goes red until its row is
//! deleted. A ledger that can only pass is worth nothing.
//!
//! **What each cell is asserted against: the DECISION, never a restatement.**
//! Every axis below calls the function the PRODUCT calls at the moment it
//! decides — `terminal_mount_takes_remote_resume_readiness` (the mount's
//! readiness fork), `terminal_reveal_seed_allows_authoritative_screen` (the
//! retained-rehydrate seed), `codex_like_session` (the family predicate the seed
//! gate reads). That is deliberate and was a review finding: an earlier cut
//! asserted the `codex_like` cell against a test-local
//! `matches!(kind, Codex | CodexLiteLlm)`, which is a tautology — it reads no
//! production code, so widening the product's own `codex_like_session`, or
//! handing the seed policy a CC-inclusive bool at the call site, left this whole
//! file GREEN while the ledger rows below went stale. A lock that can only pass
//! is worth nothing; a lock that restates the code it is guarding is the same
//! thing wearing an assertion.
//!
//! **Scope honesty.** The axes here are the ones reachable as pure functions.
//! `resolve_active_open_mount_epoch` (the anti-churn epoch machinery, §7.10) is
//! a `&mut ShellState` method whose inputs are five live maps plus a clock; it
//! is NOT covered, and faking a ShellState to reach it would produce a lock on
//! the fake rather than on the product. What IS covered from §7.10 is the mount
//! IDENTITY those epochs feed — `terminal_mount_host_id`, the source of the
//! m1/m2 generation labels — which is where a cross-pathway double-construct
//! becomes visible.

use yggterm_core::SessionKind;
use yggterm_core::agent_cli::AGENT_CLIS;
use yggterm_core::{BrowserRow, BrowserRowKind};
use yggterm_server::{
    GhosttyTerminalHostMode, ManagedSessionView, RemoteDeployState, SessionMetadataEntry,
    SessionPreview, SessionSource, TerminalBackend, TerminalLaunchPhase,
};

use crate::shell::{
    codex_like_session, is_remote_scanned_sidebar_row, remote_session_starts_new_codex,
    session_path_names_remote_runtime_by_scheme, terminal_host_id_belongs_to_session,
    terminal_mount_host_id, terminal_mount_takes_remote_resume_readiness,
    terminal_reveal_seed_allows_authoritative_screen,
};
use crate::terminal_retained_replay_policy::RetainedRehydrateMode;

/// Where a session's PTY lives relative to the machine holding the row.
/// Mirrors the server matrix's enum deliberately: the two tables must be read
/// side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Locality {
    Local,
    Remote,
}

/// One arm of the shell-side matrix: a CLI on a locality.
struct ShellArm {
    kind: SessionKind,
    locality: Locality,
    /// The scheme that names a ROW on this arm (same value as the server
    /// matrix's `row_scheme`; asserted there, restated here only so this table
    /// is readable on its own).
    row_scheme: &'static str,

    // ---- §7.3 readiness / overlay ----
    /// Does this arm get the remote-resume readiness path at all?
    /// `is_remote_resume_agent_session` gates ~8 downstream signals
    /// (`terminal_has_meaningful_output`, `terminal_overlay_dismissed`,
    /// `attach_ready`, `stalled_remote_resume`, the read-poll cadence …).
    /// SHOULD be true for every remote arm.
    remote_resume_readiness: bool,
    /// Is this arm's row recognised as a remote-scanned sidebar row (which is
    /// what drives the "connecting to <machine>" loading notice)?
    /// SHOULD be true for every remote arm.
    scanned_sidebar_row: bool,
    /// Does this arm's PATH alone say "the runtime is remote"? This is the only
    /// half that can answer before a session view exists.
    /// SHOULD be true for every remote arm.
    remote_runtime_by_scheme: bool,
    /// Is this arm's cold launch (start-a-NEW-session, not resume) recognised
    /// by the cold-launch discriminator?
    /// SHOULD be true for every remote arm.
    cold_launch_discriminated: bool,

    // ---- §7.6 attach seed ----
    /// On a plain InitialRead reveal, may the seed fall back to the daemon's
    /// AUTHORITATIVE screen snapshot? Gated on `codex_like`, so CC is excluded
    /// — the `remote-cc-replay-codex-only` / snapshot-poison axis.
    replay_screen_fallback_on_initial_read: bool,
    /// What shell.rs `codex_like_session(kind)` — the family predicate the gate
    /// above reads — answers for this arm. Asserted against that FUNCTION, not
    /// against a `matches!` retyped here, so widening the product's own answer
    /// cannot leave this table green.
    codex_like: bool,
}

const ARM_MACHINE: &str = "dev";

/// A distinct fixture session id per CLI.
///
/// One shared id across arms looked tidier and was wrong: a local row's path is
/// `local://<id>` with no CLI in it, so three CLIs sharing one id produced three
/// IDENTICAL paths — and the mount-identity test below then "caught" a collision
/// that only its own fixture had created. Session ids are per session in
/// reality; two CLIs never hold the same one. Keeping them distinct is what
/// makes the distinctness lock a statement about the product.
fn arm_session_id(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Codex => "11111111-2222-3333-4444-555555555555",
        SessionKind::CodexLiteLlm => "22222222-3333-4444-5555-666666666666",
        SessionKind::ClaudeCode => "33333333-4444-5555-6666-777777777777",
        // The matrix is derived from AGENT_CLIS, so a new CLI reaching here
        // without an id is the same class of miss the table exists to catch.
        other => panic!("{other:?} is in the arm matrix but has no fixture session id"),
    }
}

const SHELL_ARMS: &[ShellArm] = &[
    ShellArm {
        kind: SessionKind::Codex,
        locality: Locality::Local,
        row_scheme: "local://",
        remote_resume_readiness: false,
        scanned_sidebar_row: false,
        remote_runtime_by_scheme: false,
        cold_launch_discriminated: false,
        replay_screen_fallback_on_initial_read: true,
        codex_like: true,
    },
    ShellArm {
        kind: SessionKind::Codex,
        locality: Locality::Remote,
        row_scheme: "remote-session://",
        remote_resume_readiness: true,
        scanned_sidebar_row: true,
        remote_runtime_by_scheme: true,
        cold_launch_discriminated: true,
        replay_screen_fallback_on_initial_read: true,
        codex_like: true,
    },
    ShellArm {
        kind: SessionKind::CodexLiteLlm,
        locality: Locality::Local,
        row_scheme: "local://",
        remote_resume_readiness: false,
        scanned_sidebar_row: false,
        remote_runtime_by_scheme: false,
        cold_launch_discriminated: false,
        replay_screen_fallback_on_initial_read: true,
        codex_like: true,
    },
    ShellArm {
        kind: SessionKind::CodexLiteLlm,
        locality: Locality::Remote,
        row_scheme: "remote-session://",
        remote_resume_readiness: true,
        scanned_sidebar_row: true,
        remote_runtime_by_scheme: true,
        cold_launch_discriminated: true,
        replay_screen_fallback_on_initial_read: true,
        codex_like: true,
    },
    ShellArm {
        kind: SessionKind::ClaudeCode,
        locality: Locality::Local,
        row_scheme: "local://",
        remote_resume_readiness: false,
        scanned_sidebar_row: false,
        remote_runtime_by_scheme: false,
        cold_launch_discriminated: false,
        // ⚠ HOLE: codex_like excludes CC, so a local CC reveal never offers the
        // authoritative screen. See RECORDED_SHELL_ARM_HOLES.
        replay_screen_fallback_on_initial_read: false,
        codex_like: false,
    },
    ShellArm {
        kind: SessionKind::ClaudeCode,
        locality: Locality::Remote,
        row_scheme: "remote-cc://",
        // ⚠ FOUR HOLES ON ONE ARM. Every cell below SHOULD read `true` — it is
        // a remote agent session exactly like its codex twin — and every one of
        // them reads the scheme `remote-session://` instead of asking about
        // locality. This single arm is why the spec makes 2b a prerequisite for
        // phase 3. See RECORDED_SHELL_ARM_HOLES.
        remote_resume_readiness: false,
        scanned_sidebar_row: false,
        remote_runtime_by_scheme: false,
        cold_launch_discriminated: false,
        replay_screen_fallback_on_initial_read: false,
        codex_like: false,
    },
];

/// A shell-side divergence that is STILL REAL. Recorded so it is a decision
/// rather than an accident, and locked in both directions so the row cannot
/// outlive the hole.
struct RecordedShellArmHole {
    /// What forks, in the vocabulary of the spec's §7 inventory.
    ///
    /// FORMAT, and it is load-bearing: `<axis name>: <what is wrong>`. The
    /// prefix before the first `:` must be the [`ShellArm`] field this hole is
    /// about — `readiness_axes_…` READS it to check that the deviating cells and
    /// the ledger are the same set.
    concern: &'static str,
    /// The spec section that inventories it.
    spec: &'static str,
    /// When it was recorded / last re-verified against main.
    recorded: &'static str,
    /// What the user sees because of it. A hole with no user-visible
    /// consequence does not belong in a ledger, it belongs in a comment.
    symptom: &'static str,
}

const RECORDED_SHELL_ARM_HOLES: &[RecordedShellArmHole] = &[
    RecordedShellArmHole {
        concern: "remote_resume_readiness: is_remote_resume_agent_session matches \
                  remote-session:// only, so ClaudeCode/Remote gets NO readiness/overlay path",
        spec: "§7.3",
        recorded: "2026-07-26",
        symptom: "a remote-cc session shows none of the ~8 remote-resume readiness signals — no \
                  attach_ready, no stalled_remote_resume, no overlay dismissal, and the slower \
                  default read-poll cadence — so a connecting or wedged remote-cc row looks \
                  identical to a healthy one",
    },
    RecordedShellArmHole {
        concern: "scanned_sidebar_row: is_remote_scanned_sidebar_row matches remote-session:// \
                  only",
        spec: "§7.3",
        recorded: "2026-07-26",
        symptom: "a remote-cc row shows no remote loading notice while its transport is still \
                  coming up — the row reads as ready before it is",
    },
    RecordedShellArmHole {
        concern: "remote_runtime_by_scheme: the scheme half of \
                  terminal_session_uses_remote_runtime knows remote-session:// and ssh://, not \
                  remote-cc://",
        spec: "§7.3",
        recorded: "2026-07-26",
        symptom: "before a session view exists, a remote-cc path is classified as LOCAL; it only \
                  becomes remote once a LiveSsh view is found, so every decision taken in that \
                  window takes the local branch",
    },
    RecordedShellArmHole {
        concern: "cold_launch_discriminated: remote_session_starts_new_codex has no start-cc twin",
        spec: "§7.3",
        recorded: "2026-07-26",
        symptom: "a freshly STARTED remote-cc session is not distinguished from a RESUMED one, so \
                  it gets the resume-flavoured reveal (which expects prior content) on a session \
                  that has none",
    },
    RecordedShellArmHole {
        concern: "replay_screen_fallback_on_initial_read: retained_rehydrate_allow_screen_fallback \
                  is codex_like-gated, and codex_like excludes ClaudeCode on BOTH localities",
        spec: "§7.6 / §7.10",
        recorded: "2026-07-26",
        symptom: "on a plain InitialRead reveal a CC session is never offered the daemon's \
                  authoritative screen, so it falls back to its own sparse xterm snapshot — the \
                  clipped / truncated-bottom paint. CC recovers only in \
                  CollapsedScrollbackRecovery mode",
    },
];

impl ShellArm {
    fn name(&self) -> String {
        format!("{:?}/{:?}", self.kind, self.locality)
    }

    /// The row path this arm produces for the fixture session. Built from the
    /// declared scheme rather than by calling the server's constructors, so
    /// this crate's table stays readable without a server dependency for
    /// identity — the server matrix already locks that the constructors emit
    /// these schemes.
    fn row_path(&self) -> String {
        let id = arm_session_id(self.kind);
        match self.locality {
            Locality::Local => format!("{}{id}", self.row_scheme),
            Locality::Remote => format!("{}{ARM_MACHINE}/{id}", self.row_scheme),
        }
    }

    /// A live session view for this arm, as the GUI would hold it mid-attach.
    fn session_view(&self, launch_action: Option<&str>) -> ManagedSessionView {
        ManagedSessionView {
            id: arm_session_id(self.kind).to_string(),
            session_path: self.row_path(),
            title: self.name(),
            kind: self.kind,
            host_label: ARM_MACHINE.to_string(),
            // Every arm here is a LIVE agent session. The remote ones are
            // LiveSsh — which is precisely the state
            // `is_remote_resume_agent_session` exists to recognise, so a false
            // cell below is a genuine miss and not a mis-set fixture.
            source: match self.locality {
                Locality::Local => SessionSource::LiveLocal,
                Locality::Remote => SessionSource::LiveSsh,
            },
            backend: TerminalBackend::Xterm,
            bridge_available: true,
            launch_phase: TerminalLaunchPhase::Running,
            remote_deploy_state: RemoteDeployState::Ready,
            launch_command: String::new(),
            status_line: String::new(),
            terminal_lines: Vec::new(),
            rendered_sections: Vec::new(),
            preview: SessionPreview {
                summary: Vec::new(),
                blocks: Vec::new(),
            },
            metadata: launch_action
                .map(|action| {
                    vec![SessionMetadataEntry {
                        label: "Remote Launch Action",
                        value: action.to_string(),
                    }]
                })
                .unwrap_or_default(),
            terminal_process_id: None,
            terminal_foreground_active: None,
            terminal_window_id: None,
            terminal_host_token: None,
            terminal_host_mode: GhosttyTerminalHostMode::Unsupported,
            embedded_surface_id: None,
            embedded_surface_detail: None,
            last_launch_error: None,
            last_window_error: None,
            ssh_target: match self.locality {
                Locality::Local => None,
                Locality::Remote => Some(ARM_MACHINE.to_string()),
            },
            ssh_prefix: None,
            stored_preview_hydrated: true,
            working: None,
        }
    }

    /// The sidebar row this arm produces.
    fn sidebar_row(&self) -> BrowserRow {
        BrowserRow {
            kind: BrowserRowKind::Session,
            full_path: self.row_path(),
            label: self.name(),
            detail_label: String::new(),
            document_kind: None,
            group_kind: None,
            session_title: Some(self.name()),
            depth: 1,
            host_label: ARM_MACHINE.to_string(),
            descendant_sessions: 0,
            expanded: false,
            session_id: Some(arm_session_id(self.kind).to_string()),
            session_cwd: None,
            session_kind: Some(self.kind),
        }
    }

    /// The wrapper subcommand a COLD launch of this arm carries in its
    /// `Remote Launch Action` metadata. Derived from the CLI family the same
    /// way the server's remote start subcommand is, so this fixture cannot
    /// claim a launch action the transport would never write.
    fn cold_launch_action(&self) -> Option<&'static str> {
        match (self.kind, self.locality) {
            (_, Locality::Local) => None,
            (SessionKind::ClaudeCode, Locality::Remote) => Some("start-cc"),
            (_, Locality::Remote) => Some("start-codex"),
        }
    }
}

#[test]
fn every_registered_cli_has_both_shell_arms() {
    for descriptor in AGENT_CLIS {
        for locality in [Locality::Local, Locality::Remote] {
            let matches = SHELL_ARMS
                .iter()
                .filter(|arm| arm.kind == descriptor.kind && arm.locality == locality)
                .count();
            assert_eq!(
                matches, 1,
                "{:?} on {locality:?} must appear EXACTLY once in the shell arm matrix — the \
                 GUI-side decisions are per-arm too, and a CLI with daemon arms but no shell \
                 arms is exactly how the remote-cc readiness hole survived (spec §6 A6)",
                descriptor.kind,
            );
        }
    }
    assert_eq!(
        SHELL_ARMS.len(),
        AGENT_CLIS.len() * 2,
        "the shell matrix must be exactly the registry crossed with locality — an extra arm \
         means an arm for a CLI that no longer ships",
    );
}

#[test]
fn every_arm_gets_the_remote_resume_readiness_its_matrix_declares() {
    for arm in SHELL_ARMS {
        let session = arm.session_view(None);
        // The MOUNT's decision, not the predicate under it: a fix applied at the
        // TerminalCanvas call site must fail here too.
        assert_eq!(
            terminal_mount_takes_remote_resume_readiness(&session),
            arm.remote_resume_readiness,
            "{}: remote-resume readiness drifted from the matrix. This gate drives ~8 downstream \
             signals (attach_ready, stalled_remote_resume, overlay dismissal, read-poll cadence) \
             — if this arm now gets them, delete its RECORDED_SHELL_ARM_HOLES row in the same \
             commit (spec §7.3)",
            arm.name(),
        );
    }
}

#[test]
fn every_arm_is_recognised_as_a_scanned_sidebar_row_its_matrix_declares() {
    for arm in SHELL_ARMS {
        assert_eq!(
            is_remote_scanned_sidebar_row(&arm.sidebar_row()),
            arm.scanned_sidebar_row,
            "{}: remote-scanned sidebar classification drifted from the matrix (spec §7.3)",
            arm.name(),
        );
    }
}

#[test]
fn every_arm_declares_remote_runtime_by_scheme_as_its_matrix_says() {
    for arm in SHELL_ARMS {
        assert_eq!(
            session_path_names_remote_runtime_by_scheme(&arm.row_path()),
            arm.remote_runtime_by_scheme,
            "{}: the scheme-only remote-runtime answer drifted from the matrix. This is the ONLY \
             half that answers before a session view exists (spec §7.3)",
            arm.name(),
        );
    }
}

#[test]
fn every_arm_cold_launch_is_discriminated_as_its_matrix_says() {
    for arm in SHELL_ARMS {
        let session = arm.session_view(arm.cold_launch_action());
        assert_eq!(
            remote_session_starts_new_codex(&session),
            arm.cold_launch_discriminated,
            "{}: the cold-launch (start vs resume) discriminator drifted from the matrix. A \
             STARTED session has no prior content; a RESUMED one does, and the reveal differs \
             (spec §7.3)",
            arm.name(),
        );
    }
}

#[test]
fn every_arm_seeds_its_attach_the_way_its_matrix_says() {
    for arm in SHELL_ARMS {
        // Asked by SessionKind, exactly as the retained-rehydrate task asks it.
        // Passing `arm.codex_like` to the policy function instead would lock the
        // policy while leaving the seed DECISION free to change under it.
        assert_eq!(
            terminal_reveal_seed_allows_authoritative_screen(
                RetainedRehydrateMode::InitialRead,
                arm.kind,
            ),
            arm.replay_screen_fallback_on_initial_read,
            "{}: the InitialRead seed policy drifted from the matrix — this is the \
             snapshot-poison / remote-cc-replay-codex-only axis (spec §7.6)",
            arm.name(),
        );
        // The family predicate the gate above reads, asserted against the
        // PRODUCT's own function. Restating its `matches!` here would be a
        // tautology: widening `codex_like_session` would leave this green.
        assert_eq!(
            codex_like_session(arm.kind),
            arm.codex_like,
            "{}: the matrix's codex_like cell must equal shell.rs `codex_like_session`, or the \
             two drift apart silently",
            arm.name(),
        );
    }
}

#[test]
fn collapsed_scrollback_recovery_offers_the_screen_to_every_arm() {
    // The one seed property that is already unified, asserted so a future
    // narrowing of the gate cannot take it away quietly: whatever the CLI, a
    // recovery reveal is allowed the daemon's authoritative screen. That is
    // what makes the CC hole above a DEGRADED path rather than a dead one.
    for arm in SHELL_ARMS {
        assert!(
            terminal_reveal_seed_allows_authoritative_screen(
                RetainedRehydrateMode::CollapsedScrollbackRecovery,
                arm.kind,
            ),
            "{}: a collapsed-scrollback RECOVERY reveal must be offered the daemon screen on \
             every arm — this is the only path by which a CC session recovers a clipped \
             viewport at all (spec §7.6)",
            arm.name(),
        );
    }
}

#[test]
fn every_arm_mount_identity_is_distinct_and_belongs_to_its_own_session() {
    // §7.10: the m1/m2 generation labels come from this id. Two arms sharing a
    // mount id, or an id that a session does not recognise as its own, is how a
    // cross-pathway switch double-constructs and blinks.
    let mut seen: Vec<(String, String)> = Vec::new();
    for arm in SHELL_ARMS {
        let path = arm.row_path();
        for epoch in [1u64, 2u64] {
            let host_id = terminal_mount_host_id(&path, epoch);
            assert!(
                terminal_host_id_belongs_to_session(&path, &host_id),
                "{}: mount host id {host_id} is not recognised as belonging to its own session \
                 {path} (spec §7.10)",
                arm.name(),
            );
            let label = format!("{} epoch {epoch}", arm.name());
            for (other_label, other_id) in &seen {
                assert_ne!(
                    other_id, &host_id,
                    "{label} and {other_label} share the mount host id {host_id} — two mounts \
                     revealing into one host is the cross-pathway double-construct. Across ARMS \
                     it means two sessions collide; across EPOCHS of one arm it means a cold \
                     remount is indistinguishable from a reveal-in-place (spec §7.10)",
                );
            }
            seen.push((label, host_id));
        }
    }

    // The epoch must actually change identity, or the anti-churn machinery's
    // "cold remount" and "reveal in place" would be indistinguishable to every
    // consumer of the label.
    for arm in SHELL_ARMS {
        let path = arm.row_path();
        assert_ne!(
            terminal_mount_host_id(&path, 1),
            terminal_mount_host_id(&path, 2),
            "{}: bumping the mount epoch must change the host id (spec §7.10)",
            arm.name(),
        );
    }
}

#[test]
fn readiness_axes_should_fork_on_locality_and_the_deviations_are_all_recorded() {
    // THE invariant, stated once. Every §7.3 axis is a property of where the
    // PTY lives. Any arm whose cell disagrees with its locality is a hole, and
    // that set must match the ledger — so a NEW hole fails here for being
    // unrecorded, and a FIXED one fails for being stale.
    //
    // ⚠ SCOPE, stated plainly because it was over-claimed once: this test makes
    // ZERO production calls. It reads the TABLE and the LEDGER, so it cannot be
    // turned red by mutating production, and it is NOT what makes holes 1-4
    // both-directions locks. That property comes from the per-axis tests above
    // (table cell vs the product's own decision function) plus
    // `recorded_holes_still_reproduce_and_none_is_unrecorded` (the hole vs its
    // locality twin). What THIS test adds is table/ledger integrity: it is the
    // only place that says a deviating cell without a ledger row — or a ledger
    // row without a deviating cell — is a bug.
    let mut deviations = Vec::new();
    let mut deviating_axes: Vec<&str> = Vec::new();
    for arm in SHELL_ARMS {
        let remote = arm.locality == Locality::Remote;
        for (axis, actual) in [
            ("remote_resume_readiness", arm.remote_resume_readiness),
            ("scanned_sidebar_row", arm.scanned_sidebar_row),
            ("remote_runtime_by_scheme", arm.remote_runtime_by_scheme),
            ("cold_launch_discriminated", arm.cold_launch_discriminated),
        ] {
            if actual != remote {
                deviations.push(format!("{} {axis}", arm.name()));
                if !deviating_axes.contains(&axis) {
                    deviating_axes.push(axis);
                }
            }
        }
    }

    assert_eq!(
        deviations,
        vec![
            "ClaudeCode/Remote remote_resume_readiness",
            "ClaudeCode/Remote scanned_sidebar_row",
            "ClaudeCode/Remote remote_runtime_by_scheme",
            "ClaudeCode/Remote cold_launch_discriminated",
        ],
        "the set of arms that fork on CLI rather than locality changed. Every deviation must be \
         a RECORDED_SHELL_ARM_HOLES row; a new one here is an unrecorded regression, and a \
         missing one means the hole was FIXED — delete its ledger row in the same commit \
         (spec §7.3, phase 3)",
    );

    // …and the ledger is READ, not just referred to in prose. Each recorded
    // hole names its axis as the prefix of `concern`, so the §7.3 rows must be
    // exactly the axes that deviated above.
    let mut recorded_axes: Vec<&str> = RECORDED_SHELL_ARM_HOLES
        .iter()
        .filter(|hole| hole.spec.contains("§7.3"))
        .map(|hole| {
            hole.concern
                .split(':')
                .next()
                .expect("a concern always has a prefix")
                .trim()
        })
        .collect();
    recorded_axes.sort_unstable();
    let mut deviating_axes = deviating_axes;
    deviating_axes.sort_unstable();
    assert_eq!(
        deviating_axes, recorded_axes,
        "the §7.3 axes that deviate from locality and the §7.3 rows of \
         RECORDED_SHELL_ARM_HOLES have to be the SAME SET. A deviating axis with no ledger row is \
         an unrecorded hole (no symptom, no owner); a ledger row for an axis that no longer \
         deviates is a row that outlived its hole (spec §7.3, phase 3)",
    );
}

#[test]
fn recorded_holes_still_reproduce_and_none_is_unrecorded() {
    // Both directions, per the phase-0 burn-down contract. Each assertion below
    // reproduces its hole against the PRODUCTION function, not against the
    // table — the table is what drifts, the function is the truth.
    let cc_remote = SHELL_ARMS
        .iter()
        .find(|arm| arm.kind == SessionKind::ClaudeCode && arm.locality == Locality::Remote)
        .expect("the CC remote arm is where these holes live");
    let codex_remote = SHELL_ARMS
        .iter()
        .find(|arm| arm.kind == SessionKind::Codex && arm.locality == Locality::Remote)
        .expect("the codex remote arm is the twin these are measured against");

    // Holes 1-4: the CC remote arm is denied what its codex twin gets, on four
    // axes. Asserting the TWIN's answer in the same breath is what makes this a
    // hole rather than a global "nothing is ready yet".
    assert!(
        terminal_mount_takes_remote_resume_readiness(&codex_remote.session_view(None))
            && !terminal_mount_takes_remote_resume_readiness(&cc_remote.session_view(None)),
        "hole 1 (§7.3 readiness) no longer reproduces — delete its RECORDED_SHELL_ARM_HOLES row \
         and flip the matrix cell in the same commit",
    );
    assert!(
        is_remote_scanned_sidebar_row(&codex_remote.sidebar_row())
            && !is_remote_scanned_sidebar_row(&cc_remote.sidebar_row()),
        "hole 2 (§7.3 scanned sidebar row) no longer reproduces — delete its ledger row",
    );
    assert!(
        session_path_names_remote_runtime_by_scheme(&codex_remote.row_path())
            && !session_path_names_remote_runtime_by_scheme(&cc_remote.row_path()),
        "hole 3 (§7.3 remote runtime by scheme) no longer reproduces — delete its ledger row",
    );
    assert!(
        remote_session_starts_new_codex(
            &codex_remote.session_view(codex_remote.cold_launch_action())
        ) && !remote_session_starts_new_codex(
            &cc_remote.session_view(cc_remote.cold_launch_action())
        ),
        "hole 4 (§7.3 cold-launch discriminator) no longer reproduces — there is now a start-cc \
         twin; delete its ledger row",
    );

    // Hole 5: the seed gate excludes CC on BOTH localities, so it is asserted
    // against the CLI axis (CC vs its codex twin) rather than a locality twin.
    // Asked through the seed DECISION, so widening `codex_like_session` and
    // widening the decision itself both land here.
    assert!(
        !terminal_reveal_seed_allows_authoritative_screen(
            RetainedRehydrateMode::InitialRead,
            SessionKind::ClaudeCode,
        ) && terminal_reveal_seed_allows_authoritative_screen(
            RetainedRehydrateMode::InitialRead,
            SessionKind::Codex,
        ),
        "hole 5 (§7.6 codex-only screen fallback) no longer reproduces — delete its ledger row",
    );

    assert_eq!(
        RECORDED_SHELL_ARM_HOLES.len(),
        5,
        "a hole was added or removed without updating this count — every recorded hole needs a \
         symptom, and every symptom needs an assertion above that proves it still reproduces",
    );
    for hole in RECORDED_SHELL_ARM_HOLES {
        assert!(
            !hole.symptom.trim().is_empty()
                && !hole.recorded.trim().is_empty()
                && !hole.spec.trim().is_empty(),
            "a recorded hole without a symptom, a date or a spec reference is an accident \
             wearing a table row: {}",
            hole.concern,
        );
    }
}
