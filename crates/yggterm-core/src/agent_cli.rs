//! `AgentCliDescriptor` — the per-CLI data (harness spec §3, migration phase 1).
//!
//! One value per agent CLI, compile-time constructed, owned by the same crate
//! that owns [`SessionKind`], so every crate reads the same answers about a CLI
//! instead of re-deciding per call site.
//!
//! **Why this exists.** Before it, "how do I resume this CLI?" was answered by
//! an `is_claude` boolean inside the launch-command builder, and the same
//! question was re-answered — differently — by the readiness, replay and
//! scanner paths (`docs/spec-agent-cli-harness.md` §7 inventories the forks).
//! A fork like that is invisible until a CLI hits the arm nobody updated, which
//! is exactly how the remote-cc predicate holes were born.
//!
//! **Scope of this slice (phase 1a):** invocation shape only — the data behind
//! `resume_argv` / `launch_argv`. Store scanning (`read_store_entry`,
//! `session_store_globs`), working-signal source and prompt hints are the
//! remainder of phase 1 and are deliberately NOT stubbed here: an unused field
//! that nobody reads is a second source of truth waiting to drift.

use crate::session_kind::SessionKind;

/// How a CLI names an existing session on its resume invocation.
///
/// The two shapes in the fleet today. A new CLI picks one; if a third exists it
/// is added HERE, which is what makes the launch builder's job mechanical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeSelector {
    /// `claude --resume <id>` — a flag on the bare command.
    Flag(&'static str),
    /// `codex resume <id>` — a subcommand.
    Subcommand(&'static str),
}

/// One agent CLI's invocation contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentCliDescriptor {
    /// The enum key this descriptor serves.
    pub kind: SessionKind,
    /// Human name for UI ("Codex", "Claude Code", …).
    pub display_name: &'static str,
    /// The executable, as invoked on the session's host.
    pub binary_name: &'static str,
    /// How an existing session id is named on resume.
    pub resume_selector: ResumeSelector,
    /// Whether resuming into a known cwd passes it explicitly.
    ///
    /// Codex is re-rooted with `-C "$PWD"` because `codex resume` otherwise
    /// resolves the session's ORIGINAL directory; Claude Code takes the
    /// process cwd. This is a real per-CLI divergence, so it is data — it used
    /// to be an `is_claude`/`has_cwd` branch pair in the builder.
    pub resume_re_roots_with_cwd: bool,
    /// True when the CLI re-derives full content from its own store on resume
    /// (all shipped CLIs do). Drives replay policy §5.4: re-derivable ⇒ the PTY
    /// is disposable and rows ride every persist.
    pub content_rederives_on_resume: bool,
}

impl AgentCliDescriptor {
    /// Tokens for resuming `session_id`, WITHOUT the binary and without
    /// transport/env wrapping — the harness owns those (spec §3).
    ///
    /// `cwd_known` reports whether the caller established a working directory
    /// for the session; only a re-rooting CLI uses it.
    pub fn resume_tokens(&self, session_id_quoted: &str, cwd_known: bool) -> Vec<String> {
        let mut tokens = Vec::new();
        match self.resume_selector {
            ResumeSelector::Flag(flag) => tokens.push(flag.to_string()),
            ResumeSelector::Subcommand(sub) => tokens.push(sub.to_string()),
        }
        if self.resume_re_roots_with_cwd && cwd_known {
            tokens.push("-C".to_string());
            tokens.push("\"$PWD\"".to_string());
        }
        tokens.push(session_id_quoted.to_string());
        tokens
    }

    /// Tokens for the CLI's own resume PICKER (no session id).
    pub fn resume_picker_tokens(&self) -> Vec<String> {
        match self.resume_selector {
            ResumeSelector::Flag(flag) => vec![flag.to_string()],
            ResumeSelector::Subcommand(sub) => vec![sub.to_string()],
        }
    }
}

