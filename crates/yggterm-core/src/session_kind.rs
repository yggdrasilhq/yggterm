use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Codex,
    CodexLiteLlm,
    ClaudeCode,
    Shell,
    SshShell,
    Document,
}

impl SessionKind {
    /// Whether this kind is a first-class agent CLI.
    ///
    /// DERIVED from the descriptor registry (harness spec §3): adding a CLI
    /// without registering a descriptor is impossible by construction, and the
    /// old hand-listed `matches!` — which every new CLI had to remember to
    /// update, in every predicate that had its own copy — cannot drift from it.
    pub fn is_agent(self) -> bool {
        crate::agent_cli::AGENT_CLIS
            .iter()
            .any(|descriptor| descriptor.kind == self)
    }

    pub fn self_generates_copy(self) -> bool {
        matches!(self, SessionKind::ClaudeCode)
    }
}
