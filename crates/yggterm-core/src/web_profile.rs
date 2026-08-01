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
//!
//! The same argument extends to a profile's METADATA ([`ProfileMeta`], the
//! `profile.json` sidecar): an avatar the GUI picker draws and an avatar the
//! surface badge draws must be the same avatar, and "may this profile be
//! deleted" must have exactly one answer no matter which surface asks. Both
//! live here for that reason — the render sites and the delete path call in,
//! they never re-derive.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

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

/// The metadata sidecar's file name, inside the profile's own jar directory.
///
/// `ychrome/docs/agent-engine.md` §7 already reserved this path for the
/// per-profile agent policy (`"agent_drive": "allow" | "deny"`). That key is
/// NOT known to this struct and must survive a rewrite by it — see
/// [`ProfileMeta::unknown_keys`].
pub const WEB_PROFILE_META_FILE: &str = "profile.json";

/// The curated avatar table: 48 single-codepoint emoji, every one of them
/// `Emoji_Presentation=Yes` so it paints as an emoji with no variation
/// selector, in a terminal cell and in a GUI label alike.
///
/// ⚠ APPEND-ONLY IS NOT ENOUGH — this table is an INPUT to
/// [`default_web_profile_emoji`], which indexes it modulo its length. Changing
/// the length or the order re-assigns every profile's default avatar. Treat a
/// reorder as a user-visible change, not a cleanup.
pub const WEB_PROFILE_AVATAR_EMOJI: [&str; 48] = [
    "🦊", "🐯", "🐼", "🐨", "🦁", "🐸", "🐙", "🦉", "🦋", "🐝", "🐧", "🐢", "🦈", "🐳", "🦄", "🐬",
    "🌵", "🌻", "🍀", "🌲", "🍁", "🌸", "🌊", "🔥", "🍎", "🍊", "🍋", "🍇", "🍉", "🍑", "🥑", "🍄",
    "🚀", "🎈", "🎨", "🎧", "📚", "🔮", "🧭", "🧩", "🌙", "⭐", "🌈", "🌟", "💎", "🎯", "🧪", "🪐",
];

/// FNV-1a (64-bit), spelled out rather than borrowed from `std`.
///
/// `DefaultHasher` is SipHash with fixed keys TODAY, but its output is
/// explicitly not guaranteed stable across Rust releases — and a profile whose
/// avatar changed when the toolchain moved would be exactly the
/// "non-determinism" the project forbids. This is 12 lines and frozen forever.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The default avatar for a profile: a pure function of its NORMALIZED name.
///
/// No randomness, no clock, no directory listing, no insertion order — the same
/// name yields the same emoji in this process, in the next one, and in a
/// different binary linking this crate. That is why the derivation lives here
/// and not at a render site.
pub fn default_web_profile_emoji(profile: &str) -> &'static str {
    let name = normalize_web_profile(Some(profile));
    let index = (fnv1a64(name.as_bytes()) % WEB_PROFILE_AVATAR_EMOJI.len() as u64) as usize;
    WEB_PROFILE_AVATAR_EMOJI[index]
}

/// The stored avatar, if the file holds one this build is willing to DRAW.
///
/// The sidecar is written by processes that never saw
/// [`web_profile_emoji_is_valid`] (`ychrome`, the agent engine — `agent_drive`
/// is specced into this same file), so validity has to be asked on the READ
/// side too. Asking it here rather than in [`ProfileMeta::from_json`] is
/// deliberate: the foreign bytes still round-trip verbatim through
/// [`ProfileMeta::to_json`], so a value this build declines to paint is
/// preserved rather than silently deleted on the next avatar edit.
///
/// One predicate, both directions: the typed field and the badge ask the same
/// question, so "what may be an avatar" cannot be answered two ways.
pub fn web_profile_stored_avatar(meta: &ProfileMeta) -> Option<&str> {
    let stored = meta.emoji.as_deref()?.trim();
    web_profile_emoji_is_valid(stored).then_some(stored)
}

/// THE avatar for a profile — the one function every render site calls.
///
/// A stored [`ProfileMeta::emoji`] wins *when it is one this build will draw*;
/// otherwise the deterministic default IS the answer. There is no third
/// branch: the old "first letter on a hardcoded gradient" fallback is gone,
/// because a fallback that only some surfaces implement is how the picker and
/// the badge came to disagree.
pub fn web_profile_avatar(profile: &str, meta: &ProfileMeta) -> String {
    match web_profile_stored_avatar(meta) {
        Some(emoji) => emoji.to_string(),
        None => default_web_profile_emoji(profile).to_string(),
    }
}

/// Profiles this build protects BY CONSTRUCTION — permanent no matter what any
/// `profile.json` says, or does not say.
///
/// The LIST is the owner, not the comparison. A surface that wants to know
/// "is this profile permanent?" calls
/// [`web_profile_is_protected_by_construction`]; re-spelling
/// `name == WEB_PROFILE_DEFAULT` at a render site is the second encoding this
/// module exists to refuse, because the day a second name joins this list the
/// re-speller keeps offering a verb the guard will refuse.
pub const WEB_PROFILE_PERMANENT: [&str; 1] = [WEB_PROFILE_DEFAULT];

/// Whether a profile is permanent regardless of its file.
///
/// The answer never consults the sidecar, so deleting
/// `~/.yggterm/web-profiles/default/profile.json` (or never having written
/// one) cannot unprotect it.
pub fn web_profile_is_protected_by_construction(profile: &str) -> bool {
    let name = normalize_web_profile(Some(profile));
    WEB_PROFILE_PERMANENT.contains(&name.as_str())
}

