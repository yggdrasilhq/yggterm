//! THE scheme registry — phase 0 of the agent-CLI harness contract
//! (`docs/spec-agent-cli-harness.md` §2.3/§4/§8).
//!
//! Every session-path / runtime-key scheme is declared HERE, once, with its
//! role. Every predicate that filters by scheme must derive from this
//! registry — a predicate with a hand-written scheme list is a bug (the
//! sanitizer's missing `cc-runtime://` is the standing exhibit, §7.2).
//!
//! Phase 0 is a PURE ADDITION: the registry + the predicate LOCKS. The locks
//! live in the crates that own each predicate and iterate this registry; a
//! hole a lock finds is either fixed on the spot (when it is a live
//! user-confirmed bug) or recorded in [`KNOWN_PREDICATE_HOLES`] — the
//! burn-down list later phases empty. A hole that gets FIXED must be removed
//! from the table in the same commit (the locks assert both directions, so a
//! stale table entry fails the build exactly like a new hole).

use crate::SessionKind;

/// What a scheme names. One string can play two roles — `local://` is BOTH
/// the sidebar row identity and the daemon's runtime key for local rows
/// (§7.1: "one string, two roles — the registry must model that, not paper
/// over it").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeRole {
    /// Sidebar/session ROW identity — what the user clicks, what persists.
    RowIdentity,
    /// Terminal-runtime key — what the owning daemon's PTY table is keyed by.
    RuntimeKey,
    /// Both at once (`local://`).
    RowAndRuntimeKey,
}

/// Which side of the transport seam the scheme's referent lives on, from the
/// GUI host's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeLocality {
    Local,
    Remote,
}

/// One scheme, declared once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemeDescriptor {
    /// The full prefix including its separator (`://` or `::`).
    pub prefix: &'static str,
    pub role: SchemeRole,
    pub locality: SchemeLocality,
    /// The agent CLI this scheme is specific to. `None` = kind-agnostic
    /// (`local://` hosts shells AND every local agent kind — which is exactly
    /// why path-prefix display dispatch is banned, AGENTS.md).
    pub kind: Option<SessionKind>,
    /// May this scheme carry a FIRST-CLASS agent session? Drives which
    /// predicates the locks require to cover it.
    pub agent: bool,
    /// Parse-only alias: still recognized on the wire and on disk, never
    /// constructed for new sessions. Locks require PARSERS to accept these
    /// but never require constructors to produce them.
    pub legacy: bool,
    /// A realistic example key for lock tests to feed predicates. Synthetic
    /// UUIDs only — never a real session id (public repo).
    pub example: &'static str,
}

