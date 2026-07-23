//! The command registry — the single source of truth for yggterm's shell
//! commands and their ALT+ KeyTips accelerators.
//!
//! Per `[[campaign-alt-keytips-layer]]`: KeyTips, the keymap config file, the
//! ALT+ settings modal, and the `command invoke <id>` app-control probe are all
//! VIEWS of this registry. There is exactly one place that says "the sidebar
//! toggle is bound to `b`": here. The titlebar badge, the resolver, and the
//! editor read the letter from the registry rather than hardcoding it, so a
//! remap in one place moves everywhere.
//!
//! ## Reserved-letters namespace (spec decision a, FINALIZED 2026-07-10)
//!
//! yggterm shell chrome draws its top-level KeyTips ONLY from letters Excel's
//! top level does not use. Excel's top-level letters (F, H, N, P, M, A, R, W, X,
//! Y, Q) are RESERVED for app contributions so a focused Cellulose can be 100%
//! Excel-faithful while shell chrome stays reachable in one flat namespace.
//! [`reserved_letter`] enforces this; [`assert_shell_namespace_clean`] (a test)
//! fails the build if any shell default keytip lands on an Excel letter.

/// Excel's top-level ribbon KeyTip letters, reserved for app contributions.
/// A yggterm shell command must never claim one of these at the top level.
pub const EXCEL_RESERVED_LETTERS: &[char] =
    &['f', 'h', 'n', 'p', 'm', 'a', 'r', 'w', 'x', 'y', 'q'];

/// True if `letter` belongs to Excel's reserved top-level namespace.
pub fn reserved_letter(letter: char) -> bool {
    EXCEL_RESERVED_LETTERS.contains(&letter.to_ascii_lowercase())
}

/// A shell command. The variant is the stable identity; [`CommandSpec`] carries
/// its metadata. Actions live in `shell.rs` (they call `ShellState` methods);
/// this enum stays free of shell dependencies so it can be reasoned about and
/// unit-tested in isolation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellCommand {
    ToggleSidebar,
    ViewWeb,
    ViewTerminal,
    ToggleConnect,
    ToggleNotifications,
    ToggleSettings,
    ToggleMetadata,
    ToggleFullscreen,
    ToggleAlwaysOnTop,
    OpenInsertMenu,
    OpenRowMenu,
    JumpSession,
    InsertSession,
    InsertTerminal,
    NextSession,
    PrevSession,
    OpenKeymapEditor,
    FocusSearch,
}

/// Static metadata for one command.
pub struct CommandSpec {
    pub command: ShellCommand,
    /// Stable, dotted id — the key used by the keymap config, the settings
    /// modal, and `command invoke <id>`. Never changes once shipped.
    pub id: &'static str,
    /// Human label shown in the KeyTips legend and the settings modal.
    pub title: &'static str,
    /// The parent chord node, by id, or `None` for a top-level command. A child
    /// command's full chord is `parent.chord ++ self.keytip`.
    pub parent: Option<&'static str>,
    /// The default KeyTip letter (the Excel-familiar preset). `None` means the
    /// command has no ALT+ accelerator (it is reached by mouse or `command
    /// invoke`, e.g. the Ctrl+Alt+PgUp/PgDn session-nav pair).
    pub default_keytip: Option<char>,
    /// The scope this command DESCENDS INTO when its keytip is pressed: it acts
    /// (opens the container) and then shows that container's fresh KeyTips rather
    /// than dismissing (Excel's ALT,H,… nesting). `None` = act and dismiss.
    ///
    /// The value is a `ScopeId::as_str()` id (`"insert.menu"`, `"settings"`,
    /// `"rowmenu"`, `"session.jump"`) — the registry, not a hardcoded `match` in
    /// the renderer, says where a chord goes next.
    pub descends_into: Option<&'static str>,
}