/// Why a permanent profile's protection cannot be toggled — the sentence the
/// picker card's row menu shows on its DISABLED "Protect profile" entry, and
/// the refusal the `server app web profile protect` verb answers with.
///
/// One string, both surfaces: the card and the verb are the same affordance
/// reached two ways, and an agent that got a different answer from the CLI than
/// the user gets from the menu would have no way to know which one is the
/// product's actual rule.
pub const WEB_PROFILE_PERMANENT_REASON: &str = "default is always protected";

/// Whether a profile refuses deletion: permanent by construction, or marked
/// protected by its owner.
pub fn web_profile_is_protected(profile: &str, meta: &ProfileMeta) -> bool {
    web_profile_is_protected_by_construction(profile) || meta.protected
}

/// Why a delete was refused. Every variant carries a sentence the UI can show
/// verbatim: a refusal the user cannot read is indistinguishable from a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebProfileDeleteRefusal {
    /// The name could not name a jar under `~/.yggterm/web-profiles/`.
    UnsafeName,
    /// The reserved ephemeral profile keeps nothing on disk.
    Ephemeral,
    /// The default profile is permanent.
    DefaultProfile,
    /// The owner marked this profile protected.
    Protected,
}

impl WebProfileDeleteRefusal {
    /// The NAMED reason, for the toast/title the refusal shows.
    pub fn reason(self) -> &'static str {
        match self {
            Self::UnsafeName => "that is not a profile name this host can delete",
            Self::Ephemeral => {
                "the temporary profile keeps nothing on disk, so there is nothing to delete"
            }
            Self::DefaultProfile => "the default profile is permanent and cannot be deleted",
            Self::Protected => "this profile is protected — unprotect it first",
        }
    }
}

/// THE delete guard. `None` ⇒ the jar may be removed.
///
/// Both the two-click ✕ in the picker and any future caller ask this; nothing
/// re-implements "is it safe to delete this profile".
pub fn web_profile_delete_refusal(
    profile: &str,
    meta: &ProfileMeta,
) -> Option<WebProfileDeleteRefusal> {
    let trimmed = profile.trim();
    if normalize_web_profile(Some(profile)) != trimmed {
        return Some(WebProfileDeleteRefusal::UnsafeName);
    }
    if web_profile_is_ephemeral(trimmed) {
        return Some(WebProfileDeleteRefusal::Ephemeral);
    }
    if web_profile_is_protected_by_construction(trimmed) {
        return Some(WebProfileDeleteRefusal::DefaultProfile);
    }
    if web_profile_is_protected(trimmed, meta) {
        return Some(WebProfileDeleteRefusal::Protected);
    }
    None
}

/// Characters that continue the grapheme cluster they follow rather than
/// starting a new one. Deliberately narrow: everything an emoji avatar can
/// legitimately contain (ZWJ sequences, variation selectors, skin tones,
/// keycaps, flag tags, combining diacriticals) and nothing else.
fn continues_emoji_cluster(c: char) -> bool {
    matches!(u32::from(c),
        0x200D                      // zero-width joiner
        | 0xFE0E | 0xFE0F           // variation selectors
        | 0x20E3                    // combining enclosing keycap
        | 0x1F3FB..=0x1F3FF         // skin-tone modifiers
        | 0x0300..=0x036F           // combining diacritical marks
        | 0xE0020..=0xE007F         // tag characters (subdivision flags)
    )
}

/// Grapheme clusters in `text`, counted well enough for emoji.
///
/// Not a full UAX #29 implementation and not trying to be — it exists so
/// "one emoji" can be checked without pulling a segmentation crate into a
/// crate that a terminal must start without.
pub fn count_emoji_clusters(text: &str) -> usize {
    let mut clusters = 0usize;
    let mut previous: Option<char> = None;
    let mut regional_half_open = false;
    for c in text.chars() {
        let regional = (0x1F1E6..=0x1F1FF).contains(&u32::from(c));
        if regional && regional_half_open {
            // Second half of a flag: joins the first, opens nothing.
            regional_half_open = false;
            previous = Some(c);
            continue;
        }
        regional_half_open = regional;
        let joins_previous = previous.is_some_and(|p| u32::from(p) == 0x200D)
            || continues_emoji_cluster(c);
        if !joins_previous {
            clusters += 1;
        }
        previous = Some(c);
    }
    clusters
}

/// Longest avatar this accepts, in bytes. A single cluster can be arbitrarily
/// long (combining marks stack without limit); the sidecar is metadata, not a
/// payload channel.
pub const WEB_PROFILE_EMOJI_MAX_BYTES: usize = 32;

/// Whether a hand-typed avatar is acceptable: one or two grapheme clusters, no
/// whitespace, no control characters, and within [`WEB_PROFILE_EMOJI_MAX_BYTES`].
pub fn web_profile_emoji_is_valid(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    if trimmed.is_empty() || trimmed.len() > WEB_PROFILE_EMOJI_MAX_BYTES {
        return false;
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    (1..=2).contains(&count_emoji_clusters(trimmed))
}

/// A profile's `profile.json`: the metadata this build understands, plus every
/// key it does not.
///
/// `extra` is not a convenience — it is the contract. `agent_drive` is specced
/// to live in this same file and is written by a DIFFERENT process; a rewrite
/// here that dropped it would silently re-grant a profile the owner had denied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileMeta {
    /// The owner's chosen avatar. `None` ⇒ [`default_web_profile_emoji`]
    /// answers at render time; nothing is written to hold a derivable default.
    pub emoji: Option<String>,
    /// The owner marked this profile undeletable.
    pub protected: bool,
    /// A label to show instead of the directory name. Identity (locks, paths)
    /// still keys on the directory name — this is decoration only.
    pub display_name: Option<String>,
    /// Keys this build does not know, preserved verbatim across a rewrite.
    extra: Map<String, Value>,
}

