//! emd-renderer — the markdown-superset DOCUMENT MODEL and parser.
//!
//! Spec: `docs/spec-emd-renderer.md` (yggterm repo). One sentence: source
//! markdown parses into ONE typed block tree; every consumer (yedit's
//! document surface today; paper, the notion-class app, and ztlkn, the
//! zettelkasten app, next) renders and edits THAT tree, and edits splice
//! back into the SOURCE — the source file is the document, and lossless
//! round-trip is the invariant that makes fluid WYSIWYG trustworthy.
//!
//! Layering (deliberate):
//! - THIS crate: model + parse + source-range mapping. Pure, UI-free — no
//!   Dioxus, no theme. Server-side consumers (a future ztlkn graph indexer)
//!   can depend on it without a UI stack.
//! - The Dioxus RENDER of these blocks still lives in yggterm-shell
//!   (`md_block_node` / `md_inline_nodes` + `document_reading_typography`);
//!   extracting it is the spec's next seam, once the render stops changing
//!   weekly.
//! - Superset grammar (wikilinks, tags, callouts, tasks, frontmatter) grows
//!   HERE as new typed nodes — never as post-hoc string hacks in a renderer.
//!   A new node variant deliberately BREAKS every renderer's match until it
//!   decides how to draw it (unknown-widget-fails-loud, the repo taste).
//!
//! Security stance carried over from the shell: NEVER innerHTML — raw HTML
//! blocks/spans in the source are dropped by construction; note-derived
//! content must not reach the shell's JS context.