/// The registry. Order is display order in the settings modal.
///
/// Every top-level `default_keytip` here is checked against
/// [`EXCEL_RESERVED_LETTERS`] by [`assert_shell_namespace_clean`].
pub const SHELL_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        command: ShellCommand::ToggleSidebar,
        id: "sidebar.toggle",
        title: "Toggle sidebar",
        parent: None,
        default_keytip: Some('b'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::ViewWeb,
        id: "view.web",
        title: "Web view",
        parent: None,
        default_keytip: Some('v'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::ViewTerminal,
        id: "view.terminal",
        title: "Terminal view",
        parent: None,
        default_keytip: Some('t'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::ToggleConnect,
        id: "connect.toggle",
        title: "Connect SSH",
        parent: None,
        default_keytip: Some('c'),
        descends_into: None,
    },
    CommandSpec {
        // Was `n` in the pre-spec draft — `n` is Excel's Insert tab, reserved.
        command: ShellCommand::ToggleNotifications,
        id: "notifications.toggle",
        title: "Notifications",
        parent: None,
        default_keytip: Some('l'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::ToggleSettings,
        id: "settings.toggle",
        title: "Settings",
        parent: None,
        default_keytip: Some('g'),
        descends_into: Some("settings"),
    },
    CommandSpec {
        // Was `m` in the pre-spec draft — `m` is Excel's Formulas tab, reserved.
        command: ShellCommand::ToggleMetadata,
        id: "metadata.toggle",
        title: "Session metadata",
        parent: None,
        default_keytip: Some('d'),
        descends_into: None,
    },
    CommandSpec {
        // Was `f` in the pre-spec draft — `f` is Excel's File tab, reserved.
        command: ShellCommand::ToggleFullscreen,
        id: "window.fullscreen",
        title: "Toggle fullscreen",
        parent: None,
        default_keytip: Some('u'),
        descends_into: None,
    },
    CommandSpec {
        // Was `a` in the pre-spec draft — `a` is Excel's Data tab, reserved.
        command: ShellCommand::ToggleAlwaysOnTop,
        id: "window.always-on-top",
        title: "Always on top",
        parent: None,
        default_keytip: Some('o'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::OpenKeymapEditor,
        id: "keymap.editor",
        title: "Edit ALT+ keys",
        parent: None,
        default_keytip: Some('k'),
        descends_into: None,
    },
    CommandSpec {
        // Replaces the bare "/" hotkey (user call 2026-07-23): a plain
        // printable key stole real typing whenever focus judgment misfired;
        // search focus belongs in the ALT+ layer like every other affordance.
        command: ShellCommand::FocusSearch,
        id: "search.focus",
        title: "Search",
        parent: None,
        default_keytip: Some('s'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::OpenInsertMenu,
        id: "insert.menu",
        title: "New… menu",
        parent: None,
        default_keytip: Some('i'),
        descends_into: Some("insert.menu"),
    },
    CommandSpec {
        command: ShellCommand::InsertSession,
        id: "insert.session",
        title: "New session",
        parent: Some("insert.menu"),
        default_keytip: Some('s'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::InsertTerminal,
        id: "insert.terminal",
        title: "New terminal",
        parent: Some("insert.menu"),
        default_keytip: Some('t'),
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::OpenRowMenu,
        id: "session.menu",
        title: "Row actions",
        parent: None,
        default_keytip: Some('e'),
        descends_into: Some("rowmenu"),
    },
    CommandSpec {
        command: ShellCommand::JumpSession,
        id: "session.jump",
        title: "Jump to session",
        parent: None,
        default_keytip: Some('j'),
        descends_into: Some("session.jump"),
    },
    CommandSpec {
        command: ShellCommand::NextSession,
        id: "session.next",
        title: "Next live session",
        parent: None,
        default_keytip: None,
        descends_into: None,
    },
    CommandSpec {
        command: ShellCommand::PrevSession,
        id: "session.prev",
        title: "Previous live session",
        parent: None,
        default_keytip: None,
        descends_into: None,
    },
];

/// Look up a command's spec by its stable id.
pub fn spec_for_id(id: &str) -> Option<&'static CommandSpec> {
    SHELL_COMMANDS.iter().find(|spec| spec.id == id)
}

/// Look up a command's spec by variant.
pub fn spec_for_command(command: ShellCommand) -> &'static CommandSpec {
    SHELL_COMMANDS
        .iter()
        .find(|spec| spec.command == command)
        .expect("every ShellCommand variant has a CommandSpec")
}

/// An effective keymap: the Excel-familiar defaults with the user's per-command
/// overrides applied. The registry is the SSOT for structure; a keymap is the
/// SSOT for the *letters* actually in force (defaults ∪ overrides).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    /// command id → the keytip letter in force for it (already lowercased).
    /// A command with no accelerator (default `None`, not overridden) is absent.
    overrides: std::collections::BTreeMap<String, char>,
}

impl Keymap {
    /// The pure-default keymap (no user overrides).
    pub fn defaults() -> Self {
        Self {
            overrides: std::collections::BTreeMap::new(),
        }
    }

    /// Build from a set of user overrides (command id → letter). Unknown ids and
    /// non-alphanumeric letters are ignored; a letter is lowercased.
    pub fn from_overrides<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = (S, char)>,
        S: Into<String>,
    {
        let mut overrides = std::collections::BTreeMap::new();
        for (id, letter) in entries {
            let id = id.into();
            let letter = letter.to_ascii_lowercase();
            if spec_for_id(&id).is_some() && letter.is_ascii_alphanumeric() {
                overrides.insert(id, letter);
            }
        }
        Self { overrides }
    }

    /// The user overrides in force (command id → letter), for persistence.
    pub fn overrides(&self) -> &std::collections::BTreeMap<String, char> {
        &self.overrides
    }

    /// True if `id` has a user override (not just the default).
    pub fn is_overridden(&self, id: &str) -> bool {
        self.overrides.contains_key(id)
    }

    /// The keytip letter in force for a command id (override, else default),
    /// or `None` when the command has no accelerator.
    pub fn keytip_for_id(&self, id: &str) -> Option<char> {
        if let Some(letter) = self.overrides.get(id) {
            return Some(*letter);
        }
        spec_for_id(id).and_then(|spec| spec.default_keytip)
    }

    /// The keytip letter in force for a command variant.
    pub fn keytip_for(&self, command: ShellCommand) -> Option<char> {
        self.keytip_for_id(spec_for_command(command).id)
    }

    /// The full chord string that reaches a command (e.g. `"is"` for New
    /// session, whose parent is the insert menu on `i`). `None` if the command
    /// or any ancestor lacks a keytip.
    pub fn chord_for_id(&self, id: &str) -> Option<String> {
        let spec = spec_for_id(id)?;
        let mut chord = String::new();
        if let Some(parent) = spec.parent {
            chord.push_str(&self.chord_for_id(parent)?);
        }
        chord.push(self.keytip_for_id(id)?);
        Some(chord)
    }

}

// NOTE: the flat chord resolver that once lived here (`resolve` / `Resolution` /
// `command_for_chord` / `is_prefix`) is GONE. It was the trial run's model — one
// static table of global commands, one flat chord space — and it structurally
// could not express instances, dynamic sets, or app contributions
// (docs/alt-keytips.md §2). Chord resolution now belongs to `keytip::KeyTipTree`,
// which resolves over the per-scope declaration tree. What survives here is the
// COMMAND TABLE itself: `SHELL_COMMANDS` (structure + default letters + titles),
// the `ShellCommand` action enum, and the `Keymap` letters view the editor uses.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_shell_namespace_clean() {
        // Spec decision (a): no top-level shell keytip may land on an Excel
        // reserved letter. Submenu keytips (parent != None) are free to reuse
        // any letter — they live under the parent's namespace.
        for spec in SHELL_COMMANDS {
            if spec.parent.is_none()
                && let Some(letter) = spec.default_keytip
            {
                assert!(
                    !reserved_letter(letter),
                    "shell command `{}` claims Excel-reserved top-level letter `{}`",
                    spec.id,
                    letter
                );
            }
        }
    }

    #[test]
    fn top_level_keytips_are_unique() {
        let mut seen = std::collections::BTreeMap::new();
        for spec in SHELL_COMMANDS {
            if spec.parent.is_none()
                && let Some(letter) = spec.default_keytip
                && let Some(prev) = seen.insert(letter, spec.id)
            {
                panic!(
                    "top-level keytip `{letter}` claimed by both `{prev}` and `{}`",
                    spec.id
                );
            }
        }
    }

    // Chord RESOLUTION is `keytip::KeyTipTree`'s job now (see its tests). What this
    // module still owns is the command table's letters, so that is what these test.
    #[test]
    fn default_letters_are_the_documented_ones() {
        let keymap = Keymap::defaults();
        assert_eq!(keymap.keytip_for(ShellCommand::ToggleSidebar), Some('b'));
        assert_eq!(keymap.keytip_for(ShellCommand::OpenInsertMenu), Some('i'));
        assert_eq!(keymap.keytip_for(ShellCommand::InsertSession), Some('s'));
        assert_eq!(keymap.keytip_for(ShellCommand::InsertTerminal), Some('t'));
    }

    #[test]
    fn reassigned_letters_replace_excel_ones() {
        let keymap = Keymap::defaults();
        // The four pre-spec violations moved OFF Excel's reserved letters (n/m/f/a)
        // onto namespace-clean ones — the reserved set stays free for apps (§7).
        assert_eq!(keymap.keytip_for(ShellCommand::ToggleNotifications), Some('l'));
        assert_eq!(keymap.keytip_for(ShellCommand::ToggleMetadata), Some('d'));
        assert_eq!(keymap.keytip_for(ShellCommand::ToggleFullscreen), Some('u'));
        assert_eq!(keymap.keytip_for(ShellCommand::ToggleAlwaysOnTop), Some('o'));
    }

    #[test]
    fn overrides_move_the_binding_and_the_badge() {
        // Rebind notifications from `l` to `j`: one source moves both.
        let keymap = Keymap::from_overrides([("notifications.toggle", 'j')]);
        assert_eq!(keymap.keytip_for(ShellCommand::ToggleNotifications), Some('j'));
        assert!(keymap.is_overridden("notifications.toggle"));
    }

    #[test]
    fn chord_for_child_walks_parents() {
        let keymap = Keymap::defaults();
        assert_eq!(keymap.chord_for_id("insert.session").as_deref(), Some("is"));
        assert_eq!(keymap.chord_for_id("sidebar.toggle").as_deref(), Some("b"));
        // Session nav has no accelerator.
        assert_eq!(keymap.chord_for_id("session.next"), None);
    }
}
