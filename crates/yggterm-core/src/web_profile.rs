//! Web-profile identity — the ONE owner of what a profile name means.
//!
//! A web profile is a storage jar at `~/.yggterm/web-profiles/<name>/`
//! (cookies, SQLite WAL/SHM, IndexedDB, service workers, caches, downloads).
//! Two `WebContext`s opened on one profile corrupt it, so the daemon hands out
//! a single-writer lock keyed by profile name (`profile_write_lock` in
//! `yggterm-server`, slice 4.2).
//!
//! That lock is only as sound as the KEY, which is why normalization lives
//! here rather than in the GUI: if the GUI normalized `"Default "` to
//! `"default"` while the daemon keyed the raw string, two clients would hold
//! two "different" locks over one directory — the exact corruption the lock
//! exists to prevent. One owner, both crates.

/// Reserved profile name for an ephemeral (private-browsing) surface: no jar on
/// disk, all website data in memory, gone when the surface closes. A
/// `web-profiles/temp/` directory on disk is ignored by design.
pub const WEB_PROFILE_TEMP: &str = "temp";

/// Fallback profile when none is named (or the name is unsafe).
pub const WEB_PROFILE_DEFAULT: &str = "default";

/// Canonical profile name for a caller-supplied value.
///
/// Rejects anything that could escape `~/.yggterm/web-profiles/` (path
/// separators, `.`, `..`, empty) and falls back to [`WEB_PROFILE_DEFAULT`].
/// Surrounding whitespace is trimmed, so `"default "` and `"default"` are one
/// profile — and therefore one lock.
pub fn normalize_web_profile(profile: Option<&str>) -> String {
    let name = profile.map(str::trim).unwrap_or("");
    let safe = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(std::path::is_separator);
    if safe {
        name.to_string()
    } else {
        WEB_PROFILE_DEFAULT.to_string()
    }
}

/// True when the profile keeps NO shared state on disk. An ephemeral profile
/// gives every surface its own in-memory context, so there is nothing for
/// concurrent writers to corrupt and no write-lock is required.
pub fn web_profile_is_ephemeral(profile: &str) -> bool {
    profile == WEB_PROFILE_TEMP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_falls_back_for_unsafe_and_empty_names() {
        for raw in [
            None,
            Some(""),
            Some("   "),
            Some("."),
            Some(".."),
            Some("a/b"),
            Some("a\\b"),
            Some("/etc/passwd"),
            Some("../../escape"),
        ] {
            assert_eq!(
                normalize_web_profile(raw),
                WEB_PROFILE_DEFAULT,
                "unsafe profile {raw:?} must fall back to default"
            );
        }
    }

    #[test]
    fn normalize_trims_so_one_directory_is_one_lock_key() {
        // The whole point of sharing this with the daemon: these must collapse
        // to one key, or two clients hold two locks over one jar.
        assert_eq!(normalize_web_profile(Some("default ")), "default");
        assert_eq!(normalize_web_profile(Some(" work")), "work");
        assert_eq!(normalize_web_profile(Some("work")), "work");
    }

    #[test]
    fn temp_profile_is_ephemeral_and_others_are_not() {
        assert!(web_profile_is_ephemeral(WEB_PROFILE_TEMP));
        assert!(!web_profile_is_ephemeral(WEB_PROFILE_DEFAULT));
        assert!(!web_profile_is_ephemeral("work"));
    }
}
