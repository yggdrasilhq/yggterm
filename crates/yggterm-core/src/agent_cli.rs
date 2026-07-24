//! `AgentCliDescriptor` — the per-CLI data (harness spec §3, migration phase 1).
//!
//! One value per agent CLI, compile-time constructed, owned by the same crate
//! that owns [`SessionKind`], so every crate reads the same answers about a CLI
//! instead of re-deciding per call site.
//!
//! **Why this exists.** Before it, "how do I resume this CLI?" was answered by
//! an `is_claude` boolean inside the launch-command builder, and the same
//! question was re-answered — differently — by the readiness, replay and
//! scanner paths (`docs/spec-agent-cli-harness.md` §7 inventories the forks).
//! A fork like that is invisible until a CLI hits the arm nobody updated, which
//! is exactly how the remote-cc predicate holes were born.
//!
//! **Phase 1a (shipped)** covered invocation shape — the data behind
//! `resume_argv` / `launch_argv`. **Phase 1b (this slice)** adds the STORE half:
//! `session_store_globs` + `read_store_entry`, so "where does this CLI keep its
//! sessions, and which files are they?" is answered in exactly one place. Before
//! it, that question was re-answered at **twenty-three** call sites across all
//! three crates, each spelling the layout by hand (`"/.codex/sessions/"`,
//! `home.join(".claude").join("projects")`, `matches!(parent, ".codex" |
//! ".codex-litellm")`, and two embedded python scripts) — and each one was a
//! place a fourth CLI would have to be remembered.
//!
//! Working-signal source and prompt-signature hints remain unstubbed: an unused
//! field that nobody reads is a second source of truth waiting to drift.

use std::path::{Path, PathBuf};

use crate::session_kind::SessionKind;

/// How a CLI names an existing session on its resume invocation.
///
/// The two shapes in the fleet today. A new CLI picks one; if a third exists it
/// is added HERE, which is what makes the launch builder's job mechanical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeSelector {
    /// `claude --resume <id>` — a flag on the bare command.
    Flag(&'static str),
    /// `codex resume <id>` — a subcommand.
    Subcommand(&'static str),
}

/// One session, as read out of a CLI's OWN store (spec §3 `read_store_entry`).
///
/// Deliberately the *material* a scanner needs, not a finished row: the tree
/// builder owns row shape, and generated titles/summaries are one shared chore
/// layered on top (spec §3 "descriptors supply title *material*").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStoreEntry {
    /// The CLI's own session uuid — the one a user would type after
    /// `codex resume` / `claude -r`.
    pub session_id: String,
    /// The directory this session must be resumed in.
    pub cwd: String,
    pub modified_epoch_ms: u128,
    /// A title the CLI itself recorded in the transcript, when it records one.
    /// `None` is normal (codex keeps no in-file title) and is NOT a failure —
    /// the generated-copy store answers for those.
    pub title: Option<String>,
    /// A short human detail line the CLI's own transcript yields (CC's first
    /// user message). `None` ⇒ the tree flattener computes `short_id · cwd`.
    pub detail: Option<String>,
}

/// One agent CLI's invocation + store contract.
///
/// Deliberately NOT `PartialEq`: it carries a fn pointer, and comparing those
/// compares addresses, which says nothing useful. Descriptors are identified by
/// their `kind`.
#[derive(Debug, Clone, Copy)]
pub struct AgentCliDescriptor {
    /// The enum key this descriptor serves.
    pub kind: SessionKind,
    /// Human name for UI ("Codex", "Claude Code", …).
    pub display_name: &'static str,
    /// The executable, as invoked on the session's host.
    pub binary_name: &'static str,
    /// How an existing session id is named on resume.
    pub resume_selector: ResumeSelector,
    /// Whether resuming into a known cwd passes it explicitly.
    ///
    /// Codex is re-rooted with `-C "$PWD"` because `codex resume` otherwise
    /// resolves the session's ORIGINAL directory; Claude Code takes the
    /// process cwd. This is a real per-CLI divergence, so it is data — it used
    /// to be an `is_claude`/`has_cwd` branch pair in the builder.
    pub resume_re_roots_with_cwd: bool,
    /// True when the CLI re-derives full content from its own store on resume
    /// (all shipped CLIs do). Drives replay policy §5.4: re-derivable ⇒ the PTY
    /// is disposable and rows ride every persist.
    pub content_rederives_on_resume: bool,
    /// Where this CLI persists its OWN sessions on a host, as globs relative to
    /// `$HOME`. yggterm never writes into these — it reads them (spec §3).
    ///
    /// Supported metacharacters are `**` (any number of path segments) and `*`
    /// (any run of characters within one segment). The literal prefix of a glob
    /// is its *store root*, which is what every containment predicate keys on;
    /// see [`AgentCliDescriptor::store_roots`].
    pub session_store_globs: &'static [&'static str],
    /// Name fragments that disqualify a file the glob otherwise matched.
    /// A glob cannot express "not containing", and codex writes `.bak.` copies
    /// beside real transcripts.
    pub store_excluded_name_fragments: &'static [&'static str],
    /// Environment variable that relocates this CLI's home — the directory
    /// ABOVE the store root — when set. `None` ⇒ the store is always under
    /// `$HOME`.
    ///
    /// ⚠ **Recorded fork, not fixed by this phase:** local resolution reads
    /// `YGGTERM_CODEX_HOME` (`resolve_codex_home`), while the server's REMOTE
    /// scan reads the CLI's own `CODEX_HOME` (`resolve_remote_codex_home`,
    /// `yggterm-server/src/lib.rs`). Two names for one concept is a §7 fork, but
    /// unifying them changes which sessions a host finds — a wire change, and
    /// phase 1 is a refactor. Phase 2's four-arm matrix settles it.
    pub store_home_env_override: Option<&'static str>,
    /// Read one store file into an [`AgentStoreEntry`]. `None` when the file is
    /// not a readable session of this CLI (no identity records yet, truncated,
    /// or not ours). Feeds the cwd-tree scanner AND identity rebinding.
    pub read_store_entry: fn(&Path) -> Option<AgentStoreEntry>,
}

