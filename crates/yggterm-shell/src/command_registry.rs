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
    InsertSession,
    InsertTerminal,
    NextSession,
    PrevSession,
    OpenKeymapEditor,
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
    /// True when pressing this command's keytip should OPEN a sub-level of fresh
    /// KeyTips rather than act-and-dismiss (Excel's ALT,H,… nesting). The insert
    /// "+" menu is the one such node today.
    pub opens_submenu: bool,
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
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::ViewWeb,
        id: "view.web",
        title: "Web view",
        parent: None,
        default_keytip: Some('v'),
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::ViewTerminal,
        id: "view.terminal",
        title: "Terminal view",
        parent: None,
        default_keytip: Some('t'),
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::ToggleConnect,
        id: "connect.toggle",
        title: "Connect SSH",
        parent: None,
        default_keytip: Some('c'),
        opens_submenu: false,
    },
    CommandSpec {
        // Was `n` in the pre-spec draft — `n` is Excel's Insert tab, reserved.
        command: ShellCommand::ToggleNotifications,
        id: "notifications.toggle",
        title: "Notifications",
        parent: None,
        default_keytip: Some('l'),
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::ToggleSettings,
        id: "settings.toggle",
        title: "Settings",
        parent: None,
        default_keytip: Some('g'),
        opens_submenu: false,
    },
    CommandSpec {
        // Was `m` in the pre-spec draft — `m` is Excel's Formulas tab, reserved.
        command: ShellCommand::ToggleMetadata,
        id: "metadata.toggle",
        title: "Session metadata",
        parent: None,
        default_keytip: Some('d'),
        opens_submenu: false,
    },
    CommandSpec {
        // Was `f` in the pre-spec draft — `f` is Excel's File tab, reserved.
        command: ShellCommand::ToggleFullscreen,
        id: "window.fullscreen",
        title: "Toggle fullscreen",
        parent: None,
        default_keytip: Some('u'),
        opens_submenu: false,
    },
    CommandSpec {
        // Was `a` in the pre-spec draft — `a` is Excel's Data tab, reserved.
        command: ShellCommand::ToggleAlwaysOnTop,
        id: "window.always-on-top",
        title: "Always on top",
        parent: None,
        default_keytip: Some('o'),
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::OpenKeymapEditor,
        id: "keymap.editor",
        title: "Edit ALT+ keys",
        parent: None,
        default_keytip: Some('k'),
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::OpenInsertMenu,
        id: "insert.menu",
        title: "New… menu",
        parent: None,
        default_keytip: Some('i'),
        opens_submenu: true,
    },
    CommandSpec {
        command: ShellCommand::InsertSession,
        id: "insert.session",
        title: "New session",
        parent: Some("insert.menu"),
        default_keytip: Some('s'),
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::InsertTerminal,
        id: "insert.terminal",
        title: "New terminal",
        parent: Some("insert.menu"),
        default_keytip: Some('t'),
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::NextSession,
        id: "session.next",
        title: "Next live session",
        parent: None,
        default_keytip: None,
        opens_submenu: false,
    },
    CommandSpec {
        command: ShellCommand::PrevSession,
        id: "session.prev",
        title: "Previous live session",
        parent: None,
        default_keytip: None,
        opens_submenu: false,
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

    /// The command a full chord string resolves to, if any.
    fn command_for_chord(&self, chord: &str) -> Option<ShellCommand> {
        SHELL_COMMANDS
            .iter()
            .find(|spec| self.chord_for_id(spec.id).as_deref() == Some(chord))
            .map(|spec| spec.command)
    }

    /// True if `prefix` is a strict prefix of some command's chord (so more
    /// keys could still complete a binding).
    fn is_prefix(&self, prefix: &str) -> bool {
        if prefix.is_empty() {
            return true;
        }
        SHELL_COMMANDS.iter().any(|spec| {
            self.chord_for_id(spec.id)
                .is_some_and(|chord| chord.len() > prefix.len() && chord.starts_with(prefix))
        })
    }

    /// Resolve a typed KeyTips sequence.
    pub fn resolve(&self, sequence: &str) -> Resolution {
        let sequence = sequence.to_ascii_lowercase();
        if sequence.is_empty() {
            return Resolution::Pending;
        }
        // An exact command wins even when it is also a prefix of a longer chord
        // (the insert menu on `i` is both an action — open the submenu — and the
        // `is`/`it` prefix). `opens_submenu` disambiguates the behaviour.
        if let Some(command) = self.command_for_chord(&sequence) {
            return Resolution::Command(command);
        }
        if self.is_prefix(&sequence) {
            return Resolution::Pending;
        }
        Resolution::Invalid
    }
}

/// The outcome of feeding a KeyTips sequence to the keymap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// A valid prefix; wait for more keys.
    Pending,
    /// The sequence maps to this command.
    Command(ShellCommand),
    /// No binding and no prefix — dismiss the overlay.
    Invalid,
}

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

    #[test]
    fn resolves_default_chords() {
        let keymap = Keymap::defaults();
        assert_eq!(
            keymap.resolve("b"),
            Resolution::Command(ShellCommand::ToggleSidebar)
        );
        assert_eq!(
            keymap.resolve("i"),
            Resolution::Command(ShellCommand::OpenInsertMenu)
        );
        assert_eq!(
            keymap.resolve("is"),
            Resolution::Command(ShellCommand::InsertSession)
        );
        assert_eq!(
            keymap.resolve("it"),
            Resolution::Command(ShellCommand::InsertTerminal)
        );
        assert_eq!(keymap.resolve("i"), Resolution::Command(ShellCommand::OpenInsertMenu));
        assert_eq!(keymap.resolve("z"), Resolution::Invalid);
        assert_eq!(keymap.resolve("ip"), Resolution::Invalid);
    }

    #[test]
    fn reassigned_letters_replace_excel_ones() {
        let keymap = Keymap::defaults();
        // The four pre-spec violations are gone from their old letters…
        assert_eq!(keymap.resolve("n"), Resolution::Invalid);
        assert_eq!(keymap.resolve("m"), Resolution::Invalid);
        assert_eq!(keymap.resolve("f"), Resolution::Invalid);
        assert_eq!(keymap.resolve("a"), Resolution::Invalid);
        // …and reachable on their new, namespace-clean letters.
        assert_eq!(
            keymap.resolve("l"),
            Resolution::Command(ShellCommand::ToggleNotifications)
        );
        assert_eq!(
            keymap.resolve("d"),
            Resolution::Command(ShellCommand::ToggleMetadata)
        );
        assert_eq!(
            keymap.resolve("u"),
            Resolution::Command(ShellCommand::ToggleFullscreen)
        );
        assert_eq!(
            keymap.resolve("o"),
            Resolution::Command(ShellCommand::ToggleAlwaysOnTop)
        );
    }

    #[test]
    fn overrides_move_the_binding_and_the_badge() {
        // Rebind notifications from `l` to `j`.
        let keymap = Keymap::from_overrides([("notifications.toggle", 'j')]);
        assert_eq!(keymap.keytip_for(ShellCommand::ToggleNotifications), Some('j'));
        assert_eq!(
            keymap.resolve("j"),
            Resolution::Command(ShellCommand::ToggleNotifications)
        );
        // The old letter no longer resolves.
        assert_eq!(keymap.resolve("l"), Resolution::Invalid);
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