/// The keys [`ProfileMeta`] owns. Anything else in the file is `extra`.
const PROFILE_META_KNOWN_KEYS: [&str; 3] = ["emoji", "protected", "display_name"];

impl ProfileMeta {
    /// Parse a `profile.json` body. A body that is not a JSON object (empty,
    /// truncated, corrupt) yields defaults with NO unknown keys — the file
    /// carried no readable state to preserve.
    pub fn from_json(text: &str) -> Self {
        let Some(object) = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| match value {
                Value::Object(map) => Some(map),
                _ => None,
            })
        else {
            return Self::default();
        };
        let string_field = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let mut extra = object.clone();
        for key in PROFILE_META_KNOWN_KEYS {
            extra.remove(key);
        }
        Self {
            emoji: string_field("emoji"),
            protected: object
                .get("protected")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            display_name: string_field("display_name"),
            extra,
        }
    }

    /// Serialize back to a `profile.json` body: the unknown keys first, then
    /// this build's own, and only the ones that carry state. A default avatar
    /// is derived, never stored, so a profile that never chose one writes no
    /// `emoji` key at all.
    pub fn to_json(&self) -> String {
        let mut object = self.extra.clone();
        for key in PROFILE_META_KNOWN_KEYS {
            object.remove(key);
        }
        if let Some(emoji) = self.emoji.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
            object.insert("emoji".to_string(), Value::String(emoji.to_string()));
        }
        if self.protected {
            object.insert("protected".to_string(), Value::Bool(true));
        }
        if let Some(name) = self
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            object.insert("display_name".to_string(), Value::String(name.to_string()));
        }
        let mut body = serde_json::to_string_pretty(&Value::Object(object))
            .unwrap_or_else(|_| "{}".to_string());
        body.push('\n');
        body
    }

    /// Keys this build does not understand, as they will be written back.
    pub fn unknown_keys(&self) -> &Map<String, Value> {
        &self.extra
    }

    /// Carry an unknown key through, for tests and for callers that round-trip
    /// a value this build has no field for.
    pub fn set_unknown_key(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        if PROFILE_META_KNOWN_KEYS.contains(&key.as_str()) {
            return;
        }
        self.extra.insert(key, value);
    }

    /// The sidecar's path inside a profile jar directory.
    pub fn path_in(profile_dir: &Path) -> PathBuf {
        profile_dir.join(WEB_PROFILE_META_FILE)
    }

    /// Read a profile's metadata. A missing file (or an unreadable one) is
    /// DEFAULTS, never an error: a profile that never chose an avatar is the
    /// common case, not a failure.
    pub fn read(profile_dir: &Path) -> Self {
        match std::fs::read_to_string(Self::path_in(profile_dir)) {
            Ok(text) => Self::from_json(&text),
            Err(_) => Self::default(),
        }
    }

    /// Write a profile's metadata, creating the jar directory if the profile
    /// has not been opened yet (choosing an avatar before first use is legal).
    pub fn write(&self, profile_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(profile_dir)?;
        std::fs::write(Self::path_in(profile_dir), self.to_json())
    }
}

/// The host's web-profile jar root inside a yggterm home.
///
/// One spelling of `<home>/web-profiles`, because the GUI picker, the surface
/// badges and the `server app web profile` verbs all have to look in the same
/// directory or they are describing different products.
pub fn web_profiles_root(home: &Path) -> PathBuf {
    home.join("web-profiles")
}

/// Existing host-owned profile jars under `root`, as the picker lists them:
/// directory names, always including `default`, never the reserved ephemeral
/// `temp` (which keeps nothing on disk and gets its own card), sorted and
/// deduplicated.
pub fn list_web_profiles_in(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && let Some(name) = entry.file_name().to_str()
                && !name.is_empty()
                && !name.starts_with('.')
                && name != WEB_PROFILE_TEMP
            {
                names.push(name.to_string());
            }
        }
    }
    if !names.iter().any(|name| name == WEB_PROFILE_DEFAULT) {
        names.push(WEB_PROFILE_DEFAULT.to_string());
    }
    names.sort();
    names.dedup();
    names
}

/// Why a metadata WRITE was refused. Deletion has its own guard
/// ([`WebProfileDeleteRefusal`]); this one governs the sidecar, which a
/// permanent profile may still edit — `default` can choose an avatar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebProfileMetaRefusal {
    /// The name could not name a jar under `~/.yggterm/web-profiles/`.
    UnsafeName,
    /// The reserved ephemeral profile keeps nothing on disk.
    Ephemeral,
}

impl WebProfileMetaRefusal {
    /// The NAMED reason, shown verbatim by the picker's notice line and by the
    /// CLI's refusal.
    pub fn reason(self) -> &'static str {
        match self {
            Self::UnsafeName => "that is not a profile name this host can edit",
            Self::Ephemeral => "the temporary profile keeps nothing on disk",
        }
    }
}

