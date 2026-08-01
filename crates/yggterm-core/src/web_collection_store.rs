//! THE collection store: `~/.yggterm/web-profiles/<profile>/collections/<id>.md`.
//!
//! [`crate::web_collection`] owns what a collection FILE is; this module owns
//! where those files live, how one is named, when a snapshot is worth writing,
//! and which files a prune may delete. See `ychrome/docs/collections.md` (the
//! spec of record) — increment I2.
//!
//! # Three rules, and the code is shaped so they cannot be broken by accident
//!
//! 1. **A snapshot identical to the previous one is not written.** A browser
//!    idle for a day produces ONE snapshot, not twenty-four. Identity is the
//!    ITEM SET ([`snapshot_signature`]) and never the file bytes: two snapshots
//!    of the same tabs differ in `created_at` by construction, so a byte
//!    comparison would answer "different" every single time and the refusal
//!    would never fire.
//!
//! 2. **Collections are NEVER pruned.** That is the only difference between the
//!    two kinds that matters, so it is enforced twice and in two different
//!    ways: [`plan_snapshot_prune`] never puts a non-snapshot in its `prune`
//!    list (it reports them as `protected` instead, so the answer is auditable
//!    rather than merely absent), and the ONLY delete this store exposes —
//!    [`CollectionStore::remove_snapshot`] — re-reads the file's own `kind`
//!    immediately before unlinking and refuses a collection. There is no
//!    blanket `remove`, so a future caller cannot reach past the guard by
//!    picking the more convenient function.
//!
//! 3. **Every decision takes `now_ms`.** No function here reads a clock. The
//!    prune rules are exercised against fixed instants, the way
//!    `yggterm-server/src/automation.rs` does it.

use std::path::{Path, PathBuf};

use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

use crate::web_collection::{Block, Collection, Field, Item};
use crate::web_profile::{web_profile_dir, web_profile_dir_in};

/// The directory inside a profile jar that holds every collection file.
pub const COLLECTIONS_DIRNAME: &str = "collections";

/// The extension every collection file carries. Markdown, because the file IS
/// the export format and the user edits it in yedit.
pub const COLLECTION_EXTENSION: &str = "md";

/// Longest id this store will allocate or accept. It is a filename.
pub const COLLECTION_ID_MAX_LEN: usize = 64;

/// Default snapshot retention: 30 days.
pub const DEFAULT_SNAPSHOT_MAX_AGE_DAYS: u64 = 30;

/// Default snapshot retention: 200 per profile.
pub const DEFAULT_SNAPSHOT_MAX_COUNT: usize = 200;

const MS_PER_DAY: u64 = 24 * 60 * 60 * 1000;

/// Filenames Windows refuses to open no matter the extension. A collection
/// called "Aux" slugs to `aux`, and `aux.md` is unopenable there — so these are
/// treated as ALREADY TAKEN by [`allocate_id`] and the name gets a suffix.
/// Cheaper than discovering it during the 3.0.0 platform pass.
const RESERVED_FILE_STEMS: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

// ---------------------------------------------------------------------------
// Kind
// ---------------------------------------------------------------------------

/// A collection's `kind`. A snapshot IS a collection with a kind, not a second
/// store — so promoting one is [`promote`], one field, not a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    Collection,
    Snapshot,
}

impl CollectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collection => "collection",
            Self::Snapshot => "snapshot",
        }
    }

    /// A file with no `kind` is a COLLECTION: snapshots are the ones we write,
    /// and we always stamp them. Matches [`Collection::is_snapshot`].
    pub fn of(collection: &Collection) -> Self {
        if collection.is_snapshot() {
            Self::Snapshot
        } else {
            Self::Collection
        }
    }
}

// ---------------------------------------------------------------------------
// Ids — a name becomes a filename
// ---------------------------------------------------------------------------

/// Slugify a human name into the stem of a filename: lowercase ASCII, runs of
/// anything else collapsed to one `-`, trimmed, capped.
///
/// Pure, and deliberately lossy in a boring way — the NAME is kept verbatim in
/// the frontmatter, so the slug only has to be stable and legal, not pretty.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_dash = false;
    for ch in name.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else {
            None
        };
        match mapped {
            Some(ch) => {
                if pending_dash && !out.is_empty() {
                    out.push('-');
                }
                pending_dash = false;
                out.push(ch);
            }
            None => pending_dash = true,
        }
        if out.len() >= COLLECTION_ID_MAX_LEN {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        // A name that is entirely emoji or CJK still needs a file.
        "collection".to_string()
    } else {
        trimmed
    }
}

/// Whether an id may be used as a collection filename.
///
/// The guard that keeps `../../.ssh/authorized_keys` out of
/// [`CollectionStore::path_for`]: ASCII letters, digits, `-` and `_` only, so
/// no separator, no `.`, no `..`, and nothing a shell or a filesystem treats
/// specially.
pub fn id_is_valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= COLLECTION_ID_MAX_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Allocate an id for `name` that no existing collection holds.
///
/// Deterministic: the same name against the same set of taken ids always yields
/// the same id, and collisions suffix `-2`, `-3`, … rather than appending a
/// clock or a random value (which would make the same input produce two files
/// on two runs — the non-determinism this project forbids).
pub fn allocate_id(name: &str, taken: &[String]) -> String {
    let base = slugify(name);
    let is_free = |candidate: &str| {
        !RESERVED_FILE_STEMS.contains(&candidate)
            && !taken.iter().any(|existing| existing == candidate)
    };
    if is_free(&base) {
        return base;
    }
    for suffix in 2..=9999u32 {
        let tail = format!("-{suffix}");
        let head_len = COLLECTION_ID_MAX_LEN.saturating_sub(tail.len());
        let head = base
            .get(..head_len)
            .unwrap_or(&base)
            .trim_end_matches('-')
            .to_string();
        let candidate = format!("{head}{tail}");
        if is_free(&candidate) {
            return candidate;
        }
    }
    // 9998 collections sharing one name is not a case worth a fallible return.
    format!("{}-x", base)
}