impl AgentCliDescriptor {
    /// Tokens for resuming `session_id`, WITHOUT the binary and without
    /// transport/env wrapping — the harness owns those (spec §3).
    ///
    /// `cwd_known` reports whether the caller established a working directory
    /// for the session; only a re-rooting CLI uses it.
    pub fn resume_tokens(&self, session_id_quoted: &str, cwd_known: bool) -> Vec<String> {
        let mut tokens = Vec::new();
        match self.resume_selector {
            ResumeSelector::Flag(flag) => tokens.push(flag.to_string()),
            ResumeSelector::Subcommand(sub) => tokens.push(sub.to_string()),
        }
        if self.resume_re_roots_with_cwd && cwd_known {
            tokens.push("-C".to_string());
            tokens.push("\"$PWD\"".to_string());
        }
        tokens.push(session_id_quoted.to_string());
        tokens
    }

    /// Tokens for the CLI's own resume PICKER (no session id).
    pub fn resume_picker_tokens(&self) -> Vec<String> {
        match self.resume_selector {
            ResumeSelector::Flag(flag) => vec![flag.to_string()],
            ResumeSelector::Subcommand(sub) => vec![sub.to_string()],
        }
    }

    /// The store roots — each glob's literal prefix — relative to `$HOME`,
    /// e.g. `.codex/sessions`. Derived, never declared twice.
    pub fn store_roots(&self) -> Vec<&'static str> {
        unique(
            self.session_store_globs
                .iter()
                .map(|glob| literal_prefix(glob)),
        )
    }

    /// The path fragments a containment test keys on, e.g. `/.codex/sessions/`.
    ///
    /// This is the one thing eleven call sites used to spell by hand. It is
    /// deliberately anchored on both sides: an unanchored `.codex/sessions`
    /// would also match `/home/x/backup-.codex/sessions/…`.
    pub fn store_path_fragments(&self) -> Vec<String> {
        self.store_roots()
            .into_iter()
            .map(|root| format!("/{root}/"))
            .collect()
    }

    /// Whether `path` lives under one of this CLI's store roots. Coarse: says
    /// nothing about whether the file is a *session* (see
    /// [`AgentCliDescriptor::store_path_is_session_file`]).
    pub fn store_path_is_under_root(&self, path: &str) -> bool {
        self.store_path_fragments()
            .iter()
            .any(|fragment| path.contains(fragment.as_str()))
    }

    /// The prefix of `path` up to and including this CLI's home directory
    /// (the segment ABOVE the store root), e.g. `/home/user/.codex`. `None` when
    /// `path` is not in this CLI's store.
    pub fn store_home_prefix_of<'a>(&self, path: &'a str) -> Option<&'a str> {
        for (root, fragment) in self
            .store_roots()
            .into_iter()
            .zip(self.store_path_fragments())
        {
            let Some(index) = path.find(fragment.as_str()) else {
                continue;
            };
            // `/.codex/sessions/` → keep `/.codex`, drop the `/sessions/` tail.
            // A root with no parent segment IS the home dir, so keep all of it.
            let keep = match root.rfind('/') {
                Some(separator) => 1 + separator,
                None => 1 + root.len(),
            };
            return path.get(..index + keep);
        }
        None
    }

    /// Whether `file_name` alone looks like one of this CLI's session files —
    /// the glob's LAST segment plus the exclusions, with no opinion about which
    /// directory the file sits in.
    ///
    /// This is the predicate for callers that already know they are inside the
    /// store and must keep working when the store has been relocated by
    /// [`AgentCliDescriptor::store_home_env_override`] (the codex tree walk
    /// runs against `resolve_codex_home()`, which may be anywhere).
    pub fn store_file_name_is_session(&self, file_name: &str) -> bool {
        if self
            .store_excluded_name_fragments
            .iter()
            .any(|fragment| file_name.contains(fragment))
        {
            return false;
        }
        self.session_store_globs.iter().any(|glob| {
            glob.rsplit('/')
                .next()
                .is_some_and(|last| segment_matches(last, file_name))
        })
    }

    /// Whether `path` is a session FILE of this CLI: under a store root, glob
    /// matched, and not an excluded name.
    pub fn store_path_is_session_file(&self, path: &str) -> bool {
        if self
            .store_excluded_name_fragments
            .iter()
            .any(|fragment| file_name_of(path).contains(fragment))
        {
            return false;
        }
        self.session_store_globs.iter().any(|glob| {
            let Some(root) = path_tail_after_root(path, literal_prefix(glob)) else {
                return false;
            };
            glob_matches(glob_tail(glob), root)
        })
    }

    /// A path inside this CLI's HOME directory that is NOT its session store —
    /// e.g. Claude Code's pid registry at `~/.claude/sessions`. The home name
    /// itself stays declared exactly once.
    pub fn home_relative_path(&self, home: &Path, tail: &str) -> Option<PathBuf> {
        Some(home.join(self.store_home_dir_names().first()?).join(tail))
    }

    /// This CLI's home directory names (`.codex`, `.claude`) — each store
    /// root's FIRST segment. What a caller holding one path component, rather
    /// than a full path, must compare against.
    pub fn store_home_dir_names(&self) -> Vec<&'static str> {
        unique(
            self.store_roots()
                .into_iter()
                .map(|root| root.split('/').next().unwrap_or(root)),
        )
    }

    /// A synthetic path that IS one of this CLI's session files — the store
    /// twin of `SchemeDescriptor::example`, derived from the glob so it cannot
    /// drift from it. Used by the coverage locks, never by product code.
    pub fn example_store_path(&self, home: &str) -> String {
        let glob = self.session_store_globs.first().copied().unwrap_or("");
        let expanded: Vec<String> = glob
            .split('/')
            .map(|segment| match segment {
                "**" => String::from("sample"),
                other => other.replace('*', "sample"),
            })
            .collect();
        format!("{home}/{}", expanded.join("/"))
    }

    /// Absolute store roots on this machine, honouring
    /// [`AgentCliDescriptor::store_home_env_override`].
    pub fn store_roots_absolute(&self, home: &Path) -> Vec<PathBuf> {
        self.store_roots()
            .into_iter()
            .map(|root| {
                let Some(env_name) = self.store_home_env_override else {
                    return home.join(root);
                };
                let Some(value) = std::env::var_os(env_name) else {
                    return home.join(root);
                };
                // The override names the CLI HOME (`~/.codex`), so it replaces
                // the root's first segment and keeps the rest (`sessions`).
                let overridden = crate::expand_tilde(PathBuf::from(value));
                match root.split_once('/') {
                    Some((_home_segment, tail)) => overridden.join(tail),
                    None => overridden,
                }
            })
            .collect()
    }
}