/// The registry. Adding a scheme here is the ONLY way to introduce one; the
/// locks in each owning crate pick it up on their next run.
pub const SESSION_PATH_SCHEMES: &[SchemeDescriptor] = &[
    // ── Row identity (+ runtime key for local) ─────────────────────────────
    SchemeDescriptor {
        prefix: "local://",
        role: SchemeRole::RowAndRuntimeKey,
        locality: SchemeLocality::Local,
        kind: None,
        agent: true,
        legacy: false,
        example: "local://00000000-0000-4000-8000-000000000001",
    },
    SchemeDescriptor {
        // Historical name — this is the remote CODEX row scheme
        // (`remote-session://` ≠ `remote-codex://` only for historical
        // reasons; the 4-tuple module of §4 will hide that).
        prefix: "remote-session://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Codex),
        agent: true,
        legacy: false,
        example: "remote-session://devhost/00000000-0000-4000-8000-000000000002",
    },
    SchemeDescriptor {
        prefix: "remote-cc://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::ClaudeCode),
        agent: true,
        legacy: false,
        example: "remote-cc://devhost/00000000-0000-4000-8000-000000000003",
    },
    // ── Runtime keys ───────────────────────────────────────────────────────
    SchemeDescriptor {
        prefix: "codex-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Codex),
        agent: true,
        legacy: false,
        example: "codex-runtime://00000000-0000-4000-8000-000000000004",
    },
    SchemeDescriptor {
        prefix: "cc-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::ClaudeCode),
        agent: true,
        legacy: false,
        example: "cc-runtime://00000000-0000-4000-8000-000000000005",
    },
    // ── Non-agent (registered so shared predicates can be locked too) ──────
    SchemeDescriptor {
        prefix: "live::",
        role: SchemeRole::RowAndRuntimeKey,
        locality: SchemeLocality::Local,
        kind: Some(SessionKind::Shell),
        agent: false,
        legacy: false,
        example: "live::00000000-0000-4000-8000-000000000006",
    },
    SchemeDescriptor {
        prefix: "ssh://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::SshShell),
        agent: false,
        legacy: false,
        example: "ssh://devhost/00000000-0000-4000-8000-000000000007",
    },
    SchemeDescriptor {
        prefix: "document::",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Local,
        kind: Some(SessionKind::Document),
        agent: false,
        legacy: false,
        example: "document::00000000-0000-4000-8000-000000000008",
    },
    // ── Legacy parse-only aliases (§7.1, lib.rs:1634 family) ───────────────
    SchemeDescriptor {
        prefix: "codex://",
        role: SchemeRole::RowAndRuntimeKey,
        locality: SchemeLocality::Local,
        kind: Some(SessionKind::Codex),
        agent: true,
        legacy: true,
        example: "codex://00000000-0000-4000-8000-000000000009",
    },
    SchemeDescriptor {
        prefix: "codex::",
        role: SchemeRole::RowAndRuntimeKey,
        locality: SchemeLocality::Local,
        kind: Some(SessionKind::Codex),
        agent: true,
        legacy: true,
        example: "codex::00000000-0000-4000-8000-00000000000a",
    },
    SchemeDescriptor {
        prefix: "codex-litellm://",
        role: SchemeRole::RowAndRuntimeKey,
        locality: SchemeLocality::Local,
        kind: Some(SessionKind::CodexLiteLlm),
        agent: true,
        legacy: true,
        example: "codex-litellm://00000000-0000-4000-8000-00000000000b",
    },
    SchemeDescriptor {
        prefix: "codex-litellm::",
        role: SchemeRole::RowAndRuntimeKey,
        locality: SchemeLocality::Local,
        kind: Some(SessionKind::CodexLiteLlm),
        agent: true,
        legacy: true,
        example: "codex-litellm::00000000-0000-4000-8000-00000000000c",
    },
    SchemeDescriptor {
        prefix: "local::",
        role: SchemeRole::RowAndRuntimeKey,
        locality: SchemeLocality::Local,
        kind: None,
        agent: true,
        legacy: true,
        example: "local::00000000-0000-4000-8000-00000000000d",
    },
    SchemeDescriptor {
        prefix: "remote-runtime://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: None,
        agent: true,
        legacy: true,
        example: "remote-runtime://devhost/00000000-0000-4000-8000-00000000000e",
    },
];

/// Current (non-legacy) schemes that may carry an agent session, by role.
pub fn agent_row_identity_schemes() -> impl Iterator<Item = &'static SchemeDescriptor> {
    SESSION_PATH_SCHEMES.iter().filter(|scheme| {
        scheme.agent
            && !scheme.legacy
            && matches!(
                scheme.role,
                SchemeRole::RowIdentity | SchemeRole::RowAndRuntimeKey
            )
    })
}

pub fn agent_runtime_key_schemes() -> impl Iterator<Item = &'static SchemeDescriptor> {
    SESSION_PATH_SCHEMES.iter().filter(|scheme| {
        scheme.agent
            && !scheme.legacy
            && matches!(
                scheme.role,
                SchemeRole::RuntimeKey | SchemeRole::RowAndRuntimeKey
            )
    })
}

/// Current remote agent schemes (row or runtime) — the set the remote-side
/// predicates (`session_path_is_remote_agent`, write strategy, resume
/// readiness) must cover.
pub fn remote_agent_schemes() -> impl Iterator<Item = &'static SchemeDescriptor> {
    SESSION_PATH_SCHEMES
        .iter()
        .filter(|scheme| scheme.agent && !scheme.legacy && scheme.locality == SchemeLocality::Remote)
}