// ---------------------------------------------------------------------------
// Timestamps — injected, never read
// ---------------------------------------------------------------------------

/// Render an instant as the frontmatter spells it: RFC 3339 at `utc_offset_secs`
/// (`2026-08-01T16:04:12+05:30`).
///
/// The offset is an ARGUMENT for the same reason `now_ms` is: a test pins a
/// string, and the CLI passes the machine's real offset. Sub-second precision is
/// dropped — a collection is not a trace.
pub fn format_timestamp(now_ms: u64, utc_offset_secs: i32) -> String {
    let seconds = (now_ms / 1000) as i64;
    let offset = UtcOffset::from_whole_seconds(utc_offset_secs).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::from_unix_timestamp(seconds)
        .map(|when| when.to_offset(offset))
        .ok()
        .and_then(|when| when.format(&Rfc3339).ok())
        .unwrap_or_else(|| format!("@{now_ms}"))
}

/// Read a frontmatter timestamp back. `None` when the value is absent or is
/// something this build cannot read — which is NOT an error, and is why
/// [`plan_snapshot_prune`] refuses to age-prune an entry whose age it does not
/// know rather than guessing one.
pub fn parse_timestamp(text: &str) -> Option<u64> {
    let when = OffsetDateTime::parse(text.trim(), &Rfc3339).ok()?;
    u64::try_from(when.unix_timestamp()).ok().map(|s| s * 1000)
}

// ---------------------------------------------------------------------------
// Frontmatter shaping
// ---------------------------------------------------------------------------

/// Tags as the frontmatter carries them: `tags: [finance, study]`. A bare
/// `finance, study` is accepted too, because the file is hand-editable.
pub fn parse_tag_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

/// The canonical spelling this build writes.
pub fn format_tag_list(tags: &[String]) -> String {
    format!("[{}]", tags.join(", "))
}

/// Every tag on a collection, in file order.
pub fn tags_of(collection: &Collection) -> Vec<String> {
    collection.field("tags").map(parse_tag_list).unwrap_or_default()
}

/// Add a tag if it is not already there. `false` = it already was, and nothing
/// was rewritten.
pub fn add_tag(collection: &mut Collection, tag: &str) -> bool {
    let tag = tag.trim();
    if tag.is_empty() {
        return false;
    }
    let mut tags = tags_of(collection);
    if tags.iter().any(|existing| existing == tag) {
        return false;
    }
    tags.push(tag.to_string());
    collection.set_field("tags", format_tag_list(&tags));
    true
}

/// Replace the collection's NOTE — the prose between the frontmatter and the
/// first folder or item.
///
/// Only that leading run is touched: prose the user wrote further down, next to
/// a folder, is theirs and stays exactly where they put it.
pub fn set_note(collection: &mut Collection, note: &str) {
    let body_starts_at = collection
        .blocks
        .iter()
        .position(|block| !matches!(block, Block::Raw(_)))
        .unwrap_or(collection.blocks.len());
    let mut replacement = vec![Block::Raw(String::new())];
    for line in note.trim_end().lines() {
        replacement.push(Block::Raw(line.to_string()));
    }
    if !note.trim().is_empty() {
        replacement.push(Block::Raw(String::new()));
    }
    collection.blocks.splice(0..body_starts_at, replacement);
}

/// The collection's note, as prose.
pub fn note_of(collection: &Collection) -> String {
    let mut lines = Vec::new();
    for block in &collection.blocks {
        match block {
            Block::Raw(raw) => lines.push(raw.as_str()),
            _ => break,
        }
    }
    lines.join("\n").trim().to_string()
}

/// What a fresh collection carries. A struct rather than eight positional
/// arguments, so a caller cannot swap `name` and `profile` silently.
#[derive(Debug, Clone, Default)]
pub struct NewCollection<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub profile: &'a str,
    pub tags: &'a [String],
    pub note: Option<&'a str>,
}

/// Build a new collection file's content. Pure: same inputs, same bytes.
pub fn build_collection(
    spec: &NewCollection<'_>,
    kind: CollectionKind,
    now_ms: u64,
    utc_offset_secs: i32,
) -> Collection {
    let stamp = format_timestamp(now_ms, utc_offset_secs);
    let mut fields = vec![Field {
        key: "id".to_string(),
        value: spec.id.to_string(),
    }];
    if let Some(name) = spec.name.map(str::trim).filter(|name| !name.is_empty()) {
        fields.push(Field {
            key: "name".to_string(),
            value: name.to_string(),
        });
    }
    fields.push(Field {
        key: "kind".to_string(),
        value: kind.as_str().to_string(),
    });
    fields.push(Field {
        key: "created_at".to_string(),
        value: stamp.clone(),
    });
    fields.push(Field {
        key: "updated_at".to_string(),
        value: stamp,
    });
    fields.push(Field {
        key: "profile".to_string(),
        value: spec.profile.to_string(),
    });
    if !spec.tags.is_empty() {
        fields.push(Field {
            key: "tags".to_string(),
            value: format_tag_list(spec.tags),
        });
    }
    let mut collection = Collection::with_frontmatter(fields);
    match spec.note.map(str::trim).filter(|note| !note.is_empty()) {
        Some(note) => set_note(&mut collection, note),
        None => collection.blocks.push(Block::Raw(String::new())),
    }
    collection
}

/// Stamp `updated_at`. Called by every mutation, so "when did this last change"
/// has one answer and a diff shows one line.
pub fn touch(collection: &mut Collection, now_ms: u64, utc_offset_secs: i32) {
    collection.set_field("updated_at", format_timestamp(now_ms, utc_offset_secs));
}

