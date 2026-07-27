//! ASCII → (XKB keysym, evdev hardware code) — the table spike C said we owe.
//!
//! **Why a table at all.** `wpe_view_backend_dispatch_keyboard_event` takes a
//! `key_code` that is an **XKB keysym**, not an ASCII byte and not a scancode,
//! and a `hardware_key_code` that is the **evdev code + 8**. Passing a raw
//! character where a keysym belongs produces an event WebKit silently ignores,
//! which is indistinguishable from "input does not work" — spike C lost time to
//! exactly this class of mistake.
//!
//! So the public API never takes a key code. It takes text, and this module is
//! the only thing that knows the encoding.
//!
//! For printable ASCII the keysym IS the character's byte value (Latin-1 block
//! of X11 keysyms), so the keysym half is a cast. The hardware code is not
//! derivable and must be looked up.

/// One key press to synthesize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyStroke {
    pub keysym: u32,
    pub hardware_code: u32,
    /// Shift must be held for this character (e.g. `A`, `!`).
    pub shift: bool,
}

/// X11 keysyms for the keys that have no printable character.
pub mod keysym {
    pub const BACKSPACE: u32 = 0xFF08;
    pub const TAB: u32 = 0xFF09;
    pub const RETURN: u32 = 0xFF0D;
    pub const ESCAPE: u32 = 0xFF1B;
    pub const DELETE: u32 = 0xFFFF;
    pub const LEFT: u32 = 0xFF51;
    pub const UP: u32 = 0xFF52;
    pub const RIGHT: u32 = 0xFF53;
    pub const DOWN: u32 = 0xFF54;
}

/// evdev codes for the unshifted US layout, indexed by the character they
/// produce. `hardware_key_code` is this + [`EVDEV_OFFSET`].
const EVDEV_OFFSET: u32 = 8;

/// `(character, evdev code, needs shift)` for printable US-layout ASCII.
///
/// Deliberately a table and not a formula: evdev codes follow the physical
/// keyboard's row order, which no arithmetic on the character recovers.
const US_LAYOUT: &[(char, u32, bool)] = &[
    // digit row
    ('1', 2, false), ('!', 2, true),
    ('2', 3, false), ('@', 3, true),
    ('3', 4, false), ('#', 4, true),
    ('4', 5, false), ('$', 5, true),
    ('5', 6, false), ('%', 6, true),
    ('6', 7, false), ('^', 7, true),
    ('7', 8, false), ('&', 8, true),
    ('8', 9, false), ('*', 9, true),
    ('9', 10, false), ('(', 10, true),
    ('0', 11, false), (')', 11, true),
    ('-', 12, false), ('_', 12, true),
    ('=', 13, false), ('+', 13, true),
    // top letter row
    ('q', 16, false), ('w', 17, false), ('e', 18, false), ('r', 19, false),
    ('t', 20, false), ('y', 21, false), ('u', 22, false), ('i', 23, false),
    ('o', 24, false), ('p', 25, false),
    ('[', 26, false), ('{', 26, true),
    (']', 27, false), ('}', 27, true),
    // home row
    ('a', 30, false), ('s', 31, false), ('d', 32, false), ('f', 33, false),
    ('g', 34, false), ('h', 35, false), ('j', 36, false), ('k', 37, false),
    ('l', 38, false),
    (';', 39, false), (':', 39, true),
    ('\'', 40, false), ('"', 40, true),
    ('`', 41, false), ('~', 41, true),
    ('\\', 43, false), ('|', 43, true),
    // bottom row
    ('z', 44, false), ('x', 45, false), ('c', 46, false), ('v', 47, false),
    ('b', 48, false), ('n', 49, false), ('m', 50, false),
    (',', 51, false), ('<', 51, true),
    ('.', 52, false), ('>', 52, true),
    ('/', 53, false), ('?', 53, true),
    (' ', 57, false),
];

/// The keystroke for `ch`, or `None` if this crate cannot type it.
///
/// Returning `None` rather than guessing is deliberate: a wrong keysym is
/// silently swallowed by WebKit, so a caller that cannot type a character must
/// learn that from an error and not from a page that mysteriously did nothing.
pub(crate) fn stroke_for_char(ch: char) -> Option<KeyStroke> {
    if ch == '\n' || ch == '\r' {
        return Some(KeyStroke {
            keysym: keysym::RETURN,
            hardware_code: 28 + EVDEV_OFFSET,
            shift: false,
        });
    }
    if ch == '\t' {
        return Some(KeyStroke {
            keysym: keysym::TAB,
            hardware_code: 15 + EVDEV_OFFSET,
            shift: false,
        });
    }

    // Uppercase letters share the lowercase key with shift held; the KEYSYM is
    // still the uppercase character's own code point.
    let (lookup, shift_for_case) = if ch.is_ascii_uppercase() {
        (ch.to_ascii_lowercase(), true)
    } else {
        (ch, false)
    };

    US_LAYOUT
        .iter()
        .find(|(c, _, _)| *c == lookup)
        .map(|(_, evdev, shifted)| KeyStroke {
            // Printable ASCII keysyms ARE the character's code point.
            keysym: ch as u32,
            hardware_code: evdev + EVDEV_OFFSET,
            shift: *shifted || shift_for_case,
        })
}

