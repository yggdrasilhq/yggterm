//! THE BIRTH-TITLE CONVENTION: `New {Machine} {Thing}`, in one place.
//!
//! Owner spec: a session born from a launcher says WHERE it is and WHAT it is,
//! so a browser session on a host called `atlas` reads `New Atlas Ychrome`
//! rather than `New Ychrome` — because with three machines in one sidebar, the
//! app name alone does not identify the row.
//!
//! ⛔ **It is one builder because it was two.** The agent path already composed
//! a birth title from the registry; an APP spawn took its title straight from
//! the manifest verb's own `label` and never called the builder at all. So a
//! machine name added to the builder would have reached agent rows and silently
//! missed every app row — the exact split the spec exists to close. Both paths
//! now compose here.
//!
//! ⚠ **This module deliberately knows nothing about either registry.** It is
//! handed already-resolved words. `agent_cli` knows a CLI's display name and
//! `app_registry` knows a manifest's, and a convention that reached into either
//! would have to reach into both.

/// Compose a birth title.
///
/// `machine` is the resolved machine WORD ([`machine_title_word`]), or `None`
/// on a host with no name worth saying — in which case the title is simply
/// `New {thing}`, which is what every row said before this existed.
///
/// `qualifier` distinguishes one launch of a thing from another (an incognito
/// browser window from an ordinary one). It is the APP'S OWN words, passed
/// through untouched: the libyggterm contract puts naming in the app's hands,
/// and a shell that rewrote "(Incognito)" into its own vocabulary would be
/// deciding what someone else's verb is called.
pub fn birth_title(machine: Option<&str>, thing: &str, qualifier: Option<&str>) -> String {
    let thing = thing.trim();
    let mut title = String::from("New ");
    if let Some(machine) = machine.map(str::trim).filter(|word| !word.is_empty()) {
        title.push_str(machine);
        title.push(' ');
    }
    title.push_str(thing);
    if let Some(qualifier) = qualifier.map(str::trim).filter(|word| !word.is_empty()) {
        title.push(' ');
        title.push_str(qualifier);
    }
    title.trim().to_string()
}

/// The MACHINE WORD for a birth title, from whatever the caller has: a machine
/// key, an ssh target, a host label.
///
/// `None` means "do not say a machine here", and the three cases that produce
/// it are all the same case: the name would tell the user nothing.
///
/// - **Empty** — nothing to say.
/// - **A LOCAL name** (`local`, `localhost`, a loopback address). Every row in
///   the sidebar that is not explicitly remote is local, so stamping "Localhost"
///   on them adds a word to every title and distinguishes nothing. ⚠ This is
///   NOT the same as "the local host has no name": a host that knows it is
///   called `atlas` says `atlas`, and that is the case the spec is about —
///   naming the machine only starts earning its place once more than one is on
///   screen.
///
/// The shape is reduced the way a person would say it aloud: the user half of
/// `dev@atlas.example.net` is not the machine, and the domain is not either.
pub fn machine_title_word(value: &str) -> Option<String> {
    let value = value.trim();
    // `user@host` — the machine is the host half. Rsplit, because a username
    // may not contain `@` but the syntax does not forbid one appearing earlier.
    let host = value.rsplit('@').next().unwrap_or(value);
    // Drop a port, then the domain: `atlas.example.net:22` is the machine
    // `atlas`, and a title carrying the whole FQDN is a title nobody can read
    // at sidebar width.
    let host = host.split(':').next().unwrap_or(host);
    let host = host.split('.').next().unwrap_or(host);
    let host = host.trim().trim_matches('-');
    if host.is_empty() {
        return None;
    }
    let lowered = host.to_ascii_lowercase();
    if matches!(lowered.as_str(), "local" | "localhost" | "127" | "0") {
        return None;
    }
    // A pure IP literal reduces to its first octet above, which would title a
    // row `New 192 Ychrome`. A number is never a machine name.
    if host.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(title_word(host))
}