/// The leading run of segments in `glob` containing no metacharacter.
fn literal_prefix(glob: &'static str) -> &'static str {
    let mut end = 0;
    for segment in glob.split('/') {
        if segment.contains('*') {
            break;
        }
        end += segment.len() + 1;
    }
    glob.get(..end.saturating_sub(1)).unwrap_or("")
}

/// `glob` minus its literal prefix — the pattern applied below the store root.
fn glob_tail(glob: &'static str) -> &'static str {
    let prefix = literal_prefix(glob);
    glob.get(prefix.len() + 1..).unwrap_or("")
}

/// The part of `path` below `/<root>/`, or `None` when it is not under it.
fn path_tail_after_root<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let fragment = format!("/{root}/");
    let index = path.find(&fragment)?;
    path.get(index + fragment.len()..)
}

fn file_name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Segment-wise glob match supporting `**` (any number of segments) and `*`
/// (any run within one segment). Deliberately tiny and dependency-free — the
/// patterns it serves are declared right here in this file, not user input.
fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    segments_match(&pattern, &path)
}

fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.first() {
        None => path.is_empty(),
        Some(&"**") => {
            // `**` consumes zero or more segments; try each split point.
            (0..=path.len()).any(|take| segments_match(&pattern[1..], &path[take..]))
        }
        Some(head) => match path.first() {
            None => false,
            Some(segment) => {
                segment_matches(head, segment) && segments_match(&pattern[1..], &path[1..])
            }
        },
    }
}

fn segment_matches(pattern: &str, segment: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return pattern == segment;
    };
    let Some(rest) = segment.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    if parts.is_empty() {
        return rest.is_empty();
    }
    let (last, middle) = parts.split_last().expect("non-empty");
    let mut cursor = rest;
    for part in middle {
        let Some(index) = cursor.find(part) else {
            return false;
        };
        cursor = &cursor[index + part.len()..];
    }
    cursor.len() >= last.len() && cursor.ends_with(last)
}

