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
    // ── The 2026-08-08 intake. Every row here is DERIVED from its
    // `AgentCliDescriptor` (`remote_row_scheme` / `runtime_key_scheme`) and the
    // lock `every_agent_descriptor_scheme_is_registered_and_vice_versa` fails the
    // build if the two ever disagree — a new CLI cannot land a scheme in one
    // place and forget the other.
    SchemeDescriptor {
        prefix: "remote-pi://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Pi),
        agent: true,
        legacy: false,
        example: "remote-pi://devhost/00000000-0000-4000-8000-000000000006",
    },
    SchemeDescriptor {
        prefix: "pi-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Pi),
        agent: true,
        legacy: false,
        example: "pi-runtime://00000000-0000-4000-8000-000000000026",
    },
    SchemeDescriptor {
        prefix: "remote-opencode://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::OpenCode),
        agent: true,
        legacy: false,
        example: "remote-opencode://devhost/00000000-0000-4000-8000-000000000007",
    },
    SchemeDescriptor {
        prefix: "opencode-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::OpenCode),
        agent: true,
        legacy: false,
        example: "opencode-runtime://00000000-0000-4000-8000-000000000027",
    },
    SchemeDescriptor {
        prefix: "remote-qwen://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::QwenCode),
        agent: true,
        legacy: false,
        example: "remote-qwen://devhost/00000000-0000-4000-8000-000000000008",
    },
    SchemeDescriptor {
        prefix: "qwen-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::QwenCode),
        agent: true,
        legacy: false,
        example: "qwen-runtime://00000000-0000-4000-8000-000000000028",
    },
    SchemeDescriptor {
        prefix: "remote-kimi://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Kimi),
        agent: true,
        legacy: false,
        example: "remote-kimi://devhost/00000000-0000-4000-8000-000000000009",
    },
    SchemeDescriptor {
        prefix: "kimi-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Kimi),
        agent: true,
        legacy: false,
        example: "kimi-runtime://00000000-0000-4000-8000-000000000029",
    },
    SchemeDescriptor {
        prefix: "remote-muse://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Muse),
        agent: true,
        legacy: false,
        example: "remote-muse://devhost/00000000-0000-4000-8000-000000000010",
    },
    SchemeDescriptor {
        prefix: "muse-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Muse),
        agent: true,
        legacy: false,
        example: "muse-runtime://00000000-0000-4000-8000-000000000030",
    },
    SchemeDescriptor {
        prefix: "remote-agy://",
        role: SchemeRole::RowIdentity,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Antigravity),
        agent: true,
        legacy: false,
        example: "remote-agy://devhost/00000000-0000-4000-8000-000000000011",
    },
    SchemeDescriptor {
        prefix: "agy-runtime://",
        role: SchemeRole::RuntimeKey,
        locality: SchemeLocality::Remote,
        kind: Some(SessionKind::Antigravity),
        agent: true,
        legacy: false,
        example: "agy-runtime://00000000-0000-4000-8000-000000000031",
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

/// Current remote AGENT ROW schemes — `remote-session://` and `remote-cc://`.
///
/// Narrower than [`remote_agent_schemes`], which also yields the runtime keys.
/// This is the set every predicate that asks "is this row a remote agent
/// session" must cover: readiness/overlay, the scanned-row classification and
/// the cold-launch discriminator (harness spec §7.3). Each of those hand-listed
/// `remote-session://` alone and therefore skipped remote Claude Code entirely;
/// deriving from here means registering a future CLI's remote scheme covers all
/// of them at once, which is the whole point of the registry.
pub fn remote_agent_row_schemes() -> impl Iterator<Item = &'static SchemeDescriptor> {
    SESSION_PATH_SCHEMES.iter().filter(|scheme| {
        scheme.agent
            && !scheme.legacy
            && scheme.locality == SchemeLocality::Remote
            && matches!(
                scheme.role,
                SchemeRole::RowIdentity | SchemeRole::RowAndRuntimeKey
            )
    })
}