/// One word, title-cased, with any INTERNAL capitals left alone.
///
/// `atlas` becomes `Atlas`; `myBox` stays `MyBox` rather than being flattened to
/// `Mybox`. A host the user has capitalized deliberately is theirs to spell.
fn title_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The QUALIFIER for one of an app's verbs, derived from what the verb calls
/// itself: the part of the verb's label that is not already the app's name.
///
/// A manifest's verbs are conventionally labelled `New Ychrome` and
/// `New Ychrome (Incognito)`. The first is the app plainly; the second says one
/// more thing, and that one more thing is the qualifier. Appending the WHOLE
/// verb label instead would title a row `New Atlas Ychrome New Ychrome
/// (Incognito)`.
///
/// `None` when the verb adds nothing — which is the ordinary case and is why a
/// primary launch reads exactly as the spec asks: `New {Machine} {App}`.
pub fn app_verb_title_qualifier(app_label: &str, verb_label: &str) -> Option<String> {
    let verb = verb_label.trim();
    let app = app_label.trim();
    if verb.is_empty() || app.is_empty() {
        return None;
    }
    // Strip the two things the label shares with the title being built, in the
    // order a label writes them.
    let rest = verb.strip_prefix("New ").unwrap_or(verb).trim();
    let rest = match rest.to_ascii_lowercase().strip_prefix(&app.to_ascii_lowercase()) {
        // Slice the ORIGINAL by the matched length: the comparison is
        // case-insensitive, but what survives must be the app's own casing.
        Some(_) => rest[app.len()..].trim(),
        None => rest,
    };
    // Nothing left, or the label was only ever the app's name.
    if rest.is_empty() || rest.eq_ignore_ascii_case(app) {
        return None;
    }
    Some(rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_convention_is_new_machine_thing() {
        assert_eq!(birth_title(Some("Atlas"), "Ychrome", None), "New Atlas Ychrome");
        assert_eq!(
            birth_title(Some("Atlas"), "Ychrome", Some("(Incognito)")),
            "New Atlas Ychrome (Incognito)",
        );
    }

    /// A host with no name worth saying leaves the title exactly as it read
    /// before the machine existed — this must not become `New  Ychrome`.
    #[test]
    fn no_machine_word_means_the_title_it_always_had() {
        assert_eq!(birth_title(None, "Ychrome", None), "New Ychrome");
        assert_eq!(birth_title(Some("   "), "Ychrome", None), "New Ychrome");
        assert_eq!(birth_title(None, "Ychrome", Some("(Incognito)")), "New Ychrome (Incognito)");
    }

    #[test]
    fn a_machine_word_is_the_host_said_the_way_a_person_says_it() {
        assert_eq!(machine_title_word("atlas").as_deref(), Some("Atlas"));
        assert_eq!(machine_title_word("workshop-2").as_deref(), Some("Workshop-2"));
        // The user half is not the machine, the domain is not the machine, and
        // the port certainly is not.
        assert_eq!(machine_title_word("someone@atlas.example.net").as_deref(), Some("Atlas"));
        assert_eq!(machine_title_word("atlas.example.net:22").as_deref(), Some("Atlas"));
        // Deliberate internal capitals are the user's to keep.
        assert_eq!(machine_title_word("myBox").as_deref(), Some("MyBox"));
    }

    /// ⛔ The names that would add a word to every title and distinguish
    /// nothing. Every non-remote row is local, so "Localhost" is noise.
    #[test]
    fn a_local_or_numeric_name_is_not_worth_saying() {
        for value in ["", "   ", "local", "localhost", "LOCALHOST", "127.0.0.1", "0.0.0.0"] {
            assert_eq!(machine_title_word(value), None, "{value:?}");
        }
    }

    /// The qualifier is what the VERB adds beyond the app's own name, in the
    /// app's own words — never the whole label, which would say the app twice.
    #[test]
    fn a_qualifier_is_only_what_the_verb_adds() {
        assert_eq!(app_verb_title_qualifier("Ychrome", "New Ychrome"), None);
        assert_eq!(
            app_verb_title_qualifier("Ychrome", "New Ychrome (Incognito)").as_deref(),
            Some("(Incognito)"),
        );
        // A label that does not follow the convention keeps its own words rather
        // than being discarded: the app named it, and the shell is not the
        // authority on what someone else's verb is called.
        assert_eq!(
            app_verb_title_qualifier("Ychrome", "Private window").as_deref(),
            Some("Private window"),
        );
        assert_eq!(app_verb_title_qualifier("Ychrome", "  "), None);
        assert_eq!(app_verb_title_qualifier("", "New Ychrome"), None);
    }

    /// The whole point, end to end: two verbs of one app on one machine must
    /// stay TELLABLE APART. Collapsing them is the near-duplicate-rows defect
    /// the slug fix removed, and re-introducing it here would undo that.
    #[test]
    fn two_verbs_of_one_app_do_not_land_on_the_same_title() {
        let machine = machine_title_word("atlas");
        let primary = birth_title(machine.as_deref(), "Ychrome", None);
        let other = birth_title(
            machine.as_deref(),
            "Ychrome",
            app_verb_title_qualifier("Ychrome", "New Ychrome (Incognito)").as_deref(),
        );
        assert_eq!(primary, "New Atlas Ychrome");
        assert_ne!(primary, other);
        assert_eq!(other, "New Atlas Ychrome (Incognito)");
    }
}