#[derive(Debug, Clone, PartialEq)]
pub enum MdInline {
    Text(String),
    Code(String),
    Strong(Vec<MdInline>),
    Emphasis(Vec<MdInline>),
    Strikethrough(Vec<MdInline>),
    Link {
        href: String,
        children: Vec<MdInline>,
    },
    HardBreak,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MdBlock {
    Heading {
        level: u8,
        children: Vec<MdInline>,
    },
    Paragraph(Vec<MdInline>),
    CodeBlock(String),
    BlockQuote(Vec<MdBlock>),
    List {
        ordered: bool,
        items: Vec<Vec<MdBlock>>,
    },
    Table {
        header: Vec<Vec<MdInline>>,
        rows: Vec<Vec<Vec<MdInline>>>,
    },
    Rule,
}

pub fn parse_markdown_blocks(source: &str) -> Vec<MdBlock> {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(source, options);

    // One growing tree, folded from the event stream. `block_stack` holds the
    // containers currently open (quote bodies, list items); `inline_stack`
    // the inline spans (strong/emphasis/link).
    let mut root: Vec<MdBlock> = Vec::new();
    let mut block_stack: Vec<Vec<MdBlock>> = Vec::new();
    let mut inline: Vec<MdInline> = Vec::new();
    let mut inline_stack: Vec<(u8, String, Vec<MdInline>)> = Vec::new(); // (kind, href, saved)
    let mut list_stack: Vec<(bool, Vec<Vec<MdBlock>>)> = Vec::new();
    let mut table_header: Vec<Vec<MdInline>> = Vec::new();
    let mut table_rows: Vec<Vec<Vec<MdInline>>> = Vec::new();
    let mut table_cells: Vec<Vec<MdInline>> = Vec::new();
    let mut in_table_head = false;
    let mut in_table = false;
    let mut code_block: Option<String> = None;
    let mut heading_level: Option<u8> = None;

    let heading_number = |level: HeadingLevel| -> u8 {
        match level {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        }
    };
    fn sink<'a>(
        root: &'a mut Vec<MdBlock>,
        block_stack: &'a mut Vec<Vec<MdBlock>>,
    ) -> &'a mut Vec<MdBlock> {
        block_stack.last_mut().unwrap_or(root)
    }

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading_level = Some(heading_number(level));
                inline.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let children = std::mem::take(&mut inline);
                let level = heading_level.take().unwrap_or(1);
                sink(&mut root, &mut block_stack).push(MdBlock::Heading { level, children });
            }
            Event::Start(Tag::Paragraph) => inline.clear(),
            Event::End(TagEnd::Paragraph) => {
                let children = std::mem::take(&mut inline);
                if in_table {
                    // Loose table cells parse as paragraphs; fold into the cell.
                    table_cells.last_mut().map(|cell| cell.extend(children));
                } else {
                    sink(&mut root, &mut block_stack).push(MdBlock::Paragraph(children));
                }
            }
            Event::Start(Tag::CodeBlock(_)) => code_block = Some(String::new()),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(text) = code_block.take() {
                    sink(&mut root, &mut block_stack).push(MdBlock::CodeBlock(text));
                }
            }
            Event::Start(Tag::BlockQuote(_)) => block_stack.push(Vec::new()),
            Event::End(TagEnd::BlockQuote(_)) => {
                let body = block_stack.pop().unwrap_or_default();
                sink(&mut root, &mut block_stack).push(MdBlock::BlockQuote(body));
            }
            Event::Start(Tag::List(start)) => list_stack.push((start.is_some(), Vec::new())),
            Event::End(TagEnd::List(_)) => {
                if let Some((ordered, items)) = list_stack.pop() {
                    sink(&mut root, &mut block_stack).push(MdBlock::List { ordered, items });
                }
            }
            Event::Start(Tag::Item) => block_stack.push(Vec::new()),
            Event::End(TagEnd::Item) => {
                let mut body = block_stack.pop().unwrap_or_default();
                // A tight list item's text arrives as bare inlines, not a
                // paragraph — flush whatever inline content is pending.
                if !inline.is_empty() {
                    body.insert(0, MdBlock::Paragraph(std::mem::take(&mut inline)));
                }
                if let Some((_ordered, items)) = list_stack.last_mut() {
                    items.push(body);
                }
            }
            Event::Start(Tag::Table(_)) => {
                in_table = true;
                table_header.clear();
                table_rows.clear();
            }
            Event::End(TagEnd::Table) => {
                in_table = false;
                sink(&mut root, &mut block_stack).push(MdBlock::Table {
                    header: std::mem::take(&mut table_header),
                    rows: std::mem::take(&mut table_rows),
                });
            }
            Event::Start(Tag::TableHead) => {
                in_table_head = true;
                table_cells.clear();
            }
            Event::End(TagEnd::TableHead) => {
                in_table_head = false;
                table_header = std::mem::take(&mut table_cells);
            }
            Event::Start(Tag::TableRow) => table_cells.clear(),
            Event::End(TagEnd::TableRow) => {
                table_rows.push(std::mem::take(&mut table_cells));
            }
            Event::Start(Tag::TableCell) => {
                table_cells.push(Vec::new());
                inline.clear();
            }
            Event::End(TagEnd::TableCell) => {
                let content = std::mem::take(&mut inline);
                if let Some(cell) = table_cells.last_mut() {
                    cell.extend(content);
                }
            }
            Event::Start(Tag::Strong) => {
                inline_stack.push((0, String::new(), std::mem::take(&mut inline)));
            }
            Event::End(TagEnd::Strong) => {
                if let Some((_, _, saved)) = inline_stack.pop() {
                    let children = std::mem::replace(&mut inline, saved);
                    inline.push(MdInline::Strong(children));
                }
            }
            Event::Start(Tag::Emphasis) => {
                inline_stack.push((1, String::new(), std::mem::take(&mut inline)));
            }
            Event::End(TagEnd::Emphasis) => {
                if let Some((_, _, saved)) = inline_stack.pop() {
                    let children = std::mem::replace(&mut inline, saved);
                    inline.push(MdInline::Emphasis(children));
                }
            }
            Event::Start(Tag::Strikethrough) => {
                inline_stack.push((2, String::new(), std::mem::take(&mut inline)));
            }
            Event::End(TagEnd::Strikethrough) => {
                if let Some((_, _, saved)) = inline_stack.pop() {
                    let children = std::mem::replace(&mut inline, saved);
                    inline.push(MdInline::Strikethrough(children));
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                inline_stack.push((3, dest_url.to_string(), std::mem::take(&mut inline)));
            }
            Event::End(TagEnd::Link) => {
                if let Some((_, href, saved)) = inline_stack.pop() {
                    let children = std::mem::replace(&mut inline, saved);
                    inline.push(MdInline::Link { href, children });
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                inline_stack.push((4, dest_url.to_string(), std::mem::take(&mut inline)));
            }
            Event::End(TagEnd::Image) => {
                if let Some((_, href, saved)) = inline_stack.pop() {
                    let mut children = std::mem::replace(&mut inline, saved);
                    if children.is_empty() {
                        children.push(MdInline::Text("image".to_string()));
                    }
                    inline.push(MdInline::Text("🖼 ".to_string()));
                    inline.push(MdInline::Link { href, children });
                }
            }
            Event::Text(text) => {
                if let Some(code) = code_block.as_mut() {
                    code.push_str(&text);
                } else {
                    inline.push(MdInline::Text(text.to_string()));
                }
            }
            Event::Code(code) => inline.push(MdInline::Code(code.to_string())),
            Event::SoftBreak => inline.push(MdInline::Text(" ".to_string())),
            Event::HardBreak => inline.push(MdInline::HardBreak),
            Event::Rule => sink(&mut root, &mut block_stack).push(MdBlock::Rule),
            Event::TaskListMarker(done) => {
                inline.push(MdInline::Text(if done { "☑ " } else { "☐ " }.to_string()));
            }
            // Raw HTML never reaches the DOM — dropped, not escaped-and-shown.
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(name) => {
                inline.push(MdInline::Text(format!("[{name}]")));
            }
            _ => {}
        }
    }
    if !inline.is_empty() {
        root.push(MdBlock::Paragraph(inline));
    }
    root
}

