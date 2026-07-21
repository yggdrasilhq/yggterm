//! Agent presence — who else is driving this window right now.
//!
//! Cursor v1 of the agent control plane (`docs/agent-control-plane.md`, slice
//! 3). The settled rule, and the whole of v1:
//!
//! > When an agent is working a session **and the user is viewing that same
//! > session's viewport**, the user sees that agent's colored pointer tagged
//! > `agent-N`.
//!
//! No co-presence toggle, no ghost-cursor mimicry, no visibility modes.
//! Multiple agents means multiple glyphs with distinct colors and tags.
//!
//! This module owns the identity→(index, color) assignment so the GUI overlay
//! and any future surface agree on which agent is `agent-2` and what colour it
//! wears.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Distinct, high-contrast hues for `agent-1`..`agent-8`, chosen to read against
/// both themes and to stay distinguishable from the terminal's own ANSI palette
/// and from the click grid's orange. Agents past the eighth wrap around; the tag
/// still disambiguates them.
pub const AGENT_CURSOR_COLORS: [&str; 8] = [
    "#4fc3f7", // cyan
    "#f06292", // pink
    "#aed581", // lime
    "#ba68c8", // violet
    "#ffd54f", // amber
    "#4db6ac", // teal
    "#ff8a65", // coral
    "#9fa8da", // periwinkle
];

/// How long an agent's pointer stays on screen after its last action. The
/// overlay renders the fade as a one-shot CSS animation, so an idle window pays
/// nothing once it has elapsed.
pub const AGENT_CURSOR_TTL_MS: u64 = 8_000;

/// Where one agent last acted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentPointer {
    /// Stable 1-based display index — the `N` in `agent-N`.
    pub index: u32,
    /// The session whose viewport the agent acted on. The overlay draws this
    /// pointer only while the user is looking at the same session.
    pub session_path: Option<String>,
    /// Window/CSS pixels, the same space the click verbs take.
    pub x: f64,
    pub y: f64,
    /// Millis when the agent last acted, for the TTL.
    pub updated_ms: u64,
    /// What the agent did, shown next to the tag ("click", "move", …).
    pub action: String,
}

impl AgentPointer {
    /// The `agent-N` tag the user sees.
    pub fn tag(&self) -> String {
        format!("agent-{}", self.index)
    }

    /// This agent's colour, wrapping past the palette's end.
    pub fn color(&self) -> &'static str {
        AGENT_CURSOR_COLORS[(self.index.saturating_sub(1) as usize) % AGENT_CURSOR_COLORS.len()]
    }

    /// Is this pointer still within its TTL at `now_ms`?
    pub fn is_live(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.updated_ms) < AGENT_CURSOR_TTL_MS
    }
}

/// Every agent's last known pointer, keyed by agent identity.
///
/// Identity comes from the app-control request (`--agent` / `YGGTERM_AGENT`);
/// requests that carry none share the single key `agent`, which is honest — the
/// window genuinely cannot tell two anonymous drivers apart.
#[derive(Debug, Clone, Default)]
pub struct AgentPresence {
    pointers: BTreeMap<String, AgentPointer>,
    /// Insertion order of identities, so an agent keeps its index (and colour)
    /// for the life of the window instead of shuffling as the map re-sorts.
    order: Vec<String>,
}

/// The identity used when an app-control request names no agent.
pub const ANONYMOUS_AGENT: &str = "agent";

impl AgentPresence {
    /// Record an action. Assigns `identity` its stable index on first sight.
    pub fn record(
        &mut self,
        identity: Option<&str>,
        session_path: Option<String>,
        x: f64,
        y: f64,
        action: &str,
        now_ms: u64,
    ) {
        let key = identity
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(ANONYMOUS_AGENT)
            .to_string();
        let index = match self.order.iter().position(|known| *known == key) {
            Some(position) => position as u32 + 1,
            None => {
                self.order.push(key.clone());
                self.order.len() as u32
            }
        };
        self.pointers.insert(
            key,
            AgentPointer {
                index,
                session_path,
                x,
                y,
                updated_ms: now_ms,
                action: action.to_string(),
            },
        );
    }

    /// The pointers to draw over `session_path`'s viewport right now: agents
    /// working THIS session, still inside the TTL. Ordered by index so the
    /// overlay's DOM order is stable across renders.
    pub fn visible_for(&self, session_path: Option<&str>, now_ms: u64) -> Vec<AgentPointer> {
        let mut visible: Vec<AgentPointer> = self
            .pointers
            .values()
            .filter(|pointer| pointer.is_live(now_ms))
            .filter(|pointer| pointer.session_path.as_deref() == session_path)
            .cloned()
            .collect();
        visible.sort_by_key(|pointer| pointer.index);
        visible
    }

