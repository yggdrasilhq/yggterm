// ============================================================================
// The key plane: schema-declared `key_capture` (ymacs is the forcing
// consumer; docs/spec-key-plane.md in the ymacs repo is the contract).
// Kept as its own module: the contract has its own table to lock.
// ============================================================================
#[cfg(test)]
mod key_plane_locks {
    use super::tests::test_shell_bootstrap_with_active_session;
    use super::*;
    use keyboard_types::{Key, Modifiers};

    #[test]
    fn key_capture_schema_field_parses_and_defaults_off() {
        let on: AppPaneSchema =
            serde_json::from_value(json!({"title": "t", "key_capture": true}))
                .expect("a schema declaring key capture parses");
        assert!(on.key_capture);
        let off: AppPaneSchema = serde_json::from_value(json!({"title": "t"}))
            .expect("a schema without the field parses");
        assert!(
            !off.key_capture,
            "absent means off: serde(default) is the rollout story, an old GUI ignores the field"
        );
    }

    #[test]
    fn document_pane_key_capture_names_the_pane_from_the_live_schema() {
        let bootstrap = test_shell_bootstrap_with_active_session("local://a");
        let mut shell = ShellState::new(bootstrap);
        assert!(
            shell.document_pane_key_capture("local://a").is_none(),
            "no schema yet: nothing captures keys"
        );

        let schema = AppPaneSchema {
            title: "ymacs".to_string(),
            widgets: Vec::new(),
            titlebar_switch: None,
            footer: Vec::new(),
            ribbon: Vec::new(),
            split_ratio: None,
            key_capture: true,
        };
        let seq = shell.document_pane_next_request("local://a");
        shell.document_pane_apply_schema(seq, "local://a", "doc", schema);
        assert_eq!(
            shell.document_pane_key_capture("local://a").as_deref(),
            Some("doc"),
            "the one reader of the flag must name the pane the root arm POSTs to"
        );
    }

    #[test]
    fn emacs_chord_spells_the_key_plane_table() {
        let none = Modifiers::empty();
        // Printable characters; shift is folded into the produced character.
        assert_eq!(
            emacs_chord(&Key::Character("h".into()), none).as_deref(),
            Some("h")
        );
        assert_eq!(
            emacs_chord(&Key::Character("A".into()), none).as_deref(),
            Some("A")
        );
        assert_eq!(
            emacs_chord(&Key::Character("?".into()), none).as_deref(),
            Some("?")
        );
        // Space arrives as Character(" ") (no Space variant in keyboard_types).
        assert_eq!(
            emacs_chord(&Key::Character(" ".into()), none).as_deref(),
            Some("SPC")
        );
        // Modifier prefixes, in C- M- order; C- lowers the character.
        assert_eq!(
            emacs_chord(&Key::Character("f".into()), Modifiers::CONTROL).as_deref(),
            Some("C-f")
        );
        assert_eq!(
            emacs_chord(&Key::Character("x".into()), Modifiers::ALT).as_deref(),
            Some("M-x")
        );
        assert_eq!(
            emacs_chord(&Key::Character("f".into()), Modifiers::CONTROL | Modifiers::ALT)
                .as_deref(),
            Some("C-M-f")
        );
        assert_eq!(
            emacs_chord(&Key::Character("F".into()), Modifiers::CONTROL | Modifiers::SHIFT)
                .as_deref(),
            Some("C-f"),
            "C- with shift spells the character, never S-"
        );
        // Specials, by name.
        assert_eq!(emacs_chord(&Key::Enter, none).as_deref(), Some("RET"));
        assert_eq!(emacs_chord(&Key::Tab, none).as_deref(), Some("TAB"));
        assert_eq!(emacs_chord(&Key::Escape, none).as_deref(), Some("ESC"));
        assert_eq!(emacs_chord(&Key::Backspace, none).as_deref(), Some("DEL"));
        assert_eq!(emacs_chord(&Key::ArrowUp, none).as_deref(), Some("<up>"));
        assert_eq!(emacs_chord(&Key::PageUp, none).as_deref(), Some("<prior>"));
        assert_eq!(emacs_chord(&Key::PageDown, none).as_deref(), Some("<next>"));
        assert_eq!(emacs_chord(&Key::F5, none).as_deref(), Some("<f5>"));
        // Shift on a special IS meaningful: S-TAB.
        assert_eq!(
            emacs_chord(&Key::Tab, Modifiers::SHIFT).as_deref(),
            Some("S-TAB")
        );
        // Modifier keys alone forward nothing.
        assert_eq!(emacs_chord(&Key::Control, none), None);
        assert_eq!(emacs_chord(&Key::Alt, none), None);
        assert_eq!(emacs_chord(&Key::Shift, none), None);
        assert_eq!(emacs_chord(&Key::Meta, none), None);
    }
}
