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
    pub fn is_agent(self) -> bool {
        matches!(
            self,
            SessionKind::Codex | SessionKind::CodexLiteLlm | SessionKind::ClaudeCode
        )
    }

    pub fn self_generates_copy(self) -> bool {
        matches!(self, SessionKind::ClaudeCode)
    }
}
