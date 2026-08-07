use yggterm_core::looks_like_generated_fallback_title;
use yggterm_server::SessionKind;

pub(crate) fn background_copy_retry_key(prefix: &str, session_path: &str) -> String {
    format!("{prefix}:{session_path}")
}

pub(crate) fn env_copy_generation_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub(crate) fn implicit_copy_generation_enabled_from_env(value: Option<&str>) -> bool {
    value.map_or(true, |value| env_copy_generation_enabled(Some(value)))
}

pub(crate) fn copy_generation_start_allowed(
    force: bool,
    announce: bool,
    implicit_enabled: bool,
) -> bool {
    force || announce || implicit_enabled
}

pub(crate) fn shell_title_case_words(value: &str) -> String {
    value
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn terminal_kind_title_suffix(kind: SessionKind) -> &'static str {
    // An agent CLI's human name is its own to declare — the descriptor's
    // `display_name` is the same string the metadata rail, the menus and the
    // start page use, so a row's title cannot call it something else.
    if let Some(descriptor) = yggterm_core::agent_cli::agent_cli_descriptor(kind) {
        // ⚠ `Codex LiteLLM` with a space, not the descriptor's hyphenated
        // `Codex-LiteLLM`: this string is title PROSE and lands in
        // `looks_like_generated_fallback_title`'s word matching, which the
        // hyphen would change.
        return match kind {
            SessionKind::CodexLiteLlm => "Codex LiteLLM",
            _ => descriptor.display_name,
        };
    }
    match kind {
        SessionKind::Shell => "Shell",
        SessionKind::SshShell => "SSH Terminal",
        SessionKind::Document => "Document",
        // Unreachable: every agent kind took the branch above.
        _ => "Shell",
    }
}

pub(crate) fn humanized_terminal_title(
    kind: SessionKind,
    cwd: &str,
    host_label: Option<&str>,
) -> Option<String> {
    let suffix = terminal_kind_title_suffix(kind);
    let cwd = cwd.trim().trim_end_matches('/');
    if !cwd.is_empty() {
        if let Some(home_user) = cwd.strip_prefix("/home/")
            && !home_user.is_empty()
            && !home_user.contains('/')
        {
            let title = format!("{} Home {suffix}", shell_title_case_words(home_user));
            return (!looks_like_generated_fallback_title(&title)).then_some(title);
        }
        if let Some(leaf) = cwd.rsplit('/').find(|segment| !segment.trim().is_empty()) {
            let leaf = leaf.replace('_', " ").replace('-', " ");
            let title = format!("{} {suffix}", shell_title_case_words(&leaf));
            return (!looks_like_generated_fallback_title(&title)).then_some(title);
        }
    }
    if let Some(host_label) = host_label.map(str::trim).filter(|value| !value.is_empty()) {
        let title = format!("{} {suffix}", shell_title_case_words(host_label));
        return (!looks_like_generated_fallback_title(&title)).then_some(title);
    }
    if kind.is_agent() {
        return Some(format!("{suffix} Session"));
    }
    Some(match kind {
        SessionKind::Shell => "Local Terminal".to_string(),
        SessionKind::SshShell => "SSH Terminal".to_string(),
        SessionKind::Document => "Document".to_string(),
        _ => "Local Terminal".to_string(),
    })
}