/// Every agent CLI yggterm can drive. Adding a CLI without a descriptor is
/// impossible by construction: [`SessionKind::is_agent`] is derived from this
/// table (see `session_kind.rs`).
pub const AGENT_CLIS: &[AgentCliDescriptor] = &[
    AgentCliDescriptor {
        kind: SessionKind::Codex,
        display_name: "Codex",
        binary_name: "codex",
        resume_selector: ResumeSelector::Subcommand("resume"),
        // `codex resume <id>` reopens the session's ORIGINAL cwd unless
        // re-rooted; the cwd tree's whole promise is that a row opens where the
        // tree says it lives.
        resume_re_roots_with_cwd: true,
        content_rederives_on_resume: true,
    },
    AgentCliDescriptor {
        kind: SessionKind::CodexLiteLlm,
        display_name: "Codex-LiteLLM",
        binary_name: "codex-litellm",
        resume_selector: ResumeSelector::Subcommand("resume"),
        // ⚠ Deliberately FALSE, preserving shipped behavior exactly: the
        // pre-descriptor builder gated `-C "$PWD"` on `SessionKind::Codex`
        // alone, so the LiteLLM fork never re-rooted. Whether that was intent
        // or oversight is unverified, and phase 1 is a refactor — flipping it
        // here would be a silent behavior change riding a "no wire changes"
        // phase. Recorded for phase 2's four-arm matrix to settle.
        resume_re_roots_with_cwd: false,
        content_rederives_on_resume: true,
    },
    AgentCliDescriptor {
        kind: SessionKind::ClaudeCode,
        display_name: "Claude Code",
        binary_name: "claude",
        resume_selector: ResumeSelector::Flag("--resume"),
        resume_re_roots_with_cwd: false,
        content_rederives_on_resume: true,
    },
];

/// The descriptor for `kind`, or `None` for a non-agent kind.
pub fn agent_cli_descriptor(kind: SessionKind) -> Option<&'static AgentCliDescriptor> {
    AGENT_CLIS.iter().find(|descriptor| descriptor.kind == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The registry is the SSOT for "is this an agent CLI". If these disagree,
    // a CLI can be an agent to one predicate and not to another — the fork
    // class this whole spec exists to kill.
    #[test]
    fn every_agent_kind_has_a_descriptor_and_vice_versa() {
        for kind in [
            SessionKind::Codex,
            SessionKind::CodexLiteLlm,
            SessionKind::ClaudeCode,
            SessionKind::Shell,
            SessionKind::SshShell,
            SessionKind::Document,
        ] {
            assert_eq!(
                agent_cli_descriptor(kind).is_some(),
                kind.is_agent(),
                "{kind:?}: descriptor presence must equal is_agent()"
            );
        }
    }

    #[test]
    fn descriptors_are_unique_per_kind_and_name_a_binary() {
        let mut kinds: Vec<SessionKind> = AGENT_CLIS.iter().map(|d| d.kind).collect();
        let before = kinds.len();
        kinds.sort_by_key(|kind| format!("{kind:?}"));
        kinds.dedup();
        assert_eq!(before, kinds.len(), "one descriptor per kind");
        for descriptor in AGENT_CLIS {
            assert!(!descriptor.binary_name.is_empty());
            assert!(!descriptor.display_name.is_empty());
        }
    }

    // These token shapes ARE the shipped invocations. They are asserted here so
    // the launch builder can be rewritten to consult the descriptor without
    // anyone having to re-derive what codex vs claude expect.
    #[test]
    fn resume_tokens_match_each_cli_shipped_invocation() {
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert_eq!(
            codex.resume_tokens("'abc'", true),
            vec!["resume", "-C", "\"$PWD\"", "'abc'"]
        );
        // No cwd established ⇒ no re-root, even for a re-rooting CLI.
        assert_eq!(codex.resume_tokens("'abc'", false), vec!["resume", "'abc'"]);

        let claude = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert_eq!(claude.resume_tokens("'abc'", true), vec!["--resume", "'abc'"]);

        // Behavior-preserving: the LiteLLM fork never re-rooted pre-descriptor.
        let litellm = agent_cli_descriptor(SessionKind::CodexLiteLlm).unwrap();
        assert_eq!(litellm.resume_tokens("'abc'", true), vec!["resume", "'abc'"]);
    }

    #[test]
    fn resume_picker_tokens_carry_no_session_id() {
        assert_eq!(
            agent_cli_descriptor(SessionKind::Codex)
                .unwrap()
                .resume_picker_tokens(),
            vec!["resume"]
        );
        assert_eq!(
            agent_cli_descriptor(SessionKind::ClaudeCode)
                .unwrap()
                .resume_picker_tokens(),
            vec!["--resume"]
        );
    }
}