/// Legacy aliases parsers must still accept.
pub fn legacy_alias_schemes() -> impl Iterator<Item = &'static SchemeDescriptor> {
    SESSION_PATH_SCHEMES.iter().filter(|scheme| scheme.legacy)
}

pub fn scheme_for_prefix(prefix: &str) -> Option<&'static SchemeDescriptor> {
    SESSION_PATH_SCHEMES
        .iter()
        .find(|scheme| scheme.prefix == prefix)
}

/// One recorded, dated hole: a predicate that does not yet cover a scheme the
/// registry says it must. THE burn-down list (spec §7.2 is its source; each
/// row here was RE-VERIFIED against main on the recorded date). The lock
/// tests enforce both directions:
///   - a predicate missing a scheme NOT listed here fails the build;
///   - a listed hole that no longer reproduces fails the build until the row
///     is deleted (so the table can never go stale-green).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredicateHole {
    /// The predicate's fn name, exactly as in the owning crate.
    pub predicate: &'static str,
    /// The uncovered scheme prefix.
    pub scheme: &'static str,
    /// When the hole was recorded/re-verified.
    pub recorded: &'static str,
    /// The §7.2 consequence, one line.
    pub consequence: &'static str,
}

pub const KNOWN_PREDICATE_HOLES: &[PredicateHole] = &[
    PredicateHole {
        predicate: "local_runtime_id_from_key",
        scheme: "cc-runtime://",
        recorded: "2026-07-23",
        consequence: "cc-runtime keys unrecognized by recoverable/snapshot predicates + restore normalizers",
    },
    PredicateHole {
        predicate: "uses_runtime_owned_terminal_path",
        scheme: "remote-cc://",
        recorded: "2026-07-23",
        consequence: "CC daemon-owned runtimes miss runtime-owned handling",
    },
    PredicateHole {
        predicate: "uses_runtime_owned_terminal_path",
        scheme: "cc-runtime://",
        recorded: "2026-07-23",
        consequence: "CC daemon-owned runtimes miss runtime-owned handling",
    },
    PredicateHole {
        predicate: "terminal_key_prefers_initial_screen_snapshot",
        scheme: "remote-cc://",
        recorded: "2026-07-23",
        consequence: "CC attaches don't get the codex initial-snapshot seed policy",
    },
    PredicateHole {
        predicate: "terminal_key_prefers_initial_screen_snapshot",
        scheme: "cc-runtime://",
        recorded: "2026-07-23",
        consequence: "CC attaches don't get the codex initial-snapshot seed policy",
    },
    PredicateHole {
        predicate: "launch_command_looks_like_remote_resume_attach",
        scheme: "remote-cc://",
        recorded: "2026-07-23",
        consequence: "matches resume-codex/start-codex only — resume-cc/start-cc invisible",
    },
    PredicateHole {
        predicate: "initial_remote_attach_should_preserve_retained_chunks",
        scheme: "remote-cc://",
        recorded: "2026-07-23",
        consequence: "remote CC attaches never preserve retained chunks the codex way",
    },
    PredicateHole {
        predicate: "bridge_initial_snapshot_should_use_raw_stream",
        scheme: "cc-runtime://",
        recorded: "2026-07-23",
        consequence: "codex bridges delay raw stream, CC bridges take a different path",
    },
    PredicateHole {
        predicate: "terminal_line_internal_transport_error_index",
        scheme: "cc-runtime://",
        recorded: "2026-07-23",
        consequence: "a real `…not found: cc-runtime://…` transport error is NOT excised",
    },
    PredicateHole {
        predicate: "terminal_line_internal_transport_error_index",
        scheme: "remote-cc://",
        recorded: "2026-07-23",
        consequence: "a real `…not found: remote-cc://…` transport error is NOT excised",
    },
    PredicateHole {
        predicate: "terminal_line_is_internal_transport_error",
        scheme: "cc-runtime://",
        recorded: "2026-07-23",
        consequence: "same hole as the shell twin, second copy (terminal_observe)",
    },
    PredicateHole {
        predicate: "terminal_line_is_internal_transport_error",
        scheme: "remote-cc://",
        recorded: "2026-07-23",
        consequence: "same hole as the shell twin, second copy (terminal_observe)",
    },
    PredicateHole {
        predicate: "is_hot_terminal_sidebar_path",
        scheme: "cc-runtime://",
        recorded: "2026-07-23",
        consequence: "includes remote-cc but not cc-runtime",
    },
];