pub(crate) fn title_looks_like_abbreviated_shell_label(title: &str) -> bool {
    let trimmed = title.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 10 {
        return false;
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    !words.is_empty()
        && words.len() <= 3
        && words.iter().all(|word| {
            let compact = word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
            !compact.is_empty()
                && compact.chars().count() <= 4
                && compact.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
}

pub(crate) fn title_is_low_signal_for_copy(title: &str, cwd: &str) -> bool {
    let trimmed = title.trim();
    let cwd = cwd.trim();
    trimmed.is_empty()
        || looks_like_generated_fallback_title(trimmed)
        || (!cwd.is_empty() && trimmed == cwd)
}

pub(crate) fn title_needs_generation_from_visible_titles(
    resolved_title: Option<&str>,
    cwd: &str,
    fallback_row_title: &str,
) -> bool {
    if resolved_title.is_some_and(|title| !title_is_low_signal_for_copy(title, cwd)) {
        return false;
    }
    let trimmed_fallback = fallback_row_title.trim();
    trimmed_fallback.is_empty()
        || looks_like_generated_fallback_title(trimmed_fallback)
        || (!cwd.trim().is_empty() && trimmed_fallback == cwd.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_generation_start_policy_blocks_implicit_selection_work() {
        assert!(!copy_generation_start_allowed(false, false, false));
        assert!(copy_generation_start_allowed(true, false, false));
        assert!(copy_generation_start_allowed(false, true, false));
        assert!(copy_generation_start_allowed(false, false, true));
        assert!(env_copy_generation_enabled(Some("true")));
        assert!(env_copy_generation_enabled(Some("1")));
        assert!(!env_copy_generation_enabled(Some("false")));
        assert!(!env_copy_generation_enabled(None));
        assert!(implicit_copy_generation_enabled_from_env(None));
        assert!(implicit_copy_generation_enabled_from_env(Some("true")));
        assert!(!implicit_copy_generation_enabled_from_env(Some("false")));
    }

    #[test]
    fn abbreviated_shell_label_detection_flags_short_ui_titles() {
        assert!(title_looks_like_abbreviated_shell_label("Dev Sta"));
        assert!(title_looks_like_abbreviated_shell_label("Git UI"));
        assert!(!title_looks_like_abbreviated_shell_label("Yggterm Shell"));
        assert!(!title_looks_like_abbreviated_shell_label(
            "Investigate System Load"
        ));
    }

    /// The reported defect, stated as a property of the two title producers.
    ///
    /// A human's shell in a directory called `workspace` is labelled
    /// "Workspace Shell" — that string is generated HERE, from the cwd leaf,
    /// and it is what an agent's scratch row also wore, which is why every
    /// title-based search for the agent's row missed it. An agent-plane title
    /// must therefore differ from it AND survive this module's low-signal
    /// demotion, or `resolved_session_title` falls straight back to the
    /// humanized cwd leaf and the row is anonymous again.
    #[test]
    fn an_agent_plane_title_is_not_the_label_a_humans_shell_in_the_same_cwd_wears() {
        let cwd = "/home/operator/workspace";
        let human = humanized_terminal_title(SessionKind::Shell, cwd, Some("host"))
            .expect("a human's shell in this cwd has a label");
        assert_eq!(
            human, "Workspace Shell",
            "this is the ambiguous label the incident was about"
        );

        // Both forms, because the purpose is optional and the form WITHOUT one
        // is exactly the case that used to collide with the human label.
        for purpose in [None, Some("reap leftover app processes")] {
            let agent = yggterm_server::agent_plane_session_title(
                Some("probe-7"),
                purpose,
                SessionKind::Shell,
            );
            assert_ne!(
                agent, human,
                "an agent's row must not wear the label a human's shell in the same \
                 directory wears (purpose {purpose:?})"
            );
            assert!(
                !agent.starts_with(&human),
                "{agent:?} is the human label with a suffix, so the row still reads \
                 as a shell in that directory"
            );
            assert!(
                !title_is_low_signal_for_copy(&agent, cwd),
                "the copy layer would demote {agent:?} and rename the row after its cwd"
            );
            assert!(
                !title_looks_like_abbreviated_shell_label(&agent),
                "{agent:?} reads as a short UI label, which the row treats as junk"
            );
            assert!(
                !title_needs_generation_from_visible_titles(Some(&agent), cwd, &human),
                "an agent-named row must not be queued for title regeneration"
            );
        }
    }

    #[test]
    fn humanized_terminal_title_uses_cwd_before_host_label() {
        assert_eq!(
            humanized_terminal_title(SessionKind::Shell, "/home/user/git/samplenotes", Some("dev")),
            Some("Samplenotes Shell".to_string())
        );
        assert_eq!(
            humanized_terminal_title(SessionKind::Codex, "/home/user", Some("dev")),
            Some("User Home Codex".to_string())
        );
    }
}