/// Promote a snapshot to a collection: `kind` and a `name`. One field edit and
/// a rename — no move, no copy, no new file.
///
/// Refuses a collection rather than silently succeeding: "promote" on something
/// already promoted is a caller mistake worth reporting, not a no-op to hide.
pub fn promote(
    collection: &mut Collection,
    name: &str,
    now_ms: u64,
    utc_offset_secs: i32,
) -> Result<(), String> {
    if !collection.is_snapshot() {
        return Err(format!(
            "this is already a collection (kind: {}), so there is nothing to promote",
            CollectionKind::of(collection).as_str()
        ));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("a promoted snapshot needs a name — that is what makes it a collection \
                    somebody meant to keep"
            .to_string());
    }
    collection.set_field("kind", CollectionKind::Collection.as_str());
    collection.set_field("name", name);
    touch(collection, now_ms, utc_offset_secs);
    Ok(())
}

// ---------------------------------------------------------------------------
// Reading a collection back
// ---------------------------------------------------------------------------

/// What `list` shows and what [`plan_snapshot_prune`] decides on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSummary {
    pub id: String,
    pub name: Option<String>,
    pub kind: CollectionKind,
    pub created_at_ms: Option<u64>,
    pub updated_at_ms: Option<u64>,
    pub tags: Vec<String>,
    pub item_count: usize,
    pub folder_count: usize,
}

pub fn summarize(id: &str, collection: &Collection) -> CollectionSummary {
    CollectionSummary {
        id: id.to_string(),
        name: collection.name().map(str::to_string),
        kind: CollectionKind::of(collection),
        created_at_ms: collection.field("created_at").and_then(parse_timestamp),
        updated_at_ms: collection.field("updated_at").and_then(parse_timestamp),
        tags: tags_of(collection),
        item_count: collection.item_count(),
        folder_count: collection.folders().len(),
    }
}

/// The `(title, url)` pairs a folder holds, in document order.
///
/// `None` = the whole collection. A named folder carries its NESTED folders
/// too: `Open all` on "Papers" opens what is filed under "Papers/2026" as well,
/// which is what a user pointing at a folder means.
pub fn items_in_folder(collection: &Collection, folder: Option<&str>) -> Vec<(String, String)> {
    let Some(folder) = folder else {
        return collection
            .items()
            .map(|item| (item.title.clone(), item.url.clone()))
            .collect();
    };
    let mut out = Vec::new();
    let mut inside: Option<usize> = None;
    for block in &collection.blocks {
        match block {
            Block::Folder { depth, name } => {
                inside = match inside {
                    Some(open_depth) if *depth > open_depth => Some(open_depth),
                    Some(_) => None,
                    None if name == folder => Some(*depth),
                    None => None,
                };
            }
            Block::Item(item) if inside.is_some() => {
                out.push((item.title.clone(), item.url.clone()));
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Snapshot dedupe — rule 1
// ---------------------------------------------------------------------------

/// A snapshot's IDENTITY: its item set, as sorted unique URLs.
///
/// The URL and not the title: a page whose `<title>` gained an unread count is
/// the same page, and treating it as a new one would put the twenty-four
/// snapshots back. The SET and not the order: dragging a tab is not a new
/// browsing session.
pub fn snapshot_signature(collection: &Collection) -> Vec<String> {
    let mut urls: Vec<String> = collection.items().map(|item| item.url.clone()).collect();
    urls.sort();
    urls.dedup();
    urls
}

/// Whether `candidate` says the same thing the previous snapshot already says.
pub fn snapshot_repeats(previous: &Collection, candidate: &Collection) -> bool {
    snapshot_signature(previous) == snapshot_signature(candidate)
}

/// THE refusal. `false` ⇒ do not write this snapshot.
///
/// An EMPTY candidate is refused outright: a surface with no tabs is not a
/// session worth a file, and the close hook fires exactly when the last tab
/// went away.
pub fn should_write_snapshot(previous: Option<&Collection>, candidate: &Collection) -> bool {
    if candidate.item_count() == 0 {
        return false;
    }
    match previous {
        Some(previous) => !snapshot_repeats(previous, candidate),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Prune — rule 2
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrunePolicy {
    pub max_age_days: u64,
    pub max_count: usize,
}

impl Default for PrunePolicy {
    fn default() -> Self {
        Self {
            max_age_days: DEFAULT_SNAPSHOT_MAX_AGE_DAYS,
            max_count: DEFAULT_SNAPSHOT_MAX_COUNT,
        }
    }
}

/// Why one snapshot is on the prune list. Carried so a `--dry-run` can say
/// WHICH rule reached each file rather than printing a bare list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneReason {
    /// Older than the policy allows.
    Age { age_days: u64 },
    /// Inside the age window, but past the per-profile count. `rank` is its
    /// 0-based position newest-first.
    OverCount { rank: usize },
}

impl PruneReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Age { .. } => "age",
            Self::OverCount { .. } => "over_count",
        }
    }
}

/// The plan. `protected` lists the collections the prune did not consider — so
/// "collections are never pruned" is something the output SHOWS rather than
/// something the reader has to take on trust.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    pub prune: Vec<(String, PruneReason)>,
    pub keep: Vec<String>,
    pub protected: Vec<String>,
}

/// Decide which snapshots to drop. Pure; `now_ms` is an argument.
///
/// Order of business:
/// 1. Anything that is not a snapshot leaves the decision immediately, into
///    `protected`. It can never appear in `prune`.
/// 2. Snapshots sort newest-first by `created_at`, ties broken by id, so the
///    plan is deterministic for a given input.
/// 3. Age: older than `max_age_days` is pruned. An entry whose `created_at`
///    this build cannot read has an UNKNOWN age and is never age-pruned —
///    guessing would delete a hand-written file on the strength of a parse
///    failure.
/// 4. Count: of what survives, everything past `max_count` is pruned. Unknown
///    timestamps sort last, so the count rule still reaches them.
pub fn plan_snapshot_prune(
    entries: &[CollectionSummary],
    now_ms: u64,
    policy: PrunePolicy,
) -> PrunePlan {
    let mut plan = PrunePlan::default();
    let mut snapshots: Vec<&CollectionSummary> = Vec::new();
    for entry in entries {
        match entry.kind {
            CollectionKind::Snapshot => snapshots.push(entry),
            CollectionKind::Collection => plan.protected.push(entry.id.clone()),
        }
    }
    snapshots.sort_by(|a, b| {
        b.created_at_ms
            .unwrap_or(0)
            .cmp(&a.created_at_ms.unwrap_or(0))
            .then_with(|| a.id.cmp(&b.id))
    });

    let max_age_ms = policy.max_age_days.saturating_mul(MS_PER_DAY);
    let mut rank = 0usize;
    for entry in snapshots {
        let age_ms = entry
            .created_at_ms
            .map(|created| now_ms.saturating_sub(created));
        if let Some(age_ms) = age_ms
            && age_ms > max_age_ms
        {
            plan.prune.push((
                entry.id.clone(),
                PruneReason::Age {
                    age_days: age_ms / MS_PER_DAY,
                },
            ));
            continue;
        }
        if rank >= policy.max_count {
            plan.prune
                .push((entry.id.clone(), PruneReason::OverCount { rank }));
            rank += 1;
            continue;
        }
        plan.keep.push(entry.id.clone());
        rank += 1;
    }
    plan
}

// ---------------------------------------------------------------------------
// The store on disk
// ---------------------------------------------------------------------------

/// One profile's collections directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionStore {
    dir: PathBuf,
}