/// Every agent CLI yggterm can drive. Adding a CLI without a descriptor is
/// impossible by construction: [`SessionKind::is_agent`] is derived from this
/// table (see `session_kind.rs`).
pub const AGENT_CLIS: &[AgentCliDescriptor] = &[
    AgentCliDescriptor {
        kind: SessionKind::Codex,
        display_name: "Codex",
        binary_name: "codex",
        resume_selector: ResumeSelector::Subcommand("resume"),
        // `codex resume <id>` reopens the session's ORIGINAL cwd unless
        // re-rooted; the cwd tree's whole promise is that a row opens where the
        // tree says it lives.
        resume_re_roots_with_cwd: true,
        content_rederives_on_resume: true,
        // Codex files sessions by date: `~/.codex/sessions/2026/07/25/
        // rollout-2026-07-25T…-<uuid>.jsonl`, so the depth is not fixed.
        session_store_globs: &[".codex/sessions/**/rollout-*.jsonl"],
        store_excluded_name_fragments: &[".bak."],
        store_home_env_override: Some(crate::ENV_YGGTERM_CODEX_HOME),
        read_store_entry: read_codex_store_entry,
    },
    AgentCliDescriptor {
        kind: SessionKind::CodexLiteLlm,
        display_name: "Codex-LiteLLM",
        binary_name: "codex-litellm",
        resume_selector: ResumeSelector::Subcommand("resume"),
        // ⚠ Deliberately FALSE, preserving shipped behavior exactly: the
        // pre-descriptor builder gated `-C "$PWD"` on `SessionKind::Codex`
        // alone, so the LiteLLM fork never re-rooted. Whether that was intent
        // or oversight is unverified, and phase 1 is a refactor — flipping it
        // here would be a silent behavior change riding a "no wire changes"
        // phase. Recorded for phase 2's four-arm matrix to settle.
        resume_re_roots_with_cwd: false,
        content_rederives_on_resume: true,
        session_store_globs: &[".codex-litellm/sessions/**/rollout-*.jsonl"],
        store_excluded_name_fragments: &[".bak."],
        // No override: only `resolve_codex_home` consults an env var, and it
        // relocates `.codex` alone. Preserving that exactly.
        store_home_env_override: None,
        read_store_entry: read_codex_store_entry,
    },
    AgentCliDescriptor {
        kind: SessionKind::ClaudeCode,
        display_name: "Claude Code",
        binary_name: "claude",
        resume_selector: ResumeSelector::Flag("--resume"),
        resume_re_roots_with_cwd: false,
        content_rederives_on_resume: true,
        // CC files one flat dir per cwd, the dir name being the cwd with every
        // character outside [A-Za-z0-9-] replaced: `~/.claude/projects/
        // -home-user-gh-yggterm/<session-uuid>.jsonl`. Exactly one level.
        session_store_globs: &[".claude/projects/*/*.jsonl"],
        store_excluded_name_fragments: &[],
        store_home_env_override: None,
        read_store_entry: read_claude_code_store_entry,
    },
];

fn modified_epoch_ms_of(path: &Path) -> u128 {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// Codex keeps no title in its own transcript — the generated-copy store
/// answers for those, which is why `title` is always `None` here and the
/// scanner layers a resolver on top.
fn read_codex_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let (session_id, cwd) = crate::read_codex_session_identity_fields(path)
        .ok()
        .flatten()?;
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title: None,
        detail: None,
    })
}

fn read_claude_code_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let (session_id, cwd) = crate::read_cc_session_identity_fields(path)
        .ok()
        .flatten()?;
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title: crate::read_cc_session_title(path).ok().flatten(),
        detail: crate::read_cc_session_context(path)
            .ok()
            .filter(|context| !context.trim().is_empty()),
    })
}

/// The descriptor for `kind`, or `None` for a non-agent kind.
pub fn agent_cli_descriptor(kind: SessionKind) -> Option<&'static AgentCliDescriptor> {
    AGENT_CLIS.iter().find(|descriptor| descriptor.kind == kind)
}

/// Which CLI's store `path` lives under, if any. The store roots are mutually
/// exclusive by construction (`/.codex/sessions/` is not a substring of
/// `/.codex-litellm/sessions/`), and
/// [`agent_cli_store_roots_are_mutually_exclusive`] holds a new CLI to that.
pub fn agent_cli_for_store_path(path: &str) -> Option<&'static AgentCliDescriptor> {
    AGENT_CLIS
        .iter()
        .find(|descriptor| descriptor.store_path_is_under_root(path))
}

/// Which CLI's store SESSION FILE `path` is — stricter than
/// [`agent_cli_for_store_path`]: the glob must match and the name must not be
/// excluded. This is the predicate a scanner classifies files with.
pub fn agent_cli_for_store_session_file(path: &str) -> Option<&'static AgentCliDescriptor> {
    AGENT_CLIS
        .iter()
        .find(|descriptor| descriptor.store_path_is_session_file(path))
}

/// Every store path fragment in the fleet, e.g. `/.codex/sessions/`. The list a
/// sweeper or a "do not delete this" guard must consult instead of writing its
/// own — the round-8 clipboard sweep hand-listed exactly these three.
pub fn all_agent_cli_store_path_fragments() -> Vec<String> {
    unique(
        AGENT_CLIS
            .iter()
            .flat_map(|descriptor| descriptor.store_path_fragments()),
    )
}

/// Order-preserving unique. `Vec::dedup` only drops CONSECUTIVE duplicates,
/// which silently does nothing when two CLIs declare the same root apart in the
/// table — and these lists are small enough that the quadratic scan is free.
fn unique<T: PartialEq>(items: impl Iterator<Item = T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// The codex family: two builds of ONE CLI. They share a store layout, a
/// transcript format and an identity-rebinding mechanism, and differ only in
/// which model endpoint they talk to — so predicates that key on "a codex
/// transcript" mean both.
///
/// Declared ONCE here. Before this, every such site spelled the pair by hand
/// (`path.contains("/.codex/sessions/") || path.contains("/.codex-litellm/…")`,
/// `matches!(parent, ".codex" | ".codex-litellm")`) and a third fork would have
/// had to be remembered at each one.
pub const CODEX_FAMILY: &[SessionKind] = &[SessionKind::Codex, SessionKind::CodexLiteLlm];

/// Whether `path` is under the store of any CLI in `kinds`.
pub fn store_path_is_under_any(kinds: &[SessionKind], path: &str) -> bool {
    AGENT_CLIS
        .iter()
        .filter(|descriptor| kinds.contains(&descriptor.kind))
        .any(|descriptor| descriptor.store_path_is_under_root(path))
}

/// Whether `dir_name` is the home directory name (`.codex`, `.claude`) of any
/// CLI in `kinds` — for callers that see one path COMPONENT, not a full path.
pub fn store_home_dir_name_is_any(kinds: &[SessionKind], dir_name: &str) -> bool {
    AGENT_CLIS
        .iter()
        .filter(|descriptor| kinds.contains(&descriptor.kind))
        .any(|descriptor| descriptor.store_home_dir_names().contains(&dir_name))
}

/// A predicate that is supposed to answer for every CLI store but today misses
/// one — the store-keyed twin of [`crate::agent_scheme::KNOWN_PREDICATE_HOLES`],
/// and the same burn-down contract: a hole must be recorded here or its lock
/// fails, and a hole that stops reproducing fails until its row is DELETED, so
/// the table can never go stale-green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorePredicateHole {
    /// The predicate's fn name, exactly as in the owning crate.
    pub predicate: &'static str,
    /// The CLI whose store the predicate does not cover.
    pub kind: SessionKind,
    /// When the hole was recorded/re-verified.
    pub recorded: &'static str,
    /// The user-visible consequence, one line.
    pub consequence: &'static str,
}