    /// Every live pointer regardless of session — for `app state`, so an agent
    /// can see its own presence without guessing what the user is viewing.
    pub fn live(&self, now_ms: u64) -> Vec<(String, AgentPointer)> {
        let mut live: Vec<(String, AgentPointer)> = self
            .pointers
            .iter()
            .filter(|(_, pointer)| pointer.is_live(now_ms))
            .map(|(key, pointer)| (key.clone(), pointer.clone()))
            .collect();
        live.sort_by_key(|(_, pointer)| pointer.index);
        live
    }

    /// Drop pointers past their TTL. Keeps `order` intact so an agent that comes
    /// back keeps the colour the user already associates with it.
    pub fn prune(&mut self, now_ms: u64) {
        self.pointers.retain(|_, pointer| pointer.is_live(now_ms));
    }

    pub fn is_empty(&self) -> bool {
        self.pointers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;

    #[test]
    fn identities_keep_a_stable_index_and_colour() {
        let mut presence = AgentPresence::default();
        presence.record(Some("codex"), None, 1.0, 1.0, "move", T0);
        presence.record(Some("claude"), None, 2.0, 2.0, "move", T0);
        presence.record(Some("codex"), None, 3.0, 3.0, "click", T0 + 10);
        let live = presence.live(T0 + 10);
        assert_eq!(live.len(), 2);
        let codex = live.iter().find(|(key, _)| key == "codex").unwrap();
        let claude = live.iter().find(|(key, _)| key == "claude").unwrap();
        assert_eq!(codex.1.tag(), "agent-1");
        assert_eq!(claude.1.tag(), "agent-2");
        assert_ne!(codex.1.color(), claude.1.color());
        // The later action moved the pointer but not the identity.
        assert_eq!((codex.1.x, codex.1.y), (3.0, 3.0));
        assert_eq!(codex.1.action, "click");
    }

    #[test]
    fn an_agent_that_returns_after_its_ttl_keeps_its_original_colour() {
        let mut presence = AgentPresence::default();
        presence.record(Some("first"), None, 0.0, 0.0, "move", T0);
        presence.record(Some("second"), None, 0.0, 0.0, "move", T0);
        let late = T0 + AGENT_CURSOR_TTL_MS + 1;
        presence.prune(late);
        assert!(presence.is_empty());
        presence.record(Some("first"), None, 5.0, 5.0, "click", late);
        assert_eq!(presence.live(late)[0].1.tag(), "agent-1");
    }

    #[test]
    fn unnamed_requests_share_one_anonymous_identity() {
        let mut presence = AgentPresence::default();
        presence.record(None, None, 1.0, 1.0, "move", T0);
        presence.record(Some("   "), None, 2.0, 2.0, "click", T0);
        let live = presence.live(T0);
        assert_eq!(
            live.len(),
            1,
            "blank and absent identities are the same agent"
        );
        assert_eq!(live[0].0, ANONYMOUS_AGENT);
        assert_eq!(live[0].1.x, 2.0);
    }

    #[test]
    fn a_pointer_is_visible_only_over_the_session_it_acted_on() {
        let mut presence = AgentPresence::default();
        presence.record(Some("a"), Some("session-x".into()), 10.0, 20.0, "click", T0);
        assert_eq!(presence.visible_for(Some("session-x"), T0).len(), 1);
        assert!(
            presence.visible_for(Some("session-y"), T0).is_empty(),
            "the user viewing another session sees nothing"
        );
        assert!(presence.visible_for(None, T0).is_empty());
    }

    #[test]
    fn a_pointer_disappears_once_its_ttl_elapses() {
        let mut presence = AgentPresence::default();
        presence.record(Some("a"), None, 1.0, 1.0, "click", T0);
        assert_eq!(
            presence
                .visible_for(None, T0 + AGENT_CURSOR_TTL_MS - 1)
                .len(),
            1
        );
        assert!(
            presence
                .visible_for(None, T0 + AGENT_CURSOR_TTL_MS)
                .is_empty()
        );
    }

    #[test]
    fn visible_pointers_are_ordered_by_index_not_map_order() {
        let mut presence = AgentPresence::default();
        // "zz" is recorded first, so it is agent-1 despite sorting last by key.
        presence.record(Some("zz"), None, 0.0, 0.0, "move", T0);
        presence.record(Some("aa"), None, 0.0, 0.0, "move", T0);
        let visible = presence.visible_for(None, T0);
        assert_eq!(
            visible.iter().map(|p| p.index).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn colours_wrap_past_the_palette_without_panicking() {
        let mut presence = AgentPresence::default();
        for index in 0..(AGENT_CURSOR_COLORS.len() + 2) {
            presence.record(Some(&format!("agent{index}")), None, 0.0, 0.0, "move", T0);
        }
        let live = presence.live(T0);
        assert_eq!(live.len(), AGENT_CURSOR_COLORS.len() + 2);
        assert_eq!(live[0].1.color(), live[AGENT_CURSOR_COLORS.len()].1.color());
    }
}