impl CollectionStore {
    /// An explicit directory — what a test drives, and what
    /// [`Self::for_profile`] resolves to.
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The store for `profile` under an explicit jar root.
    pub fn for_profile_in(root: impl AsRef<Path>, profile: &str) -> Option<Self> {
        Some(Self::in_dir(
            web_profile_dir_in(root, profile)?.join(COLLECTIONS_DIRNAME),
        ))
    }

    /// The store for `profile` on this host. `None` for the ephemeral profile:
    /// temp browsing keeps nothing, collections included.
    pub fn for_profile(profile: &str) -> Option<Self> {
        Some(Self::in_dir(
            web_profile_dir(profile)?.join(COLLECTIONS_DIRNAME),
        ))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file an id names. `None` for an id that is not a legal filename —
    /// the guard, so no caller can address a path outside this directory.
    pub fn path_for(&self, id: &str) -> Option<PathBuf> {
        id_is_valid(id).then(|| self.dir.join(format!("{id}.{COLLECTION_EXTENSION}")))
    }

    /// Every collection id in this store, sorted. A missing directory is an
    /// EMPTY store, not an error.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some(COLLECTION_EXTENSION) {
                    return None;
                }
                let stem = path.file_stem()?.to_str()?.to_string();
                id_is_valid(&stem).then_some(stem)
            })
            .collect();
        ids.sort();
        ids
    }

    pub fn load(&self, id: &str) -> std::io::Result<Collection> {
        let path = self.path_for(id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{id:?} is not a legal collection id"),
            )
        })?;
        Ok(Collection::parse(&std::fs::read_to_string(path)?))
    }

    /// Write atomically: a temp file in the SAME directory, then a rename.
    ///
    /// Same directory because a rename across filesystems is a copy and is not
    /// atomic; pid in the temp name so two writers cannot land on one another's
    /// scratch file. A crash mid-write leaves the previous collection intact,
    /// which for a file whose whole promise is "never loses a link" is the only
    /// acceptable failure.
    pub fn save(&self, id: &str, collection: &Collection) -> std::io::Result<()> {
        let path = self.path_for(id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{id:?} is not a legal collection id"),
            )
        })?;
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self
            .dir
            .join(format!(".{id}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, ends_with_newline(collection.to_markdown()))?;
        match std::fs::rename(&tmp, &path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&tmp);
                Err(error)
            }
        }
    }

    /// Every collection in this store, summarized, sorted by id.
    pub fn list(&self) -> Vec<CollectionSummary> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let collection = self.load(&id).ok()?;
                Some(summarize(&id, &collection))
            })
            .collect()
    }

    /// The newest snapshot by `created_at` — what a fresh snapshot is compared
    /// against by [`should_write_snapshot`]. Ties break on id so the answer is
    /// deterministic.
    pub fn latest_snapshot(&self) -> Option<(String, Collection)> {
        let mut best: Option<CollectionSummary> = None;
        for entry in self.list() {
            if entry.kind != CollectionKind::Snapshot {
                continue;
            }
            let better = match &best {
                None => true,
                Some(current) => (
                    entry.created_at_ms.unwrap_or(0),
                    std::cmp::Reverse(entry.id.clone()),
                ) > (
                    current.created_at_ms.unwrap_or(0),
                    std::cmp::Reverse(current.id.clone()),
                ),
            };
            if better {
                best = Some(entry);
            }
        }
        let best = best?;
        let collection = self.load(&best.id).ok()?;
        Some((best.id, collection))
    }

    /// THE ONLY DELETE. Re-reads the file's own `kind` immediately before
    /// unlinking and refuses anything that is not a snapshot.
    ///
    /// Rule 2 lives here as much as in [`plan_snapshot_prune`]: a plan can be
    /// stale (the user promoted a snapshot between the plan and the sweep), and
    /// a caller can hand-roll a list. Both reach this function, and this
    /// function looks at the file.
    pub fn remove_snapshot(&self, id: &str) -> Result<(), String> {
        let path = self
            .path_for(id)
            .ok_or_else(|| format!("{id:?} is not a legal collection id"))?;
        let collection = self
            .load(id)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if !collection.is_snapshot() {
            return Err(format!(
                "{id} is a collection, not a snapshot — collections are never pruned"
            ));
        }
        std::fs::remove_file(&path).map_err(|error| format!("removing {}: {error}", path.display()))
    }

    /// An id for `name` that this store does not already hold.
    pub fn allocate(&self, name: &str) -> String {
        allocate_id(name, &self.ids())
    }
}

