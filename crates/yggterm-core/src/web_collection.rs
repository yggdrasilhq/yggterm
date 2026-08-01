//! A COLLECTION is a Markdown file, and this is the parser that keeps that
//! promise.
//!
//! See `ychrome/docs/collections.md` for the spec. The shape:
//!
//! ```markdown
//! ---
//! id: quant-reading
//! name: Quant reading
//! kind: collection
//! ---
//!
//! Notes, as prose. The half a bookmark manager never has.
//!
//! ## Papers
//!
//! - [Active Portfolio Management](https://example.org/apm.pdf)
//! ```
//!
//! # The one rule everything here serves
//!
//! **A collection may never lose a link.** Not to an unknown frontmatter key,
//! not to a line this parser does not recognise, not to a rewrite. That is why
//! every block keeps the exact source text it came from and renders THAT back
//! unless something deliberately changed it — round-tripping is identity by
//! construction rather than by careful re-serialisation, which is the kind of
//! thing that works until someone adds a field.
//!
//! A line that looks like an item but does not parse is kept as [`Block::Raw`]
//! — prose. It stays in the file, the user still sees it, and it is still
//! there after the next edit. Dropping it would be the one unrecoverable
//! failure this format has.

use std::fmt::Write as _;

/// A frontmatter entry. Ordered, and unknown keys are preserved verbatim —
/// which is what makes the format an extension point rather than a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub value: String,
}

/// One item: a link, with the source line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub url: String,
    /// Anything after the link on the same line (an HTML comment carrying
    /// `added:`, a note). Preserved because it is the user's, not ours.
    pub trailing: String,
    raw: String,
    dirty: bool,
}

impl Item {
    pub fn new(title: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            url: url.into(),
            trailing: String::new(),
            raw: String::new(),
            dirty: true,
        }
    }

    fn render(&self) -> String {
        if !self.dirty && !self.raw.is_empty() {
            return self.raw.clone();
        }
        let mut line = format!("- [{}]({})", self.title, self.url);
        if !self.trailing.is_empty() {
            let _ = write!(line, " {}", self.trailing);
        }
        line
    }
}

/// A body block. `Raw` is load-bearing: it is how a line this parser does not
/// understand survives a rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A folder. `depth` is the heading level (2 = `##`).
    Folder { depth: usize, name: String },
    Item(Item),
    /// Prose, a blank line, or anything unrecognised — kept verbatim.
    Raw(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collection {
    /// Ordered frontmatter. Empty when the file had no `---` block.
    pub fields: Vec<Field>,
    pub blocks: Vec<Block>,
    /// Whether the source carried a frontmatter block at all. A file without
    /// one must not grow one on rewrite — that would be us editing a document
    /// the user wrote by hand.
    had_frontmatter: bool,
}

fn parse_item(line: &str) -> Option<(String, String, String)> {
    let rest = line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))?;
    let rest = rest.trim_start();
    let close = rest.find("](")?;
    if !rest.starts_with('[') {
        return None;
    }
    let title = &rest[1..close];
    let after = &rest[close + 2..];
    let end = after.find(')')?;
    let url = &after[..end];
    // A link with no URL is not a link; keeping it as prose is the honest read.
    if url.trim().is_empty() {
        return None;
    }
    Some((
        title.to_string(),
        url.to_string(),
        after[end + 1..].trim().to_string(),
    ))
}

impl Collection {
    /// A collection built from scratch rather than parsed: frontmatter, and an
    /// empty body for the caller to fill.
    ///
    /// It records that it HAS frontmatter, so an empty-bodied new collection
    /// still writes its `---` block. The one thing this must not become is a
    /// second way to spell a file — everything it produces goes back through
    /// [`Collection::parse`] on the next read and must survive it unchanged.
    pub fn with_frontmatter(fields: Vec<Field>) -> Self {
        Self {
            fields,
            blocks: Vec::new(),
            had_frontmatter: true,
        }
    }