pub fn predicate_hole_allowed(predicate: &str, scheme: &str) -> bool {
    KNOWN_PREDICATE_HOLES
        .iter()
        .any(|hole| hole.predicate == predicate && hole.scheme == scheme)
}

/// The holes recorded for one predicate — the lock's second direction: each
/// must STILL reproduce, or the row has gone stale and must be deleted.
pub fn predicate_holes_for(predicate: &str) -> impl Iterator<Item = &'static PredicateHole> {
    KNOWN_PREDICATE_HOLES
        .iter()
        .filter(move |hole| hole.predicate == predicate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_prefixes_are_unique_and_well_formed() {
        for (index, scheme) in SESSION_PATH_SCHEMES.iter().enumerate() {
            assert!(
                scheme.prefix.ends_with("://") || scheme.prefix.ends_with("::"),
                "{} must end with its separator",
                scheme.prefix
            );
            assert!(
                scheme.example.starts_with(scheme.prefix),
                "{}'s example must use its own prefix",
                scheme.prefix
            );
            for other in &SESSION_PATH_SCHEMES[index + 1..] {
                assert_ne!(scheme.prefix, other.prefix, "duplicate scheme");
            }
        }
    }

    #[test]
    fn every_agent_kind_has_a_current_remote_identity_or_is_local_only() {
        // Codex and ClaudeCode both have remote row schemes; CodexLiteLlm is
        // local-only today (its legacy aliases are parse-only). This test is
        // the place a NEW CLI's scheme coverage becomes a conscious decision:
        // adding a kind to SessionKind::is_agent without deciding its remote
        // story fails here.
        for kind in [
            SessionKind::Codex,
            SessionKind::CodexLiteLlm,
            SessionKind::ClaudeCode,
        ] {
            let has_remote_row = SESSION_PATH_SCHEMES.iter().any(|scheme| {
                scheme.kind == Some(kind)
                    && !scheme.legacy
                    && scheme.locality == SchemeLocality::Remote
                    && matches!(scheme.role, SchemeRole::RowIdentity)
            });
            let local_only = matches!(kind, SessionKind::CodexLiteLlm);
            assert!(
                has_remote_row || local_only,
                "{kind:?} has no remote row scheme and is not declared local-only"
            );
        }
    }

    #[test]
    fn every_known_hole_names_a_registered_scheme() {
        for hole in KNOWN_PREDICATE_HOLES {
            assert!(
                scheme_for_prefix(hole.scheme).is_some(),
                "hole {}×{} names an unregistered scheme",
                hole.predicate,
                hole.scheme
            );
            assert!(!hole.recorded.is_empty() && !hole.consequence.is_empty());
        }
    }

    #[test]
    fn role_queries_partition_as_documented() {
        let rows: Vec<_> = agent_row_identity_schemes().map(|s| s.prefix).collect();
        assert!(rows.contains(&"local://"));
        assert!(rows.contains(&"remote-session://"));
        assert!(rows.contains(&"remote-cc://"));
        assert!(!rows.contains(&"codex-runtime://"));
        let runtimes: Vec<_> = agent_runtime_key_schemes().map(|s| s.prefix).collect();
        assert!(runtimes.contains(&"local://"), "local:// is BOTH roles");
        assert!(runtimes.contains(&"codex-runtime://"));
        assert!(runtimes.contains(&"cc-runtime://"));
        assert!(!runtimes.contains(&"remote-session://"));
        let remotes: Vec<_> = remote_agent_schemes().map(|s| s.prefix).collect();
        assert_eq!(
            remotes,
            vec![
                "remote-session://",
                "remote-cc://",
                "codex-runtime://",
                "cc-runtime://"
            ]
        );
    }
}