pub const KNOWN_STORE_PREDICATE_HOLES: &[StorePredicateHole] = &[StorePredicateHole {
    predicate: "selected_path_should_expand_ancestors",
    kind: SessionKind::ClaudeCode,
    recorded: "2026-07-25",
    consequence: "selecting a CC transcript expands its store dirs as if they were tree \
                  folders; the codex arm has excluded exactly that since before CC rows \
                  carried a real file_path",
}];

/// A source site that still spells a store path by hand, with the reason it is
/// allowed to. Every entry is a place a fourth agent CLI would have to be
/// remembered — the list should only ever shrink.
#[derive(Debug, Clone, Copy)]
pub struct RecordedStoreLiteral {
    /// The enclosing `fn` or `const`, exactly as in the source.
    pub owner: &'static str,
    pub recorded: &'static str,
    /// Why it cannot read the registry instead.
    pub reason: &'static str,
}

/// `(scanned, skipped)` line counts for [`unregistered_store_literals`].
///
/// Exists because the scanner failed SILENTLY once: a brace-counting skip was
/// fooled by the braces in embedded JS/CSS and swallowed two thirds of
/// `shell.rs`, so the lock passed while seeing almost none of the code it
/// guards. A lock that cannot go blind unnoticed needs this reported, and every
/// caller asserts it against a floor.
pub fn store_literal_scan_coverage(source: &str) -> (usize, usize) {
    let scanned = product_lines(source).len();
    (scanned, source.lines().count() - scanned)
}

/// The PRODUCT lines of `source` — everything outside a `#[cfg(test)] mod`
/// block — as `(zero-based line index, line)`.
///
/// One implementation, consulted by both the scanner and its coverage report:
/// two copies of a skip rule that can disagree is the exact failure this module
/// exists to prevent, and a coverage number derived from a *different* rule than
/// the scan would be worse than none.
///
/// ⚠ The block end is a column-0 `}`, deliberately NOT brace counting: this
/// workspace's UI code embeds JS and CSS in raw strings, whose braces are not
/// code. Counting them made the scanner swallow 90k lines of `shell.rs` — two
/// thirds of the file, including every product site the lock exists to watch —
/// and pass green while seeing almost nothing. A rustfmt'd top-level block
/// always closes on a column-0 `}`.
fn product_lines(source: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut in_test_module = false;
    let mut pending_test_attribute = false;
    for (index, line) in source.lines().enumerate() {
        if in_test_module {
            if line == "}" {
                in_test_module = false;
            }
            continue;
        }
        if line.starts_with("#[cfg(test)]") {
            pending_test_attribute = true;
            continue;
        }
        if pending_test_attribute {
            pending_test_attribute = false;
            if line.starts_with("mod ") || line.starts_with("pub mod ") {
                in_test_module = true;
                continue;
            }
        }
        out.push((index, line));
    }
    out
}

/// Scan `source` for hand-written store paths outside the registry.
///
/// The structural half of this phase, and the same discipline as the shell's
/// focus-site scan: enumerating these BY HAND is precisely how eleven copies
/// accumulated in the first place. Returns `(owner, line_number, literal)` for
/// every site not in `recorded`.
///
/// `#[cfg(test)] mod` blocks are skipped ([`product_lines`]) — fixture paths
/// are not a second source of truth, they are test data.
pub fn unregistered_store_literals(
    source: &str,
    recorded: &[RecordedStoreLiteral],
) -> Vec<(String, usize, String)> {
    // The store ROOT bare (`.codex/sessions`), not just the anchored fragment:
    // that also catches the embedded remote scripts, whose paths are python
    // strings (`'~/.claude/projects'`) with no trailing separator.
    let mut needles: Vec<String> = unique(
        AGENT_CLIS
            .iter()
            .flat_map(|descriptor| descriptor.store_roots())
            .map(str::to_string),
    );
    for descriptor in AGENT_CLIS {
        for name in descriptor.store_home_dir_names() {
            needles.push(format!("join(\"{name}\")"));
        }
    }

    let mut findings = Vec::new();
    let mut owner = String::from("<top level>");

    for (index, line) in product_lines(source) {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("fn ")
            .or_else(|| trimmed.strip_prefix("pub fn "))
            .or_else(|| trimmed.strip_prefix("pub(crate) fn "))
            .or_else(|| trimmed.strip_prefix("async fn "))
            .or_else(|| trimmed.strip_prefix("pub async fn "))
        {
            owner = rest
                .split(['(', '<', ' '])
                .next()
                .unwrap_or("<unknown>")
                .to_string();
        } else if let Some(rest) = trimmed
            .strip_prefix("const ")
            .or_else(|| trimmed.strip_prefix("pub const "))
            .or_else(|| trimmed.strip_prefix("pub(crate) const "))
        {
            owner = rest
                .split([':', ' '])
                .next()
                .unwrap_or("<unknown>")
                .to_string();
        }

        // A doc comment ABOUT the layout is documentation, not an encoding.
        if trimmed.starts_with("//") || trimmed.starts_with("#") {
            continue;
        }
        for needle in &needles {
            if line.contains(needle.as_str()) && !recorded.iter().any(|entry| entry.owner == owner)
            {
                findings.push((owner.clone(), index + 1, needle.clone()));
            }
        }
    }
    findings
}

