use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    ZedDark,
    ZedLight,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct YgguiThemeColorStop {
    pub color: String,
    pub x: f32,
    pub y: f32,
    pub alpha: f32,
}

impl Default for YgguiThemeColorStop {
    fn default() -> Self {
        Self {
            color: "#7cc8ff".to_string(),
            x: 0.5,
            y: 0.5,
            alpha: 0.82,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct YgguiThemeSpec {
    pub colors: Vec<YgguiThemeColorStop>,
    pub brightness: f32,
    pub alpha: f32,
    pub grain: f32,
}

impl Default for YgguiThemeSpec {
    fn default() -> Self {
        Self {
            colors: Vec::new(),
            brightness: 0.56,
            alpha: 0.78,
            grain: 0.12,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum YgguiClipboardContents {
    Text { text: String },
    PngBase64 { png_base64: String },
}

/// A physical window edge a piece of chrome sits against.
///
/// **Physical, never logical.** `Left` is the left of the *screen*, always —
/// it does not mean "the tree" and it never will. Which panel lands on which
/// edge is [`ChromeOrientation`]'s answer and nobody else's, which is the whole
/// reason this enum refuses to carry a panel's name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SidebarEdge {
    Left,
    Right,
}

impl SidebarEdge {
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// The CSS property that names this edge — `left:` / `right:`.
    pub const fn css_near(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// The CSS property that names the OPPOSITE edge. Emitting both keys in
    /// every branch is what keeps a mirrored style from leaving the previous
    /// orientation's anchor behind (see `SidebarPanelMode` on the style-key
    /// trap).
    pub const fn css_far(self) -> &'static str {
        self.opposite().css_near()
    }

    /// The flexbox alignment that pushes content toward this edge.
    pub const fn css_justify(self) -> &'static str {
        match self {
            Self::Left => "flex-start",
            Self::Right => "flex-end",
        }
    }

    /// Which way a panel on this edge slides when it collapses: toward its own
    /// edge, so a reveal is a slide + fade rather than a pop.
    pub const fn collapse_translate_sign(self) -> f64 {
        match self {
            Self::Left => -1.0,
            Self::Right => 1.0,
        }
    }

    /// Sign to apply to a resize drag's x delta so the panel gets WIDER.
    ///
    /// A panel's resize grip always lives on its INNER edge — the one facing
    /// the workspace — so a left-edge panel widens as the pointer moves right
    /// (`+1`) and a right-edge panel widens as it moves left (`-1`). This is
    /// the sign that used to be hard-coded once per panel; it belongs to the
    /// edge, which is why a mirror flips it for free.
    pub const fn resize_delta_sign(self) -> f32 {
        match self {
            Self::Left => 1.0,
            Self::Right => -1.0,
        }
    }
}

/// A piece of app chrome named by WHAT IT IS, never by where it sits.
///
/// The two slots are the two halves of the titlebar either side of the search
/// box, plus the panel each half drives. The search box itself is not a slot:
/// it is the mirror's axis and never moves.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ChromeSlot {
    /// The cwd tree sidebar, its titlebar `☰` toggle, the two-phase Web
    /// View/Terminal toggle, the `+` menu and the session chip — everything on
    /// the tree's side of the search box.
    Tree,
    /// The metadata / settings / notifications / app-pane rail and every
    /// titlebar button that opens one — everything on the rail's side of the
    /// search box.
    Rail,
}

/// **THE single answer to "which side is this chrome on?"**
///
/// Nothing else in the app may decide a side. Callers ask this type a question
/// (`orientation.edge(ChromeSlot::Tree)`) instead of re-testing a boolean, so
/// there is exactly one place that knows what the mirror means — the rule
/// AGENTS.md states as "name the source of truth for the thing you are
/// changing; if two places could answer the same question, collapse them".
///
/// Serialized as `{"mirrored": bool}` and persisted in `AppSettings`; a
/// settings file written before the mirror existed deserializes to
/// [`ChromeOrientation::natural`] via the struct-level `#[serde(default)]`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(default)]
pub struct ChromeOrientation {
    mirrored: bool,
}

impl Default for ChromeOrientation {
    fn default() -> Self {
        Self::natural()
    }
}

impl ChromeOrientation {
    /// Tree on the left, rail on the right — the shipped default.
    pub const fn natural() -> Self {
        Self { mirrored: false }
    }

    /// Reflected about the window's vertical centre line: tree on the right,
    /// rail on the left.
    pub const fn mirrored() -> Self {
        Self { mirrored: true }
    }

    pub const fn new(mirrored: bool) -> Self {
        Self { mirrored }
    }

    pub const fn is_mirrored(self) -> bool {
        self.mirrored
    }

    /// The physical edge `slot` sits against.
    pub const fn edge(self, slot: ChromeSlot) -> SidebarEdge {
        let natural = match slot {
            ChromeSlot::Tree => SidebarEdge::Left,
            ChromeSlot::Rail => SidebarEdge::Right,
        };
        if self.mirrored {
            natural.opposite()
        } else {
            natural
        }
    }

    /// The slot sitting against `edge` — the exact inverse of [`Self::edge`].
    /// Used where the input is a PHYSICAL edge the user reached for (a window
    /// edge the compositor reported, a page-edge motion the web engine saw) and
    /// the shell has to name the panel it belongs to.
    pub const fn slot(self, edge: SidebarEdge) -> ChromeSlot {
        let natural = match edge {
            SidebarEdge::Left => ChromeSlot::Tree,
            SidebarEdge::Right => ChromeSlot::Rail,
        };
        if self.mirrored {
            match natural {
                ChromeSlot::Tree => ChromeSlot::Rail,
                ChromeSlot::Rail => ChromeSlot::Tree,
            }
        } else {
            natural
        }
    }
}

#[cfg(test)]
mod chrome_orientation_tests {
    use super::*;

    #[test]
    fn natural_puts_the_tree_left_and_the_rail_right() {
        let o = ChromeOrientation::natural();
        assert_eq!(o.edge(ChromeSlot::Tree), SidebarEdge::Left);
        assert_eq!(o.edge(ChromeSlot::Rail), SidebarEdge::Right);
    }

    #[test]
    fn mirrored_swaps_both_slots_not_just_one() {
        let o = ChromeOrientation::mirrored();
        assert_eq!(o.edge(ChromeSlot::Tree), SidebarEdge::Right);
        assert_eq!(o.edge(ChromeSlot::Rail), SidebarEdge::Left);
    }

    #[test]
    fn slot_is_the_inverse_of_edge_in_both_orientations() {
        for orientation in [ChromeOrientation::natural(), ChromeOrientation::mirrored()] {
            for slot in [ChromeSlot::Tree, ChromeSlot::Rail] {
                assert_eq!(orientation.slot(orientation.edge(slot)), slot);
            }
            for edge in [SidebarEdge::Left, SidebarEdge::Right] {
                assert_eq!(orientation.edge(orientation.slot(edge)), edge);
            }
        }
    }

    #[test]
    fn a_resize_grip_always_widens_away_from_its_own_edge() {
        assert_eq!(SidebarEdge::Left.resize_delta_sign(), 1.0);
        assert_eq!(SidebarEdge::Right.resize_delta_sign(), -1.0);
        // Mirroring flips the sign for BOTH panels, because the sign belongs to
        // the edge and not to the panel.
        let mirrored = ChromeOrientation::mirrored();
        assert_eq!(mirrored.edge(ChromeSlot::Tree).resize_delta_sign(), -1.0);
        assert_eq!(mirrored.edge(ChromeSlot::Rail).resize_delta_sign(), 1.0);
    }

    #[test]
    fn a_settings_file_without_the_field_reads_as_natural() {
        let decoded: ChromeOrientation = serde_json::from_str("{}").expect("empty object");
        assert_eq!(decoded, ChromeOrientation::natural());
        assert!(!decoded.is_mirrored());
    }

    #[test]
    fn css_helpers_name_opposite_properties() {
        assert_eq!(SidebarEdge::Left.css_near(), "left");
        assert_eq!(SidebarEdge::Left.css_far(), "right");
        assert_eq!(SidebarEdge::Right.css_near(), "right");
        assert_eq!(SidebarEdge::Right.css_far(), "left");
        assert_eq!(SidebarEdge::Left.css_justify(), "flex-start");
        assert_eq!(SidebarEdge::Right.css_justify(), "flex-end");
    }
}