/// Source byte range of every TOP-LEVEL markdown block, in document order —
/// the substrate for block click-to-edit ([[campaign-libyggterm]] Phase 4):
/// clicking block N swaps in a mini-editor over `source[ranges[N]]`, and the
/// commit splices exactly that range. A second offset-iter pass rather than
/// threading ranges through the fold: the fold's shape stays untouched, and
/// the zip is checked (len mismatch ⇒ editing disabled, never a wrong splice).
pub fn top_level_block_ranges(source: &str) -> Vec<std::ops::Range<usize>> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    // Depth over the container tags that produce ROOT blocks in the fold.
    // Item/TableRow/etc. never occur at top level, so they need no counting:
    // anything inside them is already under an open List/Table.
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(
                Tag::Heading { .. }
                | Tag::Paragraph
                | Tag::CodeBlock(_)
                | Tag::BlockQuote(_)
                | Tag::List(_)
                | Tag::Table(_),
            ) => {
                if depth == 0 {
                    ranges.push(range.clone());
                }
                depth += 1;
            }
            Event::End(
                TagEnd::Heading(_)
                | TagEnd::Paragraph
                | TagEnd::CodeBlock
                | TagEnd::BlockQuote(_)
                | TagEnd::List(_)
                | TagEnd::Table,
            ) => {
                depth = depth.saturating_sub(1);
            }
            Event::Rule => {
                if depth == 0 {
                    ranges.push(range.clone());
                }
            }
            _ => {}
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use crate::*;

    // Block click-to-edit (Phase 4): the offset-iter pass must yield exactly
    // one source range per folded root block — the zip is what makes a
    // splice safe — and each range must slice the block's own source.
    #[test]
    fn top_level_block_ranges_align_with_the_folded_blocks() {
        let source = "# Title\n\nA paragraph with **bold**.\n\n- one\n- two\n\n```\ncode here\n```\n\n> quoted\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let blocks = parse_markdown_blocks(source);
        let ranges = top_level_block_ranges(source);
        assert_eq!(
            blocks.len(),
            ranges.len(),
            "one range per root block: {blocks:?} vs {ranges:?}"
        );
        assert_eq!(&source[ranges[0].clone()], "# Title\n");
        assert_eq!(&source[ranges[1].clone()], "A paragraph with **bold**.\n");
        // pulldown extends a list's span through its trailing blank line.
        assert_eq!(source[ranges[2].clone()].trim_end(), "- one\n- two");
        assert!(source[ranges[3].clone()].contains("code here"));
        assert!(source[ranges[4].clone()].contains("quoted"));
        assert_eq!(&source[ranges[5].clone()], "---\n");
        assert!(source[ranges[5].end..].contains("| a | b |"));

        // The splice a commit performs: replacing block 1 touches nothing else.
        let range = ranges[1].clone();
        let spliced = format!(
            "{}{}{}",
            &source[..range.start],
            "Rewritten paragraph.\n",
            &source[range.end..]
        );
        assert!(spliced.contains("# Title"));
        assert!(spliced.contains("Rewritten paragraph."));
        assert!(!spliced.contains("A paragraph with"));
        assert!(spliced.contains("code here"));
    }

    /// The document surface's markdown renderer: structure lands as typed
    /// blocks (the triage-board acceptance shape — a wide table), and raw
    /// HTML is DROPPED, never forwarded toward the DOM.
    #[test]
    fn markdown_blocks_parse_tables_and_drop_raw_html() {
        let source = "# Title\n\n<script>alert(1)</script>\n\n\
                      | a | b |\n|---|---|\n| 1 | **2** |\n\n- item one\n- item two\n";
        let blocks = crate::parse_markdown_blocks(source);
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, crate::MdBlock::Heading { level: 1, .. })),
            "heading missing: {blocks:?}"
        );
        let table = blocks.iter().find_map(|b| match b {
            crate::MdBlock::Table { header, rows } => Some((header.len(), rows.len())),
            _ => None,
        });
        assert_eq!(table, Some((2, 1)), "2-col 1-row table expected: {blocks:?}");
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, crate::MdBlock::List { ordered: false, items } if items.len() == 2)),
            "list missing: {blocks:?}"
        );
        let flat = format!("{blocks:?}");
        assert!(
            !flat.contains("script") && !flat.contains("alert"),
            "raw HTML must be dropped, not carried: {flat}"
        );
    }
}
