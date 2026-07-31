//! THE argv flag-value rule for every yggterm CLI surface.
//!
//! Both binaries (`yggterm`, `yggterm-headless`) and the server-side parsers
//! that read the same argv (see `yggterm_server::session_tenancy`) call this
//! one function, so `--flag value` and `--flag=value` can never mean different
//! things depending on which entry point the agent happened to type. A second
//! copy of this rule is how `--ephemeral-owner-pid=4242` was silently DISCARDED
//! by a `windows(2)` exact-match parser while the equivalent spaced form worked
//! (round-25 tenancy review, finding P1).

/// The value of `flag`, written either `--flag value` or `--flag=value`.
///
/// A spaced value that itself starts with `--` reads as ABSENT, not as the
/// value: `--purpose --ephemeral` is a caller who forgot the purpose text, and
/// swallowing the next flag would arm something they did not ask for. The
/// inline form has no such ambiguity, so `--purpose=--weird` is honoured.
pub fn cli_flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let inline_prefix = format!("{flag}=");
    for (index, value) in args.iter().enumerate() {
        if value == flag {
            return args
                .get(index + 1)
                .map(String::as_str)
                .filter(|next| !next.starts_with("--"));
        }
        if let Some(inline) = value.strip_prefix(&inline_prefix) {
            return Some(inline);
        }
    }
    None
}

/// The positional (non-flag) arguments from `start` onward.
///
/// The companion rule to [`cli_flag_value`], and the same reason it lives here:
/// a positional reader that skips `--flag value` pairs differently on one entry
/// point than another makes `server app web capture-element --selector x out.png`
/// mean two things. Both binaries and the shared `server app web` dispatcher
/// call this one implementation.
///
/// A `--flag` consumes the next token as its value ONLY when that token is not
/// itself a flag — the same "a flag with no value is absent" rule
/// [`cli_flag_value`] applies, so the two cannot disagree about where a value
/// ends and a positional begins.
pub fn cli_positional_args(args: &[String], start: usize) -> Vec<&str> {
    let mut positional = Vec::new();
    let mut index = start;
    while index < args.len() {
        let value = args[index].as_str();
        if value.starts_with("--") {
            if index + 1 < args.len() && !args[index + 1].starts_with("--") {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        positional.push(value);
        index += 1;
    }
    positional
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn both_spellings_of_a_flag_value_mean_the_same_thing() {
        for args in [
            argv(&["new", "--purpose", "probe the queue"]),
            argv(&["new", "--purpose=probe the queue"]),
        ] {
            assert_eq!(
                cli_flag_value(&args, "--purpose"),
                Some("probe the queue"),
                "{args:?} must parse the same as its sibling spelling"
            );
        }
    }

    #[test]
    fn a_flag_with_no_value_is_absent_rather_than_swallowing_the_next_flag() {
        let args = argv(&["new", "--purpose", "--ephemeral"]);
        assert_eq!(cli_flag_value(&args, "--purpose"), None);
        assert_eq!(cli_flag_value(&args, "--missing"), None);
        // The inline form is unambiguous, so it keeps whatever was written.
        let inline = argv(&["new", "--purpose=--ephemeral"]);
        assert_eq!(cli_flag_value(&inline, "--purpose"), Some("--ephemeral"));
    }

    #[test]
    fn a_longer_flag_sharing_a_prefix_is_not_mistaken_for_a_shorter_one() {
        let args = argv(&["new", "--ephemeral-owner-pid=4242"]);
        assert_eq!(cli_flag_value(&args, "--ephemeral"), None);
        assert_eq!(cli_flag_value(&args, "--ephemeral-owner-pid"), Some("4242"));
    }
}