/// The ONE normalization a write applies: a non-empty file ends with a newline.
///
/// It belongs here and NOT in [`Collection::to_markdown`], which is byte-exact
/// by construction and must stay that way — but a file that ends mid-line is a
/// file every editor "changes" the moment the user opens it in yedit, which
/// would show up as a spurious diff on a collection nobody touched. Adding a
/// terminator loses nothing, which is the only test that matters for a format
/// whose promise is that it never loses a link.
fn ends_with_newline(mut body: String) -> String {
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body
}

/// Append an item, refusing a URL the collection already holds.
///
/// `false` = it was already there and nothing was rewritten. The idempotence
/// primitive every add path goes through, so `add-from-history` run twice is
/// the same file.
pub fn add_link(
    collection: &mut Collection,
    folder: Option<&str>,
    title: &str,
    url: &str,
) -> bool {
    if collection.contains_url(url) {
        return false;
    }
    let title = if title.trim().is_empty() { url } else { title };
    collection.add_item(folder, Item::new(title, url));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const IST: i32 = 5 * 3600 + 1800;
    /// 2026-08-01T16:04:12+05:30 — the instant in the spec's own example.
    const AUG_1_2026: u64 = 1_785_580_452_000;

    fn snapshot_of(urls: &[&str], created_at: &str) -> Collection {
        let mut body = format!("---\nid: s\nkind: snapshot\ncreated_at: {created_at}\n---\n\n");
        for (index, url) in urls.iter().enumerate() {
            body.push_str(&format!("- [tab {index}]({url})\n"));
        }
        Collection::parse(&body)
    }

    fn summary(id: &str, kind: CollectionKind, created_at_ms: Option<u64>) -> CollectionSummary {
        CollectionSummary {
            id: id.to_string(),
            name: None,
            kind,
            created_at_ms,
            updated_at_ms: created_at_ms,
            tags: Vec::new(),
            item_count: 1,
            folder_count: 0,
        }
    }

    // -- ids ---------------------------------------------------------------

    #[test]
    fn a_name_becomes_a_filename_and_stays_one() {
        assert_eq!(slugify("Quant reading"), "quant-reading");
        assert_eq!(slugify("  Deep   Work!! "), "deep-work");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("C++ / Rust"), "c-rust");
        assert_eq!(slugify("🦊🚀"), "collection", "a nameless slug still needs a file");
        assert!(slugify(&"x".repeat(200)).len() <= COLLECTION_ID_MAX_LEN);
        for name in ["Quant reading", "../../etc/passwd", "🦊🚀", &"x".repeat(200)] {
            assert!(id_is_valid(&slugify(name)), "{name:?} slugged to an illegal id");
        }
    }

    #[test]
    fn the_id_guard_refuses_anything_that_could_leave_the_directory() {
        for bad in ["", ".", "..", "a/b", "a\\b", "../x", "a.b", "a b", &"x".repeat(65)] {
            assert!(!id_is_valid(bad), "{bad:?} must not be a legal id");
        }
        for good in ["a", "quant-reading", "snapshot_2026", "A1"] {
            assert!(id_is_valid(good));
        }
        // …and the store refuses to build a path for one.
        let store = CollectionStore::in_dir("/tmp/nope");
        assert_eq!(store.path_for("../../etc/passwd"), None);
        assert_eq!(
            store.path_for("ok"),
            Some(PathBuf::from("/tmp/nope/ok.md"))
        );
    }

    #[test]
    fn a_collision_suffixes_deterministically_and_never_reaches_for_a_clock() {
        let taken = vec!["quant-reading".to_string(), "quant-reading-2".to_string()];
        assert_eq!(allocate_id("Quant reading", &taken), "quant-reading-3");
        // Same input, same answer, every time — twice is not two files.
        for _ in 0..50 {
            assert_eq!(allocate_id("Quant reading", &taken), "quant-reading-3");
        }
        assert_eq!(allocate_id("Quant reading", &[]), "quant-reading");
        // A suffixed id still fits in a filename.
        let long = "y".repeat(64);
        let id = allocate_id(&long, &[long.clone()]);
        assert!(id.len() <= COLLECTION_ID_MAX_LEN && id_is_valid(&id), "{id}");
    }

    #[test]
    fn a_windows_reserved_name_is_treated_as_taken() {
        // `aux.md` cannot be opened on Windows at all, and 3.0.0 is a platform
        // release. Cheaper here than there.
        assert_eq!(allocate_id("AUX", &[]), "aux-2");
        assert_eq!(allocate_id("com1", &[]), "com1-2");
        assert_eq!(allocate_id("auxiliary", &[]), "auxiliary");
    }

    // -- timestamps --------------------------------------------------------

    #[test]
    fn a_timestamp_is_rendered_at_the_offset_it_was_given_and_reads_back() {
        assert_eq!(
            format_timestamp(AUG_1_2026, IST),
            "2026-08-01T16:04:12+05:30",
            "the frontmatter spelling is pinned"
        );
        assert_eq!(format_timestamp(AUG_1_2026, 0), "2026-08-01T10:34:12Z");
        // Round trip, from either spelling: the instant is what matters.
        assert_eq!(parse_timestamp("2026-08-01T16:04:12+05:30"), Some(AUG_1_2026));
        assert_eq!(parse_timestamp("2026-08-01T10:34:12Z"), Some(AUG_1_2026));
        assert_eq!(parse_timestamp("not a date"), None);
        assert_eq!(parse_timestamp(""), None);
    }

    // -- building ----------------------------------------------------------

    #[test]
    fn a_new_collection_carries_the_spec_frontmatter_and_round_trips() {
        let tags = vec!["finance".to_string(), "study".to_string()];
        let collection = build_collection(
            &NewCollection {
                id: "quant-reading",
                name: Some("Quant reading"),
                profile: "default",
                tags: &tags,
                note: Some("Notes the user wrote."),
            },
            CollectionKind::Collection,
            AUG_1_2026,
            IST,
        );
        let markdown = collection.to_markdown();
        assert_eq!(
            markdown,
            "---\nid: quant-reading\nname: Quant reading\nkind: collection\n\
             created_at: 2026-08-01T16:04:12+05:30\nupdated_at: 2026-08-01T16:04:12+05:30\n\
             profile: default\ntags: [finance, study]\n---\n\nNotes the user wrote.\n"
        );
        // …and it survives the parser it was built for.
        assert_eq!(Collection::parse(&markdown).to_markdown(), markdown);
        assert_eq!(note_of(&collection), "Notes the user wrote.");
        assert_eq!(tags_of(&collection), tags);
    }

    #[test]
    fn a_note_replaces_only_the_leading_prose_and_never_the_users_own_sections() {
        let mut collection = Collection::parse(
            "---\nid: x\n---\n\nold note\n\n## Papers\n\nprose beside the folder\n\n- [a](https://a.example)\n",
        );
        set_note(&mut collection, "new note");
        let out = collection.to_markdown();
        assert!(out.contains("new note"), "{out}");
        assert!(!out.contains("old note"), "{out}");
        assert!(
            out.contains("prose beside the folder"),
            "prose the user put next to a folder is theirs: {out}"
        );
        assert!(out.contains("- [a](https://a.example)"));
    }

    #[test]
    fn a_tag_is_added_once_and_the_frontmatter_shows_one_changed_line() {
        let mut collection = Collection::parse("---\nid: x\ntags: [finance]\n---\n");
        assert!(add_tag(&mut collection, "study"));
        assert!(!add_tag(&mut collection, "study"), "a repeat tag rewrites nothing");
        assert_eq!(collection.field("tags"), Some("[finance, study]"));
        assert_eq!(tags_of(&collection), vec!["finance", "study"]);
    }

    #[test]
    fn promoting_a_snapshot_is_one_field_and_a_name() {
        let mut snapshot = snapshot_of(&["https://a.example"], "2026-07-01T00:00:00Z");
        assert!(snapshot.is_snapshot());
        promote(&mut snapshot, "Kept", AUG_1_2026, IST).expect("promote");
        assert!(!snapshot.is_snapshot());
        assert_eq!(snapshot.name(), Some("Kept"));
        assert_eq!(snapshot.field("updated_at"), Some("2026-08-01T16:04:12+05:30"));
        // The links are untouched — a promote is not a migration.
        assert_eq!(snapshot.item_count(), 1);
        // And promoting twice is an error, not a silent success.
        assert!(promote(&mut snapshot, "Again", AUG_1_2026, IST).is_err());
        let mut other = snapshot_of(&["https://a.example"], "2026-07-01T00:00:00Z");
        assert!(promote(&mut other, "   ", AUG_1_2026, IST).is_err());
    }

    // -- dedupe (rule 1) ---------------------------------------------------

    #[test]
    fn an_identical_snapshot_is_refused_even_though_its_bytes_differ() {
        // THE rule. An idle browser must produce ONE snapshot a day, not 24.
        let previous = snapshot_of(
            &["https://a.example", "https://b.example"],
            "2026-08-01T00:00:00Z",
        );
        let hourly = snapshot_of(
            &["https://a.example", "https://b.example"],
            "2026-08-01T01:00:00Z",
        );
        assert_ne!(
            previous.to_markdown(),
            hourly.to_markdown(),
            "the bytes DO differ — which is exactly why identity cannot be the bytes"
        );
        assert!(snapshot_repeats(&previous, &hourly));
        assert!(!should_write_snapshot(Some(&previous), &hourly));
    }

    #[test]
    fn reordering_or_retitling_tabs_is_not_a_new_session_but_a_new_url_is() {
        let previous = snapshot_of(
            &["https://a.example", "https://b.example"],
            "2026-08-01T00:00:00Z",
        );
        // Same set, other order, other titles.
        let reordered = Collection::parse(
            "---\nid: s\nkind: snapshot\n---\n\n\
             - [Inbox (3)](https://b.example)\n- [A](https://a.example)\n",
        );
        assert!(!should_write_snapshot(Some(&previous), &reordered));
        // One page more IS a new session.
        let grown = snapshot_of(
            &["https://a.example", "https://b.example", "https://c.example"],
            "2026-08-01T02:00:00Z",
        );
        assert!(should_write_snapshot(Some(&previous), &grown));
        // …and so is one page fewer.
        let shrunk = snapshot_of(&["https://a.example"], "2026-08-01T02:00:00Z");
        assert!(should_write_snapshot(Some(&previous), &shrunk));
    }

    #[test]
    fn the_first_snapshot_is_written_and_an_empty_one_never_is() {
        let first = snapshot_of(&["https://a.example"], "2026-08-01T00:00:00Z");
        assert!(should_write_snapshot(None, &first));
        let empty = snapshot_of(&[], "2026-08-01T00:00:00Z");
        assert!(!should_write_snapshot(None, &empty));
        assert!(!should_write_snapshot(Some(&first), &empty));
    }

    // -- prune (rule 2) ----------------------------------------------------

    #[test]
    fn a_collection_is_never_pruned_no_matter_how_old_or_how_many() {
        // The one difference between the kinds that matters. An ancient
        // collection, with the count policy at zero, still survives.
        let ancient = summary("kept-forever", CollectionKind::Collection, Some(0));
        let plan = plan_snapshot_prune(
            &[ancient],
            AUG_1_2026,
            PrunePolicy {
                max_age_days: 0,
                max_count: 0,
            },
        );
        assert!(plan.prune.is_empty(), "a collection reached the prune list: {plan:?}");
        assert_eq!(plan.protected, vec!["kept-forever".to_string()]);
        assert!(plan.keep.is_empty(), "a collection is not 'kept', it is not considered");
    }

    #[test]
    fn snapshots_past_the_age_window_are_pruned_and_the_ones_inside_it_are_not() {
        let day = MS_PER_DAY;
        let entries = vec![
            summary("fresh", CollectionKind::Snapshot, Some(AUG_1_2026 - day)),
            summary("edge", CollectionKind::Snapshot, Some(AUG_1_2026 - 30 * day)),
            summary("old", CollectionKind::Snapshot, Some(AUG_1_2026 - 31 * day)),
        ];
        let plan = plan_snapshot_prune(&entries, AUG_1_2026, PrunePolicy::default());
        assert_eq!(
            plan.prune,
            vec![("old".to_string(), PruneReason::Age { age_days: 31 })]
        );
        assert_eq!(plan.keep, vec!["fresh".to_string(), "edge".to_string()]);
    }

    #[test]
    fn over_the_count_the_oldest_go_and_the_newest_stay() {
        let day = MS_PER_DAY;
        let entries: Vec<CollectionSummary> = (0u64..5)
            .map(|n| {
                summary(
                    &format!("s{n}"),
                    CollectionKind::Snapshot,
                    Some(AUG_1_2026 - n * day),
                )
            })
            .collect();
        let plan = plan_snapshot_prune(
            &entries,
            AUG_1_2026,
            PrunePolicy {
                max_age_days: 30,
                max_count: 2,
            },
        );
        assert_eq!(plan.keep, vec!["s0".to_string(), "s1".to_string()]);
        assert_eq!(
            plan.prune
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            vec!["s2", "s3", "s4"]
        );
        assert!(matches!(plan.prune[0].1, PruneReason::OverCount { rank: 2 }));
    }

    #[test]
    fn an_unreadable_created_at_is_never_age_pruned_because_we_will_not_guess() {
        let entries = vec![
            summary("undated", CollectionKind::Snapshot, None),
            summary("dated", CollectionKind::Snapshot, Some(AUG_1_2026)),
        ];
        let plan = plan_snapshot_prune(&entries, AUG_1_2026, PrunePolicy::default());
        assert!(plan.prune.is_empty(), "{plan:?}");
        assert_eq!(plan.keep, vec!["dated".to_string(), "undated".to_string()]);
        // …but the COUNT rule still reaches it, so it cannot be a leak.
        let capped = plan_snapshot_prune(
            &entries,
            AUG_1_2026,
            PrunePolicy {
                max_age_days: 30,
                max_count: 1,
            },
        );
        assert_eq!(
            capped.prune,
            vec![("undated".to_string(), PruneReason::OverCount { rank: 1 })]
        );
    }

    #[test]
    fn the_plan_is_deterministic_for_identical_timestamps() {
        // Two snapshots written in the same second must not swap places between
        // two runs of the same prune.
        let entries = vec![
            summary("bbb", CollectionKind::Snapshot, Some(AUG_1_2026)),
            summary("aaa", CollectionKind::Snapshot, Some(AUG_1_2026)),
        ];
        let policy = PrunePolicy {
            max_age_days: 30,
            max_count: 1,
        };
        let first = plan_snapshot_prune(&entries, AUG_1_2026, policy);
        for _ in 0..20 {
            assert_eq!(plan_snapshot_prune(&entries, AUG_1_2026, policy), first);
        }
        assert_eq!(first.keep, vec!["aaa".to_string()]);
    }

    // -- folders -----------------------------------------------------------

    #[test]
    fn a_folder_carries_its_nested_folders_and_stops_at_its_sibling() {
        let collection = Collection::parse(
            "---\nid: x\n---\n\n\
             - [root](https://root.example)\n\n\
             ## Papers\n\n- [p1](https://p1.example)\n\n\
             ### 2026\n\n- [p2](https://p2.example)\n\n\
             ## Videos\n\n- [v1](https://v1.example)\n",
        );
        assert_eq!(
            items_in_folder(&collection, Some("Papers"))
                .iter()
                .map(|(_, url)| url.as_str())
                .collect::<Vec<_>>(),
            vec!["https://p1.example", "https://p2.example"]
        );
        assert_eq!(
            items_in_folder(&collection, Some("Videos"))
                .iter()
                .map(|(_, url)| url.as_str())
                .collect::<Vec<_>>(),
            vec!["https://v1.example"]
        );
        assert_eq!(items_in_folder(&collection, None).len(), 4);
        assert!(items_in_folder(&collection, Some("Nope")).is_empty());
    }

    #[test]
    fn adding_the_same_url_twice_writes_it_once() {
        let mut collection = Collection::parse("---\nid: x\n---\n");
        assert!(add_link(&mut collection, None, "A", "https://a.example"));
        assert!(!add_link(&mut collection, None, "A again", "https://a.example"));
        assert_eq!(collection.item_count(), 1);
        // An untitled link falls back to its URL rather than rendering `- []()`.
        assert!(add_link(&mut collection, None, "  ", "https://b.example"));
        assert!(collection.to_markdown().contains("- [https://b.example](https://b.example)"));
    }

    // -- the store on disk -------------------------------------------------

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yggterm-collection-store-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }
        fn store(&self) -> CollectionStore {
            CollectionStore::in_dir(&self.0)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_save_is_atomic_and_leaves_no_scratch_file_behind() {
        let scratch = Scratch::new("atomic");
        let store = scratch.store();
        let collection = build_collection(
            &NewCollection {
                id: "reading",
                name: Some("Reading"),
                profile: "default",
                tags: &[],
                note: None,
            },
            CollectionKind::Collection,
            AUG_1_2026,
            IST,
        );
        store.save("reading", &collection).expect("save");
        assert_eq!(store.ids(), vec!["reading".to_string()]);
        let temps: Vec<String> = std::fs::read_dir(store.dir())
            .expect("read dir")
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(temps.is_empty(), "a temp file survived the rename: {temps:?}");
        assert_eq!(store.load("reading").expect("load"), collection);
    }

    #[test]
    fn a_written_file_always_ends_with_a_newline_and_nothing_else_is_normalized() {
        let scratch = Scratch::new("newline");
        let store = scratch.store();
        let mut collection = build_collection(
            &NewCollection {
                id: "s",
                name: None,
                profile: "default",
                tags: &[],
                note: None,
            },
            CollectionKind::Snapshot,
            AUG_1_2026,
            IST,
        );
        add_link(&mut collection, None, "A", "https://a.example");
        assert!(
            !collection.to_markdown().ends_with('\n'),
            "this is the case the write normalizes; if the renderer changes, so does this test"
        );
        store.save("s", &collection).expect("save");
        let body = std::fs::read_to_string(store.path_for("s").expect("path")).expect("read");
        assert!(body.ends_with("- [A](https://a.example)\n"), "{body:?}");
        // …and re-saving the file that was read back changes nothing at all.
        let reread = store.load("s").expect("load");
        store.save("s", &reread).expect("re-save");
        assert_eq!(
            std::fs::read_to_string(store.path_for("s").expect("path")).expect("read"),
            body,
            "a write of an unmodified collection must be a no-op on disk"
        );
    }

    #[test]
    fn the_only_delete_refuses_a_collection_even_when_asked_directly() {
        // The second half of rule 2: a stale plan, or a hand-rolled list, still
        // cannot delete a collection, because the guard reads the FILE.
        let scratch = Scratch::new("refuse");
        let store = scratch.store();
        let keep = build_collection(
            &NewCollection {
                id: "keep",
                name: Some("Keep"),
                profile: "default",
                tags: &[],
                note: None,
            },
            CollectionKind::Collection,
            AUG_1_2026,
            IST,
        );
        store.save("keep", &keep).expect("save collection");
        let snapshot = snapshot_of(&["https://a.example"], "2026-08-01T00:00:00Z");
        store.save("snap", &snapshot).expect("save snapshot");

        let refusal = store.remove_snapshot("keep").expect_err("must refuse");
        assert!(refusal.contains("never pruned"), "{refusal}");
        assert!(store.path_for("keep").expect("path").exists());

        store.remove_snapshot("snap").expect("a snapshot deletes");
        assert_eq!(store.ids(), vec!["keep".to_string()]);
    }

    #[test]
    fn a_promoted_snapshot_survives_the_prune_that_was_planned_before_it() {
        // The stale-plan case, end to end: plan says prune, the user promotes,
        // the sweep must not take it.
        let scratch = Scratch::new("stale-plan");
        let store = scratch.store();
        let mut snapshot = snapshot_of(&["https://a.example"], "2026-01-01T00:00:00Z");
        store.save("old-snap", &snapshot).expect("save");
        let plan = plan_snapshot_prune(&store.list(), AUG_1_2026, PrunePolicy::default());
        assert_eq!(plan.prune.len(), 1, "the plan wants it gone: {plan:?}");

        promote(&mut snapshot, "Actually keep this", AUG_1_2026, IST).expect("promote");
        store.save("old-snap", &snapshot).expect("re-save");

        let refusal = store.remove_snapshot("old-snap").expect_err("must refuse");
        assert!(refusal.contains("never pruned"), "{refusal}");
        assert!(store.path_for("old-snap").expect("path").exists());
    }

    #[test]
    fn the_latest_snapshot_is_the_one_a_new_snapshot_is_compared_against() {
        let scratch = Scratch::new("latest");
        let store = scratch.store();
        store
            .save("s1", &snapshot_of(&["https://a.example"], "2026-07-01T00:00:00Z"))
            .expect("save");
        store
            .save(
                "s2",
                &snapshot_of(&["https://a.example", "https://b.example"], "2026-07-02T00:00:00Z"),
            )
            .expect("save");
        store
            .save(
                "kept",
                &Collection::parse("---\nid: kept\nkind: collection\ncreated_at: 2026-07-09T00:00:00Z\n---\n\n- [z](https://z.example)\n"),
            )
            .expect("save");
        let (id, latest) = store.latest_snapshot().expect("a snapshot");
        assert_eq!(id, "s2", "a collection must not answer 'latest snapshot'");
        assert_eq!(
            snapshot_signature(&latest),
            vec!["https://a.example".to_string(), "https://b.example".to_string()]
        );
        // And the dedupe rule then refuses the repeat.
        let repeat = snapshot_of(
            &["https://b.example", "https://a.example"],
            "2026-07-03T00:00:00Z",
        );
        assert!(!should_write_snapshot(Some(&latest), &repeat));
    }

    #[test]
    fn an_id_is_allocated_against_what_is_actually_on_disk() {
        let scratch = Scratch::new("allocate");
        let store = scratch.store();
        assert_eq!(store.allocate("Quant reading"), "quant-reading");
        store
            .save(
                "quant-reading",
                &Collection::parse("---\nid: quant-reading\n---\n"),
            )
            .expect("save");
        assert_eq!(store.allocate("Quant reading"), "quant-reading-2");
    }

    #[test]
    fn a_missing_store_is_empty_rather_than_an_error() {
        let store = CollectionStore::in_dir("/nonexistent/yggterm/collections");
        assert!(store.ids().is_empty());
        assert!(store.list().is_empty());
        assert!(store.latest_snapshot().is_none());
        assert!(store.load("anything").is_err());
    }

    #[test]
    fn the_ephemeral_profile_has_no_collections_directory() {
        assert!(
            CollectionStore::for_profile_in("/tmp/root", crate::web_profile::WEB_PROFILE_TEMP)
                .is_none(),
            "temp browsing keeps nothing, collections included"
        );
        assert_eq!(
            CollectionStore::for_profile_in("/tmp/root", "work")
                .expect("a store")
                .dir(),
            Path::new("/tmp/root/work/collections")
        );
    }
}