/// The keystroke for a non-printable key named by its X11 keysym.
pub(crate) fn stroke_for_keysym(sym: u32) -> KeyStroke {
    let evdev = match sym {
        keysym::BACKSPACE => 14,
        keysym::TAB => 15,
        keysym::RETURN => 28,
        keysym::ESCAPE => 1,
        keysym::DELETE => 111,
        keysym::LEFT => 105,
        keysym::UP => 103,
        keysym::RIGHT => 106,
        keysym::DOWN => 108,
        _ => 0,
    };
    KeyStroke {
        keysym: sym,
        hardware_code: if evdev == 0 { 0 } else { evdev + EVDEV_OFFSET },
        shift: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact values spike C proved land: `x` is keysym 0x78 and hardware
    /// code 53 (evdev KEY_X 45 + 8). If this drifts, typed input silently stops
    /// working, which is the failure mode this table exists to prevent.
    #[test]
    fn x_matches_the_values_the_spike_proved() {
        let stroke = stroke_for_char('x').expect("x is typable");
        assert_eq!(stroke.keysym, 0x78, "keysym for 'x'");
        assert_eq!(stroke.hardware_code, 53, "evdev KEY_X (45) + 8");
        assert!(!stroke.shift);
    }

    #[test]
    fn printable_ascii_keysyms_are_the_character_itself() {
        for ch in "abcXYZ019 ,./".chars() {
            let stroke = stroke_for_char(ch)
                .unwrap_or_else(|| panic!("{ch:?} should be typable"));
            assert_eq!(
                stroke.keysym, ch as u32,
                "{ch:?}: a printable ASCII keysym is its own code point",
            );
        }
    }

    #[test]
    fn shifted_characters_ask_for_shift_and_uppercase_reuses_the_lower_key() {
        let upper = stroke_for_char('A').expect("A");
        let lower = stroke_for_char('a').expect("a");
        assert_eq!(
            upper.hardware_code, lower.hardware_code,
            "A and a are the same physical key",
        );
        assert!(upper.shift, "A needs shift");
        assert!(!lower.shift, "a does not");
        assert_eq!(upper.keysym, 'A' as u32);

        let bang = stroke_for_char('!').expect("!");
        let one = stroke_for_char('1').expect("1");
        assert_eq!(bang.hardware_code, one.hardware_code);
        assert!(bang.shift && !one.shift);
    }

    #[test]
    fn every_table_entry_has_a_plausible_evdev_code() {
        for (ch, evdev, _) in US_LAYOUT {
            assert!(
                *evdev > 0 && *evdev < 128,
                "{ch:?} has an implausible evdev code {evdev}",
            );
        }
    }

    #[test]
    fn newline_and_tab_map_to_their_named_keysyms() {
        assert_eq!(stroke_for_char('\n').unwrap().keysym, keysym::RETURN);
        assert_eq!(stroke_for_char('\t').unwrap().keysym, keysym::TAB);
        assert_eq!(stroke_for_char('\n').unwrap().hardware_code, 36);
    }

    /// A character we cannot type must be an ERROR, never a silently wrong
    /// keysym — WebKit swallows those and the page just does nothing.
    #[test]
    fn untypable_characters_are_refused_rather_than_guessed() {
        for ch in ['é', '→', '\u{1F600}'] {
            assert!(
                stroke_for_char(ch).is_none(),
                "{ch:?} must be refused, not guessed at",
            );
        }
    }

    #[test]
    fn named_keys_resolve_to_real_hardware_codes() {
        for sym in [
            keysym::RETURN,
            keysym::ESCAPE,
            keysym::BACKSPACE,
            keysym::LEFT,
            keysym::UP,
            keysym::RIGHT,
            keysym::DOWN,
        ] {
            let stroke = stroke_for_keysym(sym);
            assert_eq!(stroke.keysym, sym);
            assert!(stroke.hardware_code > 8, "{sym:#x} has no hardware code");
        }
    }
}