/// Current REMOTE row schemes, agent or not — the two above plus `ssh://`.
///
/// The set for "does this path name a runtime that lives on another machine",
/// which is a question about locality and says nothing about which CLI (or
/// whether there is a CLI at all).
pub fn remote_row_schemes() -> impl Iterator<Item = &'static SchemeDescriptor> {
    SESSION_PATH_SCHEMES.iter().filter(|scheme| {
        !scheme.legacy
            && scheme.locality == SchemeLocality::Remote
            && matches!(
                scheme.role,
                SchemeRole::RowIdentity | SchemeRole::RowAndRuntimeKey
            )
    })
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
        // ⚠ The kind list and the local-only answer are both DERIVED now. They
        // used to be hand-written here (`[Codex, CodexLiteLlm, ClaudeCode]` and
        // `matches!(kind, CodexLiteLlm)`), which is a lock that silently stops
        // covering the thing it guards the moment a CLI is added — the exact
        // stale-green shape the burn-down contract exists to forbid.
        for descriptor in crate::agent_cli::AGENT_CLIS {
            let kind = descriptor.kind;
            let has_remote_row = SESSION_PATH_SCHEMES.iter().any(|scheme| {
                scheme.kind == Some(kind)
                    && !scheme.legacy
                    && scheme.locality == SchemeLocality::Remote
                    && matches!(scheme.role, SchemeRole::RowIdentity)
            });
            assert_eq!(
                has_remote_row,
                descriptor.has_remote_arm(),
                "{kind:?}: the scheme table and the descriptor disagree about \
                 whether this CLI has a remote arm"
            );
        }
    }

    /// BOTH DIRECTIONS between the descriptor registry and the scheme table.
    ///
    /// A CLI that declares `remote_row_scheme`/`runtime_key_scheme` and forgets
    /// the table row fails here; so does a table row for a CLI whose descriptor
    /// never named it. Without this, the two are a copy of each other and the
    /// copy is what rots — which is how `cc-runtime://` came to be missing from
    /// seven predicates at once.
    #[test]
    fn every_agent_descriptor_scheme_is_registered_and_vice_versa() {
        for descriptor in crate::agent_cli::AGENT_CLIS {
            for (declared, role, what) in [
                (
                    descriptor.remote_row_scheme,
                    SchemeRole::RowIdentity,
                    "remote_row_scheme",
                ),
                (
                    descriptor.runtime_key_scheme,
                    SchemeRole::RuntimeKey,
                    "runtime_key_scheme",
                ),
            ] {
                let Some(prefix) = declared else {
                    continue;
                };
                let registered = scheme_for_prefix(prefix).unwrap_or_else(|| {
                    panic!(
                        "{:?} declares {what} {prefix:?} but no row in \
                         SESSION_PATH_SCHEMES carries that prefix",
                        descriptor.kind
                    )
                });
                assert_eq!(registered.kind, Some(descriptor.kind), "{prefix}: kind");
                assert_eq!(registered.role, role, "{prefix}: role");
                assert!(registered.agent && !registered.legacy, "{prefix}");
                assert_eq!(
                    registered.locality,
                    SchemeLocality::Remote,
                    "{prefix}: a wrapper-slug scheme is a remote scheme"
                );
            }
        }

        for scheme in SESSION_PATH_SCHEMES {
            if !scheme.agent || scheme.legacy || scheme.locality != SchemeLocality::Remote {
                continue;
            }
            let Some(kind) = scheme.kind else { continue };
            let descriptor = crate::agent_cli::agent_cli_descriptor(kind)
                .unwrap_or_else(|| panic!("{:?} has no descriptor", kind));
            let declared = match scheme.role {
                SchemeRole::RowIdentity => descriptor.remote_row_scheme,
                SchemeRole::RuntimeKey => descriptor.runtime_key_scheme,
                SchemeRole::RowAndRuntimeKey => continue,
            };
            assert_eq!(
                declared,
                Some(scheme.prefix),
                "{} is registered for {:?} but its descriptor names {:?}",
                scheme.prefix,
                kind,
                declared
            );
        }
    }

    /// The wrapper subcommands are DERIVED, and no two CLIs may collide on one.
    ///
    /// `resume-codex` and `resume-cc` are historical spellings kept on their
    /// descriptors; every new CLI's verb falls out of its `wrapper_slug`. A
    /// collision would route one CLI's resume into another's handler across the
    /// ssh hop, which is silent and unrecoverable.
    #[test]
    fn wrapper_subcommands_are_derived_and_unique() {
        let mut seen: Vec<String> = Vec::new();
        for descriptor in crate::agent_cli::AGENT_CLIS {
            let Some(slug) = descriptor.wrapper_slug else {
                assert!(
                    descriptor.remote_row_scheme.is_none()
                        && descriptor.runtime_key_scheme.is_none(),
                    "{:?} is local-only but declares a remote scheme",
                    descriptor.kind
                );
                continue;
            };
            assert!(
                !slug.is_empty() && slug.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{:?}: wrapper slug {slug:?} must be lowercase ascii",
                descriptor.kind
            );
            for verb in [
                descriptor.resume_subcommand(),
                descriptor.start_subcommand(),
                descriptor.terminate_subcommand(),
                descriptor.session_exists_subcommand(),
            ]
            .into_iter()
            .flatten()
            {
                assert!(!seen.contains(&verb), "two CLIs both claim {verb:?}");
                seen.push(verb);
            }
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
        // ⚠ DERIVED, not transcribed. This assertion used to hold a copy of the
        // four shipped prefixes, so it stopped describing the registry the
        // moment a CLI was added — and a lock that passes while covering less
        // than it claims is worse than no lock.
        let remotes: Vec<_> = remote_agent_schemes().map(|s| s.prefix).collect();
        let expected: Vec<&str> = crate::agent_cli::AGENT_CLIS
            .iter()
            .filter_map(|d| d.remote_row_scheme)
            .chain(
                crate::agent_cli::AGENT_CLIS
                    .iter()
                    .filter_map(|d| d.runtime_key_scheme),
            )
            .collect();
        let mut sorted_remotes = remotes.clone();
        sorted_remotes.sort_unstable();
        let mut sorted_expected = expected.clone();
        sorted_expected.sort_unstable();
        assert_eq!(sorted_remotes, sorted_expected);
        assert!(remotes.contains(&"remote-session://"));
        assert!(remotes.contains(&"cc-runtime://"));
    }
}
