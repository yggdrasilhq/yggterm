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