/// Assert that `probe` — a predicate under test, answering "does this predicate
/// cover this store path?" — covers every registered CLI store, except the
/// holes recorded for `predicate_name` in [`KNOWN_STORE_PREDICATE_HOLES`].
///
/// Both directions, deliberately: an uncovered store that is NOT recorded is a
/// new hole, and a recorded hole that IS covered is a stale row to delete.
pub fn assert_store_predicate_coverage(predicate_name: &str, probe: impl Fn(&str) -> bool) {
    for descriptor in AGENT_CLIS {
        let example = descriptor.example_store_path("/home/example");
        let covered = probe(&example);
        let recorded = KNOWN_STORE_PREDICATE_HOLES
            .iter()
            .any(|hole| hole.predicate == predicate_name && hole.kind == descriptor.kind);
        assert!(
            covered || recorded,
            "{predicate_name} does not cover {:?}'s store ({example}). Either fix it, or \
             record the hole in KNOWN_STORE_PREDICATE_HOLES with its consequence.",
            descriptor.kind
        );
        assert!(
            !(covered && recorded),
            "{predicate_name} now COVERS {:?}'s store — delete its KNOWN_STORE_PREDICATE_HOLES \
             row in this same commit, or the table goes stale-green.",
            descriptor.kind
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The registry is the SSOT for "is this an agent CLI". If these disagree,
    // a CLI can be an agent to one predicate and not to another — the fork
    // class this whole spec exists to kill.
    #[test]
    fn every_agent_kind_has_a_descriptor_and_vice_versa() {
        for kind in [
            SessionKind::Codex,
            SessionKind::CodexLiteLlm,
            SessionKind::ClaudeCode,
            SessionKind::Shell,
            SessionKind::SshShell,
            SessionKind::Document,
        ] {
            assert_eq!(
                agent_cli_descriptor(kind).is_some(),
                kind.is_agent(),
                "{kind:?}: descriptor presence must equal is_agent()"
            );
        }
    }

    #[test]
    fn descriptors_are_unique_per_kind_and_name_a_binary() {
        let mut kinds: Vec<SessionKind> = AGENT_CLIS.iter().map(|d| d.kind).collect();
        let before = kinds.len();
        kinds.sort_by_key(|kind| format!("{kind:?}"));
        kinds.dedup();
        assert_eq!(before, kinds.len(), "one descriptor per kind");
        for descriptor in AGENT_CLIS {
            assert!(!descriptor.binary_name.is_empty());
            assert!(!descriptor.display_name.is_empty());
        }
    }

    // These token shapes ARE the shipped invocations. They are asserted here so
    // the launch builder can be rewritten to consult the descriptor without
    // anyone having to re-derive what codex vs claude expect.
    #[test]
    fn resume_tokens_match_each_cli_shipped_invocation() {
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert_eq!(
            codex.resume_tokens("'abc'", true),
            vec!["resume", "-C", "\"$PWD\"", "'abc'"]
        );
        // No cwd established ⇒ no re-root, even for a re-rooting CLI.
        assert_eq!(codex.resume_tokens("'abc'", false), vec!["resume", "'abc'"]);

        let claude = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert_eq!(
            claude.resume_tokens("'abc'", true),
            vec!["--resume", "'abc'"]
        );

        // Behavior-preserving: the LiteLLM fork never re-rooted pre-descriptor.
        let litellm = agent_cli_descriptor(SessionKind::CodexLiteLlm).unwrap();
        assert_eq!(
            litellm.resume_tokens("'abc'", true),
            vec!["resume", "'abc'"]
        );
    }

    // ─── store half (phase 1b) ────────────────────────────────────────────

    // Byte-for-byte, the same reason the invocation strings are locked: these
    // globs ARE the shipped store layout, and a refactor must not quietly move
    // where yggterm looks for a user's sessions.
    #[test]
    fn store_globs_and_roots_match_each_cli_shipped_layout() {
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert_eq!(
            codex.session_store_globs,
            [".codex/sessions/**/rollout-*.jsonl"]
        );
        assert_eq!(codex.store_roots(), [".codex/sessions"]);
        assert_eq!(codex.store_path_fragments(), ["/.codex/sessions/"]);

        let litellm = agent_cli_descriptor(SessionKind::CodexLiteLlm).unwrap();
        assert_eq!(
            litellm.session_store_globs,
            [".codex-litellm/sessions/**/rollout-*.jsonl"]
        );
        assert_eq!(
            litellm.store_path_fragments(),
            ["/.codex-litellm/sessions/"]
        );

        let claude = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert_eq!(claude.session_store_globs, [".claude/projects/*/*.jsonl"]);
        assert_eq!(claude.store_path_fragments(), ["/.claude/projects/"]);
    }

    // The classification the old `is_codex_session_file` did by hand, now
    // asserted through the registry — including the `.bak.` exclusion, which a
    // glob alone cannot express and which a naive port would have dropped.
    #[test]
    fn store_session_file_classification_matches_the_shipped_predicates() {
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert!(codex.store_path_is_session_file(
            "/home/user/.codex/sessions/2026/07/25/rollout-2026-07-25T01-00-00-abc.jsonl"
        ));
        // Depth is not fixed — `**` must accept a flat layout too.
        assert!(codex.store_path_is_session_file("/home/user/.codex/sessions/rollout-abc.jsonl"));
        assert!(!codex.store_path_is_session_file(
            "/home/user/.codex/sessions/2026/07/25/rollout-abc.bak.jsonl"
        ));
        // Under the root but not a session file.
        assert!(!codex.store_path_is_session_file("/home/user/.codex/sessions/2026/notes.txt"));
        assert!(!codex.store_path_is_session_file("/home/user/.codex/sessions/2026/history.jsonl"));
        // Wrong CLI's store.
        assert!(
            !codex.store_path_is_session_file(
                "/home/user/.codex-litellm/sessions/2026/rollout-abc.jsonl"
            )
        );

        let claude = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert!(claude.store_path_is_session_file(
            "/home/user/.claude/projects/-home-user-gh-yggterm/68c12af2-4784.jsonl"
        ));
        // CC's layout is EXACTLY one project level — a deeper file is not a
        // session (this is what `*` rather than `**` buys).
        assert!(!claude.store_path_is_session_file(
            "/home/user/.claude/projects/-home-pi/nested/68c12af2.jsonl"
        ));
        assert!(
            !claude.store_path_is_session_file("/home/user/.claude/projects/-home-pi/session.json")
        );
    }

    // CODEX_FAMILY is the one hand-list this module keeps, so it needs a lock
    // of its own: a third codex fork that nobody adds here would silently drop
    // out of every family predicate (fd rebinding, tree expansion, the shell's
    // storage-root lookup) while still scanning and launching fine — the exact
    // half-wired shape §7 catalogues.
    #[test]
    fn codex_family_holds_every_codex_build_and_only_those() {
        for kind in CODEX_FAMILY {
            assert!(
                agent_cli_descriptor(*kind).is_some(),
                "{kind:?} is in CODEX_FAMILY without a descriptor"
            );
        }
        for descriptor in AGENT_CLIS {
            let is_codex_build = descriptor.binary_name.starts_with("codex");
            assert_eq!(
                is_codex_build,
                CODEX_FAMILY.contains(&descriptor.kind),
                "{:?} invokes `{}` — a codex build belongs in CODEX_FAMILY (and \
                 nothing else does), or its family predicates skip it",
                descriptor.kind,
                descriptor.binary_name
            );
        }
    }

    // `/.codex/sessions/` must never be found inside `/.codex-litellm/sessions/`
    // (or any future pair), or one CLI's rows silently land under another.
    #[test]
    fn agent_cli_store_roots_are_mutually_exclusive() {
        for descriptor in AGENT_CLIS {
            for fragment in descriptor.store_path_fragments() {
                for other in AGENT_CLIS {
                    if other.kind == descriptor.kind {
                        continue;
                    }
                    for other_fragment in other.store_path_fragments() {
                        assert!(
                            !other_fragment.contains(&fragment),
                            "{:?}'s store fragment {fragment} is contained in {:?}'s \
                             {other_fragment} — a path would classify as both",
                            descriptor.kind,
                            other.kind
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn store_path_lookup_routes_each_shipped_layout_to_its_cli() {
        assert_eq!(
            agent_cli_for_store_session_file(
                "/home/user/.codex/sessions/2026/07/25/rollout-abc.jsonl"
            )
            .map(|d| d.kind),
            Some(SessionKind::Codex)
        );
        assert_eq!(
            agent_cli_for_store_session_file(
                "/home/user/.codex-litellm/sessions/2026/rollout-abc.jsonl"
            )
            .map(|d| d.kind),
            Some(SessionKind::CodexLiteLlm)
        );
        assert_eq!(
            agent_cli_for_store_session_file("/home/user/.claude/projects/-home-pi/abc.jsonl")
                .map(|d| d.kind),
            Some(SessionKind::ClaudeCode)
        );
        assert_eq!(
            agent_cli_for_store_session_file("/home/user/notes.jsonl").map(|d| d.kind),
            None
        );
        // Under a root but not a session file: located, not classified.
        assert_eq!(
            agent_cli_for_store_path("/home/user/.codex/sessions/2026/history.jsonl").map(|d| d.kind),
            Some(SessionKind::Codex)
        );
        assert_eq!(
            agent_cli_for_store_session_file("/home/user/.codex/sessions/2026/history.jsonl")
                .map(|d| d.kind),
            None
        );
    }

    // The shell's `codex_storage_root_for_path` returned this; it is now the
    // descriptor's job, and it must answer identically for both codex forks.
    #[test]
    fn store_home_prefix_strips_the_sessions_tail() {
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert_eq!(
            codex.store_home_prefix_of("/home/user/.codex/sessions/2026/rollout-a.jsonl"),
            Some("/home/user/.codex")
        );
        let litellm = agent_cli_descriptor(SessionKind::CodexLiteLlm).unwrap();
        assert_eq!(
            litellm.store_home_prefix_of("/home/user/.codex-litellm/sessions/2026/rollout-a.jsonl"),
            Some("/home/user/.codex-litellm")
        );
        assert_eq!(codex.store_home_prefix_of("/home/user/notes.jsonl"), None);
    }

    #[test]
    fn store_roots_absolute_default_to_home_without_an_override() {
        let home = Path::new("/home/user");
        let claude = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert_eq!(
            claude.store_roots_absolute(home),
            [PathBuf::from("/home/user/.claude/projects")]
        );
        let litellm = agent_cli_descriptor(SessionKind::CodexLiteLlm).unwrap();
        assert_eq!(
            litellm.store_roots_absolute(home),
            [PathBuf::from("/home/user/.codex-litellm/sessions")]
        );
    }

    #[test]
    fn every_agent_cli_declares_a_store() {
        for descriptor in AGENT_CLIS {
            assert!(
                !descriptor.session_store_globs.is_empty(),
                "{:?} must declare where it keeps its sessions",
                descriptor.kind
            );
            for glob in descriptor.session_store_globs {
                let root = literal_prefix(glob);
                assert!(
                    !root.is_empty() && !root.starts_with('/') && !root.contains('*'),
                    "{glob} must start with a literal, $HOME-relative store root"
                );
                assert!(
                    !glob_tail(glob).is_empty(),
                    "{glob} must name files below its root, not the root itself"
                );
            }
        }
    }

    // The scanner is itself load-bearing, so prove it FIRES — a source lock
    // that can only pass is worth nothing.
    #[test]
    fn store_literal_scanner_catches_re_encodings_and_spares_tests_and_prose() {
        let source = "\
fn is_a_codex_transcript(path: &str) -> bool {
    path.contains(\"/.codex/sessions/\")
}
fn cc_projects() -> PathBuf {
    home.join(\".claude\").join(\"projects\")
}
/// Doc prose naming \"/.claude/projects/\" is documentation, not an encoding.
fn documented() -> bool { true }
#[cfg(test)]
mod tests {
    fn fixture() -> &'static str {
        \"/home/user/.codex/sessions/2026/rollout-a.jsonl\"
    }
}
";
        let findings = unregistered_store_literals(source, &[]);
        let owners: Vec<&str> = findings
            .iter()
            .map(|(owner, _, _)| owner.as_str())
            .collect();
        assert!(
            owners.contains(&"is_a_codex_transcript"),
            "a re-encoded store fragment must be caught: {findings:?}"
        );
        assert!(
            owners.contains(&"cc_projects"),
            "a hand-built store path must be caught: {findings:?}"
        );
        assert!(
            !owners.contains(&"documented"),
            "prose about the layout is not an encoding: {findings:?}"
        );
        assert!(
            !owners.contains(&"fixture"),
            "test fixtures are data, not a second source of truth: {findings:?}"
        );

        // …and a recorded site is exempt, by owner name.
        let recorded = [RecordedStoreLiteral {
            owner: "is_a_codex_transcript",
            recorded: "2026-07-25",
            reason: "test of the exemption path",
        }];
        let remaining = unregistered_store_literals(source, &recorded);
        assert!(
            !remaining
                .iter()
                .any(|(owner, _, _)| owner == "is_a_codex_transcript"),
            "a recorded site must be exempt: {remaining:?}"
        );
    }

    #[test]
    fn glob_matcher_handles_the_metacharacters_it_claims() {
        assert!(glob_matches("**/rollout-*.jsonl", "rollout-a.jsonl"));
        assert!(glob_matches(
            "**/rollout-*.jsonl",
            "2026/07/25/rollout-a.jsonl"
        ));
        assert!(!glob_matches("**/rollout-*.jsonl", "2026/notes.jsonl"));
        assert!(glob_matches("*/*.jsonl", "proj/a.jsonl"));
        assert!(!glob_matches("*/*.jsonl", "a.jsonl"));
        assert!(!glob_matches("*/*.jsonl", "proj/nested/a.jsonl"));
        // A `*` must not cross a separator.
        assert!(!glob_matches("*.jsonl", "proj/a.jsonl"));
        // Multiple stars in one segment.
        assert!(segment_matches(
            "rollout-*-*.jsonl",
            "rollout-2026-abc.jsonl"
        ));
        assert!(!segment_matches("rollout-*-*.jsonl", "rollout-abc.jsonl"));
        // A literal pattern is an equality test.
        assert!(segment_matches("sessions", "sessions"));
        assert!(!segment_matches("sessions", "sessions2"));
    }

    #[test]
    fn resume_picker_tokens_carry_no_session_id() {
        assert_eq!(
            agent_cli_descriptor(SessionKind::Codex)
                .unwrap()
                .resume_picker_tokens(),
            vec!["resume"]
        );
        assert_eq!(
            agent_cli_descriptor(SessionKind::ClaudeCode)
                .unwrap()
                .resume_picker_tokens(),
            vec!["--resume"]
        );
    }
}