/// A metadata write that did not happen: the policy refused it, or the
/// filesystem did.
///
/// The two are kept apart deliberately. A full disk reported as "that is not a
/// profile name this host can edit" sends the caller after the wrong problem,
/// and an agent driving the CLI has no screen to notice the difference on.
#[derive(Debug)]
pub enum WebProfileMetaError {
    /// The policy said no. Nothing was written.
    Refused(WebProfileMetaRefusal),
    /// The policy allowed it and the write itself failed.
    Write(std::io::Error),
}

impl std::fmt::Display for WebProfileMetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => f.write_str(refusal.reason()),
            Self::Write(error) => write!(f, "could not write the profile's metadata: {error}"),
        }
    }
}

impl std::error::Error for WebProfileMetaError {}

/// THE read-modify-write for a profile's `profile.json`, and the only one.
///
/// Read-modify-write is the contract, never a blind overwrite: `agent_drive`
/// is specced into this same file and is written by a DIFFERENT process
/// (`ychrome/docs/agent-engine.md` §7), so a rewrite that dropped it would
/// silently re-grant a profile the owner had denied. [`ProfileMeta`] carries
/// unknown keys through ([`ProfileMeta::unknown_keys`]); this function is what
/// guarantees every writer goes through that path.
///
/// It lives here rather than in the GUI because there are now two writers: the
/// picker card's row menu and the `server app web profile` verbs the agent
/// control plane drives. Two implementations of "edit a profile's sidecar"
/// would be two chances to drop the key.
///
/// Returns the metadata AS WRITTEN, so a caller can report what the file now
/// says without re-reading it.
pub fn update_profile_meta_in(
    root: &Path,
    profile: &str,
    edit: impl FnOnce(&mut ProfileMeta),
) -> Result<ProfileMeta, WebProfileMetaError> {
    let normalized = normalize_web_profile(Some(profile));
    if normalized != profile.trim() {
        return Err(WebProfileMetaError::Refused(
            WebProfileMetaRefusal::UnsafeName,
        ));
    }
    if web_profile_is_ephemeral(&normalized) {
        return Err(WebProfileMetaError::Refused(
            WebProfileMetaRefusal::Ephemeral,
        ));
    }
    let dir = root.join(&normalized);
    let mut meta = ProfileMeta::read(&dir);
    edit(&mut meta);
    meta.write(&dir).map_err(WebProfileMetaError::Write)?;
    Ok(meta)
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

    /// A scratch profile jar that removes itself. No `tempfile` dependency for
    /// three tests.
    struct ScratchProfileDir(PathBuf);

    impl ScratchProfileDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yggterm-profile-meta-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch profile dir");
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchProfileDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// ⚠ THE UNKNOWN-KEY LOCK. `agent_drive` is specced into this very file
    /// (`ychrome/docs/agent-engine.md` §7) and is written by a process that
    /// knows nothing about avatars. If a rewrite here drops it, a profile the
    /// owner set to `"deny"` silently becomes agent-drivable again — a
    /// security regression wearing the costume of a cosmetic feature.
    ///
    /// This fails if `to_json` stops re-emitting `extra`, or if `from_json`
    /// stops collecting it.
    #[test]
    fn a_rewrite_preserves_keys_this_build_never_heard_of() {
        let scratch = ScratchProfileDir::new("unknown-keys");
        let original = r#"{
            "agent_drive": "deny",
            "emoji": "🦊",
            "vault_hint": {"origin": "example.test", "strict": true},
            "future_number": 7
        }"#;
        std::fs::write(ProfileMeta::path_in(scratch.path()), original).expect("seed profile.json");

        let mut meta = ProfileMeta::read(scratch.path());
        assert_eq!(meta.emoji.as_deref(), Some("🦊"));
        // Now the GUI changes ONLY what it understands and writes back.
        meta.emoji = Some("🚀".to_string());
        meta.protected = true;
        meta.write(scratch.path()).expect("rewrite profile.json");

        let rewritten: Value =
            serde_json::from_str(&std::fs::read_to_string(ProfileMeta::path_in(scratch.path()))
                .expect("read back"))
                .expect("rewritten body is JSON");
        assert_eq!(
            rewritten.get("agent_drive").and_then(Value::as_str),
            Some("deny"),
            "the agent-drive denial must survive an avatar edit"
        );
        assert_eq!(
            rewritten.get("future_number").and_then(Value::as_i64),
            Some(7),
            "an unknown scalar must survive"
        );
        assert_eq!(
            rewritten.get("vault_hint"),
            Some(&serde_json::json!({"origin": "example.test", "strict": true})),
            "an unknown nested object must survive whole"
        );
        assert_eq!(rewritten.get("emoji").and_then(Value::as_str), Some("🚀"));
        assert_eq!(rewritten.get("protected").and_then(Value::as_bool), Some(true));

        // And the round trip is stable: reading the rewrite yields the same meta.
        assert_eq!(ProfileMeta::read(scratch.path()), meta);
    }

    /// A profile that never chose an avatar must not be given a FILE to hold a
    /// value the renderer can derive. Storing it would freeze today's table
    /// into the jar and make a table fix invisible to existing profiles.
    #[test]
    fn a_derivable_default_is_never_written_to_disk() {
        let scratch = ScratchProfileDir::new("no-default-write");
        let meta = ProfileMeta::default();
        meta.write(scratch.path()).expect("write defaults");
        let body = std::fs::read_to_string(ProfileMeta::path_in(scratch.path())).expect("read");
        assert!(
            !body.contains("emoji"),
            "an unset avatar must not be materialized: {body}"
        );
        assert!(
            !body.contains("protected"),
            "an unset protection must not be materialized: {body}"
        );
        // …and the renderer still has an answer.
        assert_eq!(
            web_profile_avatar("work", &ProfileMeta::read(scratch.path())),
            default_web_profile_emoji("work")
        );
    }

    /// Missing file is DEFAULTS, never an error — and a corrupt body is too.
    #[test]
    fn a_missing_or_corrupt_sidecar_reads_as_defaults() {
        let scratch = ScratchProfileDir::new("missing");
        assert_eq!(ProfileMeta::read(scratch.path()), ProfileMeta::default());
        assert_eq!(
            ProfileMeta::read(Path::new("/nonexistent/yggterm/profile/jar")),
            ProfileMeta::default()
        );
        std::fs::write(ProfileMeta::path_in(scratch.path()), "{not json at all")
            .expect("seed corrupt");
        assert_eq!(ProfileMeta::read(scratch.path()), ProfileMeta::default());
    }

    /// ⚠ THE DETERMINISM LOCK. The default avatar is a pure function of the
    /// normalized name — no clock, no randomness, no enumeration order. The
    /// "across two processes" half is the point: this exact assertion is
    /// re-run from a spawned second process by the shell-side lock, and the
    /// expected values are pinned here so a hash change is a RED test, not a
    /// silent re-shuffle of everyone's avatars.
    #[test]
    fn the_default_avatar_is_a_pure_function_of_the_name() {
        // Pinned. Regenerating these to match new code defeats the lock —
        // if they move, the table or the hash moved, and that is user-visible.
        for (name, expected) in [
            ("default", "🥑"),
            ("work", "🚀"),
            ("personal", "🎯"),
            ("banking", "🎈"),
        ] {
            assert_eq!(
                default_web_profile_emoji(name),
                expected,
                "the default avatar for {name:?} is frozen"
            );
        }
        // Normalization is upstream of the hash, so one jar is one avatar.
        assert_eq!(
            default_web_profile_emoji("work"),
            default_web_profile_emoji(" work ")
        );
        // Same call, thousand times, same answer.
        let once = default_web_profile_emoji("repeatable");
        for _ in 0..1000 {
            assert_eq!(default_web_profile_emoji("repeatable"), once);
        }
    }

    /// Names the cross-process lock derives on both sides.
    const CROSS_PROCESS_PROBE_NAMES: [&str; 5] =
        ["default", "work", "personal", "banking", "repeatable"];
    /// Prefix the child prints and the parent greps for.
    const CROSS_PROCESS_MARKER: &str = "CROSS-PROCESS-AVATAR";

    /// The CHILD half of the cross-process determinism lock: it only prints.
    /// It is a `#[test]` so the parent can invoke it by name through libtest.
    #[test]
    fn emits_derived_avatars_for_the_cross_process_lock() {
        for name in CROSS_PROCESS_PROBE_NAMES {
            println!(
                "{CROSS_PROCESS_MARKER} {name} {}",
                default_web_profile_emoji(name)
            );
        }
    }

    /// ⚠ THE CROSS-PROCESS LOCK. "Deterministic" has to mean across process
    /// boundaries, not just across calls: a hasher seeded per-process (the
    /// `DefaultHasher`/`RandomState` shape) passes every same-process
    /// assertion and still hands the same profile a different avatar in the
    /// GUI than in the daemon.
    ///
    /// This re-runs the derivation in a genuinely separate OS process and
    /// compares. Note the child is invoked with an EXACT filter, so the run is
    /// only meaningful if it actually executed something — hence the explicit
    /// "1 passed" assertion, without which an empty run reports `ok`.
    #[test]
    fn the_default_avatar_survives_a_process_boundary() {
        let exe = std::env::current_exe().expect("test binary path");
        let output = std::process::Command::new(&exe)
            .args([
                "web_profile::tests::emits_derived_avatars_for_the_cross_process_lock",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .output()
            .expect("spawn a second process running this same code");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        assert!(
            output.status.success(),
            "the child run failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("1 passed"),
            "the exact filter matched NOTHING — this lock proved nothing:\n{stdout}"
        );
        for name in CROSS_PROCESS_PROBE_NAMES {
            let expected = format!(
                "{CROSS_PROCESS_MARKER} {name} {}",
                default_web_profile_emoji(name)
            );
            // `contains`, not `==`: libtest prefixes the first `--nocapture`
            // line with its own "test <name> ... " progress text.
            assert!(
                stdout.lines().any(|line| line.trim().ends_with(&expected)),
                "a second process derived a different avatar for {name:?}; \
                 expected line {expected:?} in:\n{stdout}"
            );
        }
    }

    /// The table must be worth having: 48 DISTINCT entries, each a single
    /// codepoint that needs no variation selector to paint as an emoji.
    #[test]
    fn the_avatar_table_is_distinct_and_presentation_safe() {
        let mut seen = std::collections::BTreeSet::new();
        for emoji in WEB_PROFILE_AVATAR_EMOJI {
            assert!(
                seen.insert(emoji),
                "{emoji} appears twice — two profiles would collide by construction"
            );
            assert_eq!(
                emoji.chars().count(),
                1,
                "{emoji} is not a single codepoint; a ZWJ/VS sequence renders \
                 differently across terminal and GUI font stacks"
            );
            assert!(
                !emoji.contains('\u{FE0F}') && !emoji.contains('\u{FE0E}'),
                "{emoji} carries a variation selector"
            );
        }
        assert_eq!(seen.len(), 48, "the table is specced at 48 entries");
        // Spread: the table is only useful if realistic names land on many of
        // its entries rather than clumping onto a handful.
        let landed = (0..400)
            .map(|n| default_web_profile_emoji(&format!("profile-{n}")))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            landed.len() >= 40,
            "400 names reached only {} of 48 avatars — the hash is clumping",
            landed.len()
        );
    }

    /// A stored avatar wins over the derived one, and that is the ONLY other
    /// branch. An empty/whitespace stored value is not a third state.
    #[test]
    fn a_stored_avatar_wins_and_blank_is_not_a_third_state() {
        let chosen = ProfileMeta {
            emoji: Some("🐧".to_string()),
            ..ProfileMeta::default()
        };
        assert_eq!(web_profile_avatar("work", &chosen), "🐧");
        let blank = ProfileMeta {
            emoji: Some("   ".to_string()),
            ..ProfileMeta::default()
        };
        assert_eq!(
            web_profile_avatar("work", &blank),
            default_web_profile_emoji("work"),
            "a blank stored avatar falls back to the derived default, not to empty"
        );
    }

    /// ⚠ THE READ-SIDE AVATAR LOCK. The write path validates what the user
    /// TYPES, but this sidecar is written by other processes too (`agent_drive`
    /// is specced into the very same file), so a `profile.json` this build
    /// never wrote can carry a paragraph, a newline, or a 4 kB string in
    /// `emoji`. The badge pills are 9.5 px chips; whatever reaches them has to
    /// have passed the SAME predicate the typed field passes.
    ///
    /// Red when `web_profile_avatar` goes back to `Some(emoji) if
    /// !emoji.is_empty()`.
    #[test]
    fn a_foreign_sidecar_cannot_paint_an_arbitrary_string_into_a_badge() {
        for hostile in [
            "avatars are not sentences and this one is a paragraph",
            "🦊🚀🐧🦉",
            "line\nbreak",
            "\u{0}",
            "   ",
            &"🦊".repeat(20),
        ] {
            let meta = ProfileMeta::from_json(
                &serde_json::json!({ "emoji": hostile }).to_string(),
            );
            assert_eq!(
                web_profile_stored_avatar(&meta),
                None,
                "{hostile:?} is not something this build will draw"
            );
            assert_eq!(
                web_profile_avatar("work", &meta),
                default_web_profile_emoji("work"),
                "an unpaintable stored avatar falls back to the derived default, \
                 never into a 9.5px badge: {hostile:?}"
            );
        }
        // …and the foreign bytes are PRESERVED, not deleted: declining to paint
        // a value is not licence to destroy another process's write.
        let scratch = ScratchProfileDir::new("foreign-avatar");
        std::fs::write(
            ProfileMeta::path_in(scratch.path()),
            r#"{"emoji": "not an emoji at all", "agent_drive": "deny"}"#,
        )
        .expect("seed a foreign sidecar");
        let mut meta = ProfileMeta::read(scratch.path());
        meta.protected = true;
        meta.write(scratch.path()).expect("rewrite");
        let body = std::fs::read_to_string(ProfileMeta::path_in(scratch.path())).expect("read back");
        assert!(
            body.contains("not an emoji at all"),
            "the foreign avatar must survive a rewrite it did not cause: {body}"
        );
        assert!(body.contains("deny"), "and so must the agent policy: {body}");
        // A value this build DOES draw still wins.
        let good = ProfileMeta::from_json(r#"{"emoji": "🐧"}"#);
        assert_eq!(web_profile_stored_avatar(&good), Some("🐧"));
        assert_eq!(web_profile_avatar("work", &good), "🐧");
    }

    /// ⚠ THE PERMANENCE-IS-A-LIST LOCK. "Protected by construction" has ONE
    /// owner: [`WEB_PROFILE_PERMANENT`], read through
    /// [`web_profile_is_protected_by_construction`]. Every other answer in the
    /// crate — the protection predicate, the delete guard — is derived from it,
    /// so protecting a second name is one edit and every surface follows.
    #[test]
    fn permanence_is_one_list_and_every_answer_derives_from_it() {
        for name in WEB_PROFILE_PERMANENT {
            assert!(
                web_profile_is_protected_by_construction(name),
                "{name:?} is on the permanent list"
            );
            // No sidecar, an unprotecting sidecar — neither can change it.
            assert!(web_profile_is_protected(name, &ProfileMeta::default()));
            assert!(web_profile_is_protected(
                name,
                &ProfileMeta::from_json(r#"{"protected": false}"#)
            ));
            assert_eq!(
                web_profile_delete_refusal(name, &ProfileMeta::default()),
                Some(WebProfileDeleteRefusal::DefaultProfile),
                "{name:?} refuses deletion by construction"
            );
            // Normalization is upstream of the list, so the whitespace and the
            // bare spelling are one profile.
            assert!(web_profile_is_protected_by_construction(&format!(" {name} ")));
        }
        for ordinary in ["work", "personal", "banking", "lane-c-not-permanent"] {
            assert!(
                !web_profile_is_protected_by_construction(ordinary),
                "{ordinary:?} is an ordinary profile"
            );
            assert!(!web_profile_is_protected(ordinary, &ProfileMeta::default()));
            assert_eq!(
                web_profile_delete_refusal(ordinary, &ProfileMeta::default()),
                None
            );
        }
        assert!(
            WEB_PROFILE_PERMANENT.contains(&WEB_PROFILE_DEFAULT),
            "the default profile is permanent"
        );
    }

    /// ⚠ THE PROTECTION LOCK. A protected profile REFUSES deletion, with a
    /// reason the UI can show. This fails if the guard stops consulting
    /// `meta.protected` or starts answering `None` for it.
    #[test]
    fn a_protected_profile_refuses_deletion_with_a_named_reason() {
        let protected = ProfileMeta {
            protected: true,
            ..ProfileMeta::default()
        };
        let refusal = web_profile_delete_refusal("work", &protected);
        assert_eq!(
            refusal,
            Some(WebProfileDeleteRefusal::Protected),
            "a protected profile must refuse"
        );
        let reason = refusal.expect("refusal").reason();
        assert!(
            reason.contains("protected"),
            "the refusal must NAME itself; got {reason:?}"
        );
        // Unprotected, the same jar deletes.
        assert_eq!(
            web_profile_delete_refusal("work", &ProfileMeta::default()),
            None
        );
    }

    /// ⚠ THE DEFAULT-IS-PERMANENT LOCK. `default` is protected BY
    /// CONSTRUCTION: with NO `profile.json` at all — the state every fresh
    /// install is in — it still refuses. This fails the moment protection is
    /// made file-dependent.
    #[test]
    fn the_default_profile_is_undeletable_with_no_sidecar_on_disk() {
        let scratch = ScratchProfileDir::new("default-permanent");
        assert!(
            !ProfileMeta::path_in(scratch.path()).exists(),
            "this lock is about the NO-FILE case"
        );
        let meta = ProfileMeta::read(scratch.path());
        assert_eq!(meta, ProfileMeta::default());
        assert!(web_profile_is_protected(WEB_PROFILE_DEFAULT, &meta));
        assert_eq!(
            web_profile_delete_refusal(WEB_PROFILE_DEFAULT, &meta),
            Some(WebProfileDeleteRefusal::DefaultProfile)
        );
        // A file that explicitly says otherwise cannot unprotect it either.
        std::fs::write(
            ProfileMeta::path_in(scratch.path()),
            r#"{"protected": false}"#,
        )
        .expect("seed unprotect attempt");
        assert_eq!(
            web_profile_delete_refusal(WEB_PROFILE_DEFAULT, &ProfileMeta::read(scratch.path())),
            Some(WebProfileDeleteRefusal::DefaultProfile),
            "the default profile's permanence is not a file's decision"
        );
        // Trimming is upstream, so the whitespace spelling is the same profile.
        assert_eq!(
            web_profile_delete_refusal("default ", &ProfileMeta::default()),
            Some(WebProfileDeleteRefusal::DefaultProfile)
        );
    }

    /// The other two refusals, so a caller cannot reach `remove_dir_all` with
    /// a path-escaping name or with the ephemeral profile.
    #[test]
    fn unsafe_and_ephemeral_names_refuse_before_any_filesystem_call() {
        for name in ["../../escape", "a/b", "", "   ", ".."] {
            assert_eq!(
                web_profile_delete_refusal(name, &ProfileMeta::default()),
                Some(WebProfileDeleteRefusal::UnsafeName),
                "{name:?} must never reach the filesystem"
            );
        }
        assert_eq!(
            web_profile_delete_refusal(WEB_PROFILE_TEMP, &ProfileMeta::default()),
            Some(WebProfileDeleteRefusal::Ephemeral)
        );
    }

    /// Avatar validation: one or two clusters, and nothing that would smuggle
    /// a newline or a paragraph into a badge.
    #[test]
    fn avatar_validation_accepts_one_or_two_clusters_and_nothing_else() {
        for good in ["🦊", "🐧", "🏳️‍🌈", "👍🏽", "🇮🇳", "🦊🚀", "A", "1"] {
            assert!(
                web_profile_emoji_is_valid(good),
                "{good:?} is one or two clusters and must be accepted"
            );
        }
        for bad in [
            "",
            "   ",
            "🦊 🚀",
            "🦊🚀🐧",
            "abc",
            "🦊\nx",
            "🦊\u{0}",
            "avatars-are-not-sentences",
        ] {
            assert!(
                !web_profile_emoji_is_valid(bad),
                "{bad:?} must be rejected"
            );
        }
        // A pasted avatar usually arrives with a trailing newline; trimming is
        // upstream of validation, so that is one good cluster, not a refusal.
        assert!(web_profile_emoji_is_valid("🦊\n"));
        assert_eq!(count_emoji_clusters("🏳️‍🌈"), 1, "a ZWJ flag is ONE cluster");
        assert_eq!(count_emoji_clusters("🇮🇳"), 1, "a regional pair is ONE cluster");
        assert_eq!(count_emoji_clusters("🇮🇳🇯🇵"), 2, "two flags are TWO clusters");
        assert_eq!(count_emoji_clusters("👍🏽"), 1, "a skin tone joins its base");
    }

    /// A scratch PROFILES ROOT (the directory that holds per-profile jars),
    /// as opposed to [`ScratchProfileDir`], which is one jar.
    struct ScratchProfilesRoot(PathBuf);

    impl ScratchProfilesRoot {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yggterm-profiles-root-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch profiles root");
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchProfilesRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// ⚠ THE TWO-WRITER LOCK, and the reason this function exists at all.
    ///
    /// There are now two ways to change a profile's avatar: the picker card's
    /// row menu, and `server app web profile avatar` on the agent control
    /// plane. The card's write was previously the only one, so the unknown-key
    /// contract had exactly one implementation to get right. This asserts that
    /// the SHARED entry point both of them call keeps `agent_drive` — a key
    /// ychrome owns and this build has no field for — across an avatar change
    /// AND a protection toggle.
    ///
    /// It also pins the return value: the caller is told what the file now
    /// says without re-reading it, which is what the CLI reports.
    #[test]
    fn the_shared_write_path_preserves_a_key_neither_writer_understands() {
        let root = ScratchProfilesRoot::new("two-writer");
        let jar = root.path().join("work");
        std::fs::create_dir_all(&jar).expect("jar");
        std::fs::write(
            ProfileMeta::path_in(&jar),
            r#"{"agent_drive": "deny", "emoji": "🦊"}"#,
        )
        .expect("seed");

        let written = update_profile_meta_in(root.path(), "work", |meta| {
            meta.emoji = Some("🚀".to_string())
        })
        .expect("an avatar edit on an ordinary profile is allowed");
        assert_eq!(written.emoji.as_deref(), Some("🚀"));
        assert_eq!(
            written.unknown_keys().get("agent_drive"),
            Some(&Value::String("deny".to_string())),
            "the returned meta must still carry the key it does not understand"
        );

        let toggled = update_profile_meta_in(root.path(), "work", |meta| meta.protected = true)
            .expect("a protection toggle on an ordinary profile is allowed");
        assert!(toggled.protected);

        let body: Value = serde_json::from_str(
            &std::fs::read_to_string(ProfileMeta::path_in(&jar)).expect("read back"),
        )
        .expect("json");
        assert_eq!(
            body.get("agent_drive").and_then(Value::as_str),
            Some("deny"),
            "TWO writes through the shared path and the agent-drive denial must still be there — \
             a dropped key silently re-grants a profile the owner denied"
        );
        assert_eq!(body.get("emoji").and_then(Value::as_str), Some("🚀"));
        assert_eq!(body.get("protected").and_then(Value::as_bool), Some(true));
    }

    /// A profile that has never been opened may still choose an avatar: the
    /// write creates the jar. (The picker allows this; the CLI must too.)
    #[test]
    fn a_write_creates_the_jar_for_a_profile_that_was_never_opened() {
        let root = ScratchProfilesRoot::new("create-jar");
        update_profile_meta_in(root.path(), "fresh", |meta| meta.emoji = Some("🐧".to_string()))
            .expect("write");
        assert_eq!(
            ProfileMeta::read(&root.path().join("fresh")).emoji.as_deref(),
            Some("🐧")
        );
    }

    /// The write REFUSES what the picker refuses, by name. An unsafe name must
    /// not be normalized into a different profile's jar — silently editing
    /// `default` because the caller typed `../default` is the worst outcome.
    #[test]
    fn the_shared_write_path_refuses_unsafe_and_ephemeral_names() {
        let root = ScratchProfilesRoot::new("refusals");
        for unsafe_name in ["", "   ", ".", "..", "a/b", "../default", "/etc/passwd"] {
            let error = update_profile_meta_in(root.path(), unsafe_name, |meta| {
                meta.emoji = Some("🚀".to_string())
            })
            .expect_err("an unsafe name must be refused, never normalized into another jar");
            assert!(
                matches!(
                    error,
                    WebProfileMetaError::Refused(WebProfileMetaRefusal::UnsafeName)
                ),
                "{unsafe_name:?} gave {error:?}"
            );
        }
        let error = update_profile_meta_in(root.path(), WEB_PROFILE_TEMP, |meta| {
            meta.emoji = Some("🚀".to_string())
        })
        .expect_err("the ephemeral profile keeps nothing on disk");
        assert!(matches!(
            error,
            WebProfileMetaError::Refused(WebProfileMetaRefusal::Ephemeral)
        ));
        assert_eq!(
            error.to_string(),
            "the temporary profile keeps nothing on disk",
            "the refusal a user reads and the refusal an agent reads are one string"
        );
        // Nothing was created for any of them.
        assert!(
            !root.path().join(WEB_PROFILE_TEMP).exists(),
            "a refused write must not create a jar"
        );
    }

    /// The enumeration the picker draws and the enumeration the CLI reports
    /// are one function: `default` always present, `temp` never, dotfiles and
    /// plain files skipped, sorted.
    #[test]
    fn listing_profiles_matches_what_the_picker_draws() {
        let root = ScratchProfilesRoot::new("list");
        for dir in ["work", "default", WEB_PROFILE_TEMP, ".hidden", "agent-1"] {
            std::fs::create_dir_all(root.path().join(dir)).expect("jar");
        }
        std::fs::write(root.path().join("not-a-jar.txt"), "x").expect("stray file");
        assert_eq!(
            list_web_profiles_in(root.path()),
            vec![
                "agent-1".to_string(),
                "default".to_string(),
                "work".to_string()
            ]
        );
        // An empty (or missing) root still offers the default card.
        let empty = ScratchProfilesRoot::new("list-empty");
        assert_eq!(
            list_web_profiles_in(empty.path()),
            vec![WEB_PROFILE_DEFAULT.to_string()]
        );
        assert_eq!(
            list_web_profiles_in(Path::new("/nonexistent/yggterm/web-profiles")),
            vec![WEB_PROFILE_DEFAULT.to_string()]
        );
    }

    /// The jar root has one spelling. If this drifts, the CLI and the picker
    /// read different directories and every answer either gives is a lie about
    /// the other.
    #[test]
    fn the_profiles_root_is_the_documented_path() {
        assert_eq!(
            web_profiles_root(Path::new("/home/user/.yggterm")),
            PathBuf::from("/home/user/.yggterm/web-profiles")
        );
    }
}