    pub fn parse(source: &str) -> Self {
        let mut fields = Vec::new();
        let mut blocks = Vec::new();
        let mut lines = source.split('\n').peekable();
        let mut had_frontmatter = false;

        if lines.peek().map(|l| l.trim_end()) == Some("---") {
            had_frontmatter = true;
            lines.next();
            for line in lines.by_ref() {
                if line.trim_end() == "---" {
                    break;
                }
                match line.split_once(':') {
                    Some((key, value)) if !key.trim().is_empty() => fields.push(Field {
                        key: key.trim().to_string(),
                        value: value.trim().to_string(),
                    }),
                    // A frontmatter line we cannot read is still the user's.
                    // Carrying it as a keyless field keeps it in the file.
                    _ => fields.push(Field {
                        key: String::new(),
                        value: line.to_string(),
                    }),
                }
            }
        }

        for line in lines {
            let trimmed = line.trim_start();
            if let Some(hashes) = trimmed.strip_prefix("##") {
                let depth = 2 + hashes.chars().take_while(|c| *c == '#').count();
                let name = hashes.trim_start_matches('#').trim().to_string();
                blocks.push(Block::Folder { depth, name });
                continue;
            }
            if let Some((title, url, trailing)) = parse_item(trimmed) {
                blocks.push(Block::Item(Item {
                    title,
                    url,
                    trailing,
                    raw: line.to_string(),
                    dirty: false,
                }));
                continue;
            }
            blocks.push(Block::Raw(line.to_string()));
        }
        // A trailing newline in the source becomes a final empty Raw; that is
        // correct and is what makes the round trip byte-exact.
        Self {
            fields,
            blocks,
            had_frontmatter,
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        if self.had_frontmatter || !self.fields.is_empty() {
            out.push_str("---\n");
            for field in &self.fields {
                if field.key.is_empty() {
                    let _ = writeln!(out, "{}", field.value);
                } else {
                    let _ = writeln!(out, "{}: {}", field.key, field.value);
                }
            }
            out.push_str("---\n");
        }
        let rendered: Vec<String> = self
            .blocks
            .iter()
            .map(|block| match block {
                Block::Folder { depth, name } => format!("{} {name}", "#".repeat(*depth)),
                Block::Item(item) => item.render(),
                Block::Raw(raw) => raw.clone(),
            })
            .collect();
        out.push_str(&rendered.join("\n"));
        out
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.as_str())
    }

