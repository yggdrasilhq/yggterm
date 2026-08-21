use serde::{Deserialize, Serialize};

// `Hash` so a kind can key a map directly. Without it, callers that need to
// group by CLI reach for the slug string instead — a second spelling of an
// identity the enum already owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Codex,
    CodexLiteLlm,
    ClaudeCode,
    /// `pi` — earendil-works/pi, MIT.
    Pi,
    /// `opencode` — sst/opencode, MIT.
    OpenCode,
    /// `qwen` — QwenLM/qwen-code, Apache-2.0.
    QwenCode,
    /// `kimi` — MoonshotAI/kimi-cli, Apache-2.0.
    Kimi,
    /// `muse` — Meta's Muse Code, closed source.
    Muse,
    /// `agy` — Google Antigravity's CLI, closed source.
    Antigravity,
    /// `grok` — xAI's Grok Build CLI, `@xai-official/grok`, Apache-2.0.
    GrokBuild,
    Shell,
    SshShell,
    Document,
}

impl SessionKind {
    /// Every kind, once — so a rule that must answer for all of them can be
    /// TESTED against all of them rather than against the ones somebody
    /// remembered. `all_holds_every_variant_exactly_once` is what keeps it
    /// honest when a variant is added.
    pub const ALL: &'static [SessionKind] = &[
        SessionKind::Codex,
        SessionKind::CodexLiteLlm,
        SessionKind::ClaudeCode,
        SessionKind::Pi,
        SessionKind::OpenCode,
        SessionKind::QwenCode,
        SessionKind::Kimi,
        SessionKind::Muse,
        SessionKind::Antigravity,
        SessionKind::GrokBuild,
        SessionKind::Shell,
        SessionKind::SshShell,
        SessionKind::Document,
    ];

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

    /// Whether this kind writes its own session title, so yggterm must RESPECT
    /// it rather than generate one.
    ///
    /// DERIVED from the descriptor's `title_authority`. It used to be
    /// `matches!(self, SessionKind::ClaudeCode)` — a hand-list that a second
    /// store-authoritative CLI falls silently out of, after which yggterm
    /// generates a title for a CLI that already wrote one and the two disagree
    /// forever.
    pub fn self_generates_copy(self) -> bool {
        crate::agent_cli::agent_cli_descriptor(self)
            .is_some_and(|descriptor| descriptor.title_is_store_authoritative())
    }

    /// Whether a session of this kind has a RENDERED (non-terminal) view of its
    /// own — the surface the titlebar's "Web View" half selects.
    ///
    /// - An agent CLI does: yggterm pretty-renders the CLI's own JSONL.
    /// - A `Document` does: it IS the rendered thing (yedit's paper).
    /// - **A plain shell does NOT.** It has a PTY and nothing else to show.
    ///
    /// A shell that hosts a libyggterm app is the one nuance, and it is not this
    /// function's to answer: the APP owns that surface, so the caller asks
    /// whether a viewport pane has been declared for the specific session
    /// (`viewport_pane_for_session`). Kind alone cannot know.
    ///
    /// This exists because the question had three answers. The titlebar toggle
    /// asked `active_session.kind.is_agent()`, the open-routing asked nothing at
    /// all and happily put a shell into `Rendered`, and the conversation-provider
    /// table answered a fourth way by handing shells a "Terminal transcript"
    /// reader. That last one is why opening a yedit session showed a terminal
    /// transcript captioned "web view" — a surface that should never have been
    /// reachable for it (user report, 2026-07-25).
    pub fn offers_rendered_view(self) -> bool {
        self.is_agent() || matches!(self, SessionKind::Document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `match` is the enforcement: a new variant cannot be added without an
    /// arm here, and the arm cannot be written without giving the variant a slot
    /// in `ALL` — at which point the length assertion is the reminder to bump it.
    #[test]
    fn all_holds_every_variant_exactly_once() {
        for (index, kind) in SessionKind::ALL.iter().enumerate() {
            let slot = match kind {
                SessionKind::Codex => 0,
                SessionKind::CodexLiteLlm => 1,
                SessionKind::ClaudeCode => 2,
                SessionKind::Pi => 3,
                SessionKind::OpenCode => 4,
                SessionKind::QwenCode => 5,
                SessionKind::Kimi => 6,
                SessionKind::Muse => 7,
                SessionKind::Antigravity => 8,
                SessionKind::GrokBuild => 9,
                SessionKind::Shell => 10,
                SessionKind::SshShell => 11,
                SessionKind::Document => 12,
            };
            assert_eq!(index, slot, "{kind:?} is listed out of order in ALL");
        }
        assert_eq!(SessionKind::ALL.len(), 13);
    }

    // The rendered-view question, answered once. Before this, a plain shell
    // could be routed into `Rendered` and was handed a "Terminal transcript"
    // reader captioned "web view" — reachable on a yedit session, which is a
    // shell hosting a document app.
    #[test]
    fn only_kinds_with_a_surface_of_their_own_offer_a_rendered_view() {
        assert!(SessionKind::Codex.offers_rendered_view());
        assert!(SessionKind::CodexLiteLlm.offers_rendered_view());
        assert!(SessionKind::ClaudeCode.offers_rendered_view());
        assert!(
            SessionKind::Document.offers_rendered_view(),
            "a document IS the rendered thing"
        );
        assert!(
            !SessionKind::Shell.offers_rendered_view(),
            "a plain shell has a PTY and nothing else — an app that declares a \
             viewport pane is the caller's question, not the kind's"
        );
        assert!(!SessionKind::SshShell.offers_rendered_view());
    }

    // Every agent CLI must offer one, by construction: the whole point of the
    // rendered view is pretty-printing the CLI's own JSONL, so a new CLI cannot
    // be added without it.
    #[test]
    fn every_agent_cli_offers_a_rendered_view() {
        for descriptor in crate::agent_cli::AGENT_CLIS {
            assert!(
                descriptor.kind.offers_rendered_view(),
                "{:?} is an agent CLI, so its transcript is a rendered view",
                descriptor.kind
            );
        }
    }
}