    /// Set a known field in place, or append it. Never reorders the rest —
    /// a diff should show one line.
    pub fn set_field(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some(field) = self.fields.iter_mut().find(|f| f.key == key) {
            field.value = value;
        } else {
            self.fields.push(Field {
                key: key.to_string(),
                value,
            });
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.field("id")
    }
    pub fn name(&self) -> Option<&str> {
        self.field("name")
    }
    /// `collection` unless the file says otherwise. A file with no `kind` is a
    /// collection: snapshots are the ones we write, and we always stamp them.
    pub fn is_snapshot(&self) -> bool {
        self.field("kind") == Some("snapshot")
    }

    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Item(item) => Some(item),
            _ => None,
        })
    }

    pub fn item_count(&self) -> usize {
        self.items().count()
    }

    /// Every folder, as a flat list of `(depth, name)` in document order.
    pub fn folders(&self) -> Vec<(usize, &str)> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Folder { depth, name } => Some((*depth, name.as_str())),
                _ => None,
            })
            .collect()
    }

    /// Append an item to `folder`, creating the folder if it is absent.
    ///
    /// Appends at the END of that folder's run — the last item before the next
    /// heading — so an addition never lands in the middle of what the user
    /// arranged.
    pub fn add_item(&mut self, folder: Option<&str>, item: Item) {
        let Some(folder) = folder else {
            self.blocks.push(Block::Item(item));
            return;
        };
        let start = self.blocks.iter().position(
            |b| matches!(b, Block::Folder { name, .. } if name == folder),
        );
        match start {
            Some(start) => {
                let mut insert_at = self.blocks.len();
                for (offset, block) in self.blocks.iter().enumerate().skip(start + 1) {
                    if matches!(block, Block::Folder { .. }) {
                        insert_at = offset;
                        break;
                    }
                }
                // Step back over trailing blank lines so the item joins the
                // list rather than landing after the gap below it.
                while insert_at > start + 1
                    && matches!(&self.blocks[insert_at - 1], Block::Raw(r) if r.trim().is_empty())
                {
                    insert_at -= 1;
                }
                self.blocks.insert(insert_at, Block::Item(item));
            }
            None => {
                if !matches!(self.blocks.last(), Some(Block::Raw(r)) if r.trim().is_empty()) {
                    self.blocks.push(Block::Raw(String::new()));
                }
                self.blocks.push(Block::Folder {
                    depth: 2,
                    name: folder.to_string(),
                });
                self.blocks.push(Block::Raw(String::new()));
                self.blocks.push(Block::Item(item));
            }
        }
    }

    /// Whether this collection already holds `url`. The idempotence primitive:
    /// re-importing or re-adding must not double anything.
    pub fn contains_url(&self, url: &str) -> bool {
        self.items().any(|item| item.url == url)
    }

    /// Move the item holding `url` into `folder`. `false` = no such item.
    ///
    /// The item is LIFTED, not rewritten: it keeps the source line it came in
    /// with (its title, its spacing, its trailing note), so a move shows in a
    /// diff as one line leaving one place and arriving in another rather than
    /// as a re-serialisation of somebody's link.
    pub fn move_item(&mut self, url: &str, folder: Option<&str>) -> bool {
        let Some(at) = self.blocks.iter().position(
            |block| matches!(block, Block::Item(item) if item.url == url),
        ) else {
            return false;
        };
        let Block::Item(item) = self.blocks.remove(at) else {
            unreachable!("position matched an Item");
        };
        // A blank line orphaned by the lift would accumulate over repeated
        // moves; drop it only when the removal left two blanks touching.
        if at > 0
            && at < self.blocks.len()
            && matches!(&self.blocks[at - 1], Block::Raw(r) if r.trim().is_empty())
            && matches!(&self.blocks[at], Block::Raw(r) if r.trim().is_empty())
        {
            self.blocks.remove(at);
        }
        self.add_item(folder, item);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nid: quant-reading\nname: Quant reading\nkind: collection\ntags: [finance, study]\n---\n\nNotes the user wrote.\n\n## Papers\n\n- [APM](https://example.org/apm.pdf)\n- [Signals](https://example.org/sig) <!-- added:2026-07-02 -->\n\n## Videos\n\n- [Quant life](https://example.org/v)\n";

    #[test]
    fn parse_then_write_is_byte_identical() {
        // THE property. If this ever fails, a rewrite is silently editing the
        // user's file.
        assert_eq!(Collection::parse(SAMPLE).to_markdown(), SAMPLE);
    }

    #[test]
    fn an_unknown_frontmatter_key_survives_a_rewrite() {
        let source = "---\nid: x\nsomething_we_never_heard_of: {a: 1}\n---\n\nbody\n";
        let parsed = Collection::parse(source);
        assert_eq!(parsed.field("something_we_never_heard_of"), Some("{a: 1}"));
        assert_eq!(parsed.to_markdown(), source);
    }

    #[test]
    fn a_line_that_looks_like_an_item_but_is_not_stays_as_prose() {
        // The one unrecoverable failure this format could have is losing a
        // link, so anything unparseable is kept rather than dropped.
        let source = "---\nid: x\n---\n\n- [broken link with no url]()\n- not a link at all\n- [ok](https://example.org)\n";
        let parsed = Collection::parse(source);
        assert_eq!(parsed.item_count(), 1, "only the real link is an item");
        assert_eq!(parsed.to_markdown(), source, "but nothing was lost");
    }

    #[test]
    fn a_file_with_no_frontmatter_does_not_grow_one() {
        let source = "# Just a document\n\n- [a](https://example.org)\n";
        let parsed = Collection::parse(source);
        assert!(!parsed.fields.iter().any(|f| !f.key.is_empty()));
        assert_eq!(parsed.to_markdown(), source);
    }

    #[test]
    fn folders_are_headings_and_nest_by_depth() {
        let parsed = Collection::parse(SAMPLE);
        assert_eq!(parsed.folders(), vec![(2, "Papers"), (2, "Videos")]);
        let deep = Collection::parse("## A\n### B\n#### C\n");
        assert_eq!(deep.folders(), vec![(2, "A"), (3, "B"), (4, "C")]);
    }

    #[test]
    fn an_items_trailing_note_is_the_users_and_is_kept() {
        let parsed = Collection::parse(SAMPLE);
        let signals = parsed
            .items()
            .find(|i| i.url == "https://example.org/sig")
            .unwrap();
        assert_eq!(signals.trailing, "<!-- added:2026-07-02 -->");
    }

    #[test]
    fn adding_to_an_existing_folder_lands_at_the_end_of_that_folder() {
        let mut parsed = Collection::parse(SAMPLE);
        parsed.add_item(Some("Papers"), Item::new("New paper", "https://example.org/new"));
        let out = parsed.to_markdown();
        let papers = out.split("## Videos").next().unwrap();
        assert!(papers.contains("New paper"), "must land under Papers");
        // and after the existing entries, not before them
        assert!(
            papers.find("APM").unwrap() < papers.find("New paper").unwrap(),
            "an addition must not jump ahead of what the user arranged"
        );
        // the Videos folder is untouched
        assert!(out.contains("## Videos\n\n- [Quant life](https://example.org/v)"));
    }

    #[test]
    fn adding_to_a_new_folder_creates_it_at_the_end() {
        let mut parsed = Collection::parse(SAMPLE);
        parsed.add_item(Some("Reading"), Item::new("R", "https://example.org/r"));
        let out = parsed.to_markdown();
        assert!(out.contains("## Reading"));
        assert!(out.trim_end().ends_with("- [R](https://example.org/r)"));
        assert_eq!(Collection::parse(&out).item_count(), 4);
    }

    #[test]
    fn moving_an_item_lifts_its_source_line_rather_than_rewriting_it() {
        let mut parsed = Collection::parse(SAMPLE);
        assert!(parsed.move_item("https://example.org/sig", Some("Videos")));
        let out = parsed.to_markdown();
        // The line arrived VERBATIM — trailing note and all.
        assert!(
            out.contains("- [Signals](https://example.org/sig) <!-- added:2026-07-02 -->"),
            "the user's own line must survive a move: {out}"
        );
        let videos = out.split("## Videos").nth(1).unwrap();
        assert!(videos.contains("Signals"), "it must land under Videos: {out}");
        let papers = out.split("## Videos").next().unwrap();
        assert!(!papers.contains("Signals"), "and leave Papers: {out}");
        // Nothing was lost, and the file still parses to the same item count.
        assert_eq!(Collection::parse(&out).item_count(), 3);
        // A url nobody holds moves nothing and says so.
        assert!(!parsed.move_item("https://example.org/absent", Some("Papers")));
    }

    #[test]
    fn a_collection_built_from_scratch_parses_back_to_itself() {
        let built = Collection::with_frontmatter(vec![
            Field { key: "id".to_string(), value: "x".to_string() },
            Field { key: "kind".to_string(), value: "collection".to_string() },
        ]);
        assert_eq!(built.to_markdown(), "---\nid: x\nkind: collection\n---\n");
        assert_eq!(
            Collection::parse(&built.to_markdown()).to_markdown(),
            built.to_markdown()
        );
    }

    #[test]
    fn an_untouched_item_renders_from_its_source_line() {
        // Which is what makes the round trip identity rather than a careful
        // re-serialisation that works until someone adds a field.
        let source = "- [odd   spacing](https://example.org)   \n";
        let parsed = Collection::parse(source);
        assert_eq!(parsed.to_markdown(), source);
    }

    #[test]
    fn contains_url_is_the_idempotence_primitive() {
        let parsed = Collection::parse(SAMPLE);
        assert!(parsed.contains_url("https://example.org/apm.pdf"));
        assert!(!parsed.contains_url("https://example.org/nope"));
    }

    #[test]
    fn setting_a_field_edits_in_place_and_never_reorders() {
        let mut parsed = Collection::parse(SAMPLE);
        parsed.set_field("name", "Renamed");
        parsed.set_field("updated_at", "2026-08-01T17:00:00+05:30");
        let keys: Vec<&str> = parsed.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["id", "name", "kind", "tags", "updated_at"],
            "a rename must show as ONE changed line in a diff"
        );
        assert_eq!(parsed.field("name"), Some("Renamed"));
    }

    #[test]
    fn a_snapshot_is_a_collection_with_a_kind() {
        assert!(!Collection::parse(SAMPLE).is_snapshot());
        assert!(Collection::parse("---\nid: s\nkind: snapshot\n---\n").is_snapshot());
        // and promoting it is one field, not a migration
        let mut snap = Collection::parse("---\nid: s\nkind: snapshot\n---\n");
        snap.set_field("kind", "collection");
        snap.set_field("name", "Kept");
        assert!(!snap.is_snapshot());
    }

    #[test]
    fn an_empty_file_round_trips_and_holds_nothing() {
        let parsed = Collection::parse("");
        assert_eq!(parsed.item_count(), 0);
        assert_eq!(parsed.to_markdown(), "");
    }
}
