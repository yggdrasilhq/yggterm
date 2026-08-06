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

/// Whether a CLI flag swallows the next token as its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagArity {
    /// `--dangerously-skip-permissions` — the flag IS the whole statement.
    Standalone,
    /// `--model opus` — the following token belongs to it.
    TakesValue,
}

/// Which per-launch option supersedes a configured flag.
///
/// Declared per flag rather than inferred from the emit table: `--sandbox` is a
/// permission flag codex accepts but yggterm never emits, and inferring its
/// category from "is it in the emit list" would put it in the wrong bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverriddenBy {
    Model,
    PermissionMode,
}

/// A permission posture a delegate launch can ask for, in ONE vocabulary
/// across every agent CLI.
///
/// **Why yggterm names these rather than passing the CLI's own spelling
/// through.** A caller launching a delegate row should not have to know that
/// Claude Code says `acceptEdits` while codex says `--sandbox workspace-write`,
/// any more than it has to know that one resumes with `--resume <id>` and the
/// other with `resume <id>`. Per-CLI spelling is DATA on the descriptor
/// ([`AgentCliDescriptor::permission_modes`]), exactly like [`ResumeSelector`].
///
/// ⛔ **A mode absent from a CLI's table is REFUSED BY NAME, never
/// approximated.** A permission mode is a security boundary; a mapping that
/// reads `accept-edits` but means "never ask, sandboxed to the workspace" is a
/// second encoding of that boundary which can silently diverge from what the
/// caller believed it asked for. Codex has no plan mode and no edits-only
/// approval, so it declares neither and says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentPermissionMode {
    /// Whatever the CLI itself defaults to — emits NO tokens.
    ///
    /// Deliberately empty rather than spelling the CLI's default value: Claude
    /// Code's `--permission-mode` choices have already been renamed once
    /// (`default` → `manual`/`auto`, seen on 2.1.223), and a value we do not
    /// need to send is a value that cannot rot.
    Default,
    Plan,
    AcceptEdits,
    /// Skip permission prompts entirely — what an unattended delegate needs.
    Bypass,
}

/// The spellings [`AgentPermissionMode::parse`] accepts, in the order a refusal
/// lists them.
pub const AGENT_PERMISSION_MODE_NAMES: &[(&str, AgentPermissionMode)] = &[
    ("default", AgentPermissionMode::Default),
    ("plan", AgentPermissionMode::Plan),
    ("accept-edits", AgentPermissionMode::AcceptEdits),
    ("bypass", AgentPermissionMode::Bypass),
];

impl AgentPermissionMode {
    /// Parse the flag value. One spelling per mode: an alias table is a second
    /// vocabulary to keep in sync with the docs, and there is no user demand
    /// for one.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        AGENT_PERMISSION_MODE_NAMES
            .iter()
            .find(|(name, _)| *name == trimmed)
            .map(|(_, mode)| *mode)
            .ok_or_else(|| {
                format!(
                    "--permission-mode {raw:?} is not a mode yggterm knows. Try one of: {}",
                    AGENT_PERMISSION_MODE_NAMES
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }

    /// The spelling a caller types, and the one a reply reports back.
    pub fn name(&self) -> &'static str {
        AGENT_PERMISSION_MODE_NAMES
            .iter()
            .find(|(_, mode)| mode == self)
            .map(|(name, _)| *name)
            .unwrap_or("default")
    }
}

/// What ONE launch asks of an agent CLI, over and above the user's configured
/// defaults: which model, and which permission posture.
///
/// **The trap this closes.** `terminal new --kind claude-code` used to be
/// unusable for a delegate row because model and permission mode came ONLY from
/// global settings: the row inherited whatever the user had set as their default
/// model — the exact tier a delegate exists to avoid — and bypass could be asked
/// for only by mutating `claude_code_extra_args`, a setting the user owns and an
/// agent has no business writing. Both are now per-launch, and neither reads or
/// writes the global setting.
///
/// Empty is the norm: a launch that asks for nothing composes to byte-identical
/// behaviour with the pre-flag path, which is what keeps every human door
/// (titlebar +, KeyTips, start page) unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AgentLaunchOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<AgentPermissionMode>,
}

impl AgentLaunchOptions {
    /// Nothing asked for ⇒ every path stays exactly as it was.
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.permission_mode.is_none()
    }

    /// The tokens this launch adds, or a refusal naming what went wrong.
    ///
    /// ⛔ **Refuses rather than ignores.** A silently dropped `--model` is
    /// precisely how the inheritance trap survived unnoticed for so long: the
    /// launch reported success and the row quietly ran on the wrong tier. So a
    /// non-agent kind, an empty value, or a mode this CLI cannot express is an
    /// ERROR the caller reads, never a no-op.
    pub fn launch_tokens(&self, kind: SessionKind) -> Result<Vec<String>, String> {
        let Some(descriptor) = agent_cli_descriptor(kind) else {
            if self.is_empty() {
                return Ok(Vec::new());
            }
            let asked = self.asked_flag_names().join(" and ");
            return Err(format!(
                "{asked} applies to an agent CLI session, and --kind {} has no CLI to pass it to. \
                 Launch with --kind claude-code or --kind codex.",
                session_kind_flag_name(kind)
            ));
        };
        let mut tokens = Vec::new();
        if let Some(model) = &self.model {
            if model.trim().is_empty() {
                return Err(
                    "--model needs a model id; an empty one would silently inherit the default"
                        .to_string(),
                );
            }
            tokens.push(descriptor.model_flag.to_string());
            tokens.push(model.trim().to_string());
        }
        if let Some(mode) = self.permission_mode {
            let allowed = descriptor
                .permission_modes
                .iter()
                .find(|(candidate, _)| *candidate == mode)
                .map(|(_, tokens)| *tokens)
                .ok_or_else(|| {
                    format!(
                        "{} has no {} mode. It offers: {}.",
                        descriptor.display_name,
                        mode.name(),
                        descriptor
                            .permission_modes
                            .iter()
                            .map(|(candidate, _)| candidate.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            tokens.extend(allowed.iter().map(|token| (*token).to_string()));
        }
        Ok(tokens)
    }

    /// The configured extra args with every flag this launch overrides removed,
    /// so "per-launch wins" is settled HERE rather than by whichever CLI happens
    /// to prefer its last occurrence.
    ///
    /// Only the flags actually asked for are stripped: a launch that pins a
    /// model leaves the user's configured permission flags alone.
    pub fn strip_overridden(&self, kind: SessionKind, configured: &[String]) -> Vec<String> {
        let Some(descriptor) = agent_cli_descriptor(kind) else {
            return configured.to_vec();
        };
        let strip: Vec<(&str, FlagArity)> = descriptor
            .overridden_flags
            .iter()
            .filter(|(_, _, overridden_by)| match overridden_by {
                OverriddenBy::Model => self.model.is_some(),
                OverriddenBy::PermissionMode => self.permission_mode.is_some(),
            })
            .map(|(flag, arity, _)| (*flag, *arity))
            .collect();
        if strip.is_empty() {
            return configured.to_vec();
        }
        let mut kept = Vec::new();
        let mut index = 0;
        while index < configured.len() {
            let token = configured[index].as_str();
            // `--model=opus` is one token; `--model opus` is two.
            let (head, inline_value) = match token.split_once('=') {
                Some((head, _)) => (head, true),
                None => (token, false),
            };
            match strip.iter().find(|(flag, _)| *flag == head) {
                Some((_, FlagArity::TakesValue)) if !inline_value => index += 2,
                Some(_) => index += 1,
                None => {
                    kept.push(configured[index].clone());
                    index += 1;
                }
            }
        }
        kept
    }

    /// The flag names this launch asked for, for a refusal message.
    fn asked_flag_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.model.is_some() {
            names.push("--model");
        }
        if self.permission_mode.is_some() {
            names.push("--permission-mode");
        }
        names
    }
}

/// Read `--model` / `--permission-mode` out of a command line.
///
/// ONE reader for both binaries, the same rule the tenancy flags already follow:
/// a flag must mean the same thing whether it was typed at `yggterm` or at
/// `yggterm-headless`, so neither carries a copy of this parser.
///
/// Value-shaped refusals happen here (present-but-empty, unknown mode); the
/// KIND-shaped refusal cannot, because the caller has not resolved the kind yet
/// — [`AgentLaunchOptions::launch_tokens`] owns that one.
pub fn agent_launch_options_from_args(args: &[String]) -> Result<AgentLaunchOptions, String> {
    let model = match flag_present(args, "--model") {
        false => None,
        // Deliberately NOT `cli_flag_value`, which treats a missing value and a
        // `--`-prefixed next token alike as "absent". A caller who typed
        // `--model` and meant it deserves an error, not a silent fallback to
        // the default tier — that silence IS the bug this feature closes.
        true => Some(
            crate::cli_flag_value(args, "--model")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "--model needs a model id (e.g. --model claude-opus-5); it was given none"
                        .to_string()
                })?
                .to_string(),
        ),
    };
    let permission_mode = match flag_present(args, "--permission-mode") {
        false => None,
        true => Some(AgentPermissionMode::parse(
            crate::cli_flag_value(args, "--permission-mode")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!(
                        "--permission-mode needs a mode: {}",
                        AGENT_PERMISSION_MODE_NAMES
                            .iter()
                            .map(|(name, _)| *name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?,
        )?),
    };
    Ok(AgentLaunchOptions {
        model,
        permission_mode,
    })
}

/// Whether `flag` appears at all, in either `--flag value` or `--flag=value`
/// spelling. The question `cli_flag_value` cannot answer, because it collapses
/// "absent" and "present with no usable value".
fn flag_present(args: &[String], flag: &str) -> bool {
    let inline_prefix = format!("{flag}=");
    args.iter()
        .any(|arg| arg == flag || arg.starts_with(&inline_prefix))
}

/// The `--kind` spelling for a session kind, for refusal messages.
fn session_kind_flag_name(kind: SessionKind) -> &'static str {
    match agent_cli_descriptor(kind) {
        Some(descriptor) => match descriptor.kind {
            SessionKind::ClaudeCode => "claude-code",
            SessionKind::CodexLiteLlm => "codex-litellm",
            _ => "codex",
        },
        None => match kind {
            SessionKind::SshShell => "ssh",
            SessionKind::Document => "document",
            _ => "shell",
        },
    }
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
    /// The flag this CLI takes a model id on, e.g. `--model`.
    ///
    /// Data, not a branch, for the same reason [`ResumeSelector`] is: the next
    /// CLI to arrive spells it in its own row instead of in an `is_claude`.
    pub model_flag: &'static str,
    /// The glyph this CLI draws at the head of its input composer — codex `›`,
    /// Claude Code `❯`.
    ///
    /// Data because "is this session sitting at a prompt I can type into?" was
    /// answered by a hardcoded `›`, which is codex's glyph alone. A Claude Code
    /// row therefore read as never-ready forever, and the readiness-gated
    /// prompt delivery silently refused to send to it (live, jojo 2026-08-06).
    pub composer_marker: char,
    /// Lowercase fragments of the chrome this CLI legitimately draws BELOW its
    /// composer — codex's model/shortcut hints, Claude Code's permission-mode
    /// footer.
    ///
    /// The readiness gate needs this to tell a CURRENT composer from an OLD
    /// prompt line with real output scrolled beneath it. Which lines count as
    /// chrome is per-CLI vocabulary, so it is per-CLI data; a shared hardcoded
    /// list is what made Claude Code's `⏵⏵ bypass permissions on` read as
    /// model output.
    pub composer_footer_hints: &'static [&'static str],
    /// Which permission postures this CLI can express, and the tokens for each.
    /// A mode absent from this table is refused by name — see
    /// [`AgentPermissionMode`].
    pub permission_modes: &'static [(AgentPermissionMode, &'static [&'static str])],
    /// Every flag that SETS a model or permission posture for this CLI —
    /// including spellings yggterm never emits (codex's `-m`, `--sandbox`).
    ///
    /// This is the strip list, not the emit list: when a per-launch option wins
    /// over the user's configured extra args, "wins" has to mean the configured
    /// flag is REMOVED, not that both are on the command line arguing about it
    /// and the CLI's own last-wins rule decides. Each entry says whether it
    /// consumes the following token as its value, and which per-launch option
    /// supersedes it.
    pub overridden_flags: &'static [(&'static str, FlagArity, OverriddenBy)],
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
        model_flag: "--model",
        composer_marker: '\u{203a}',
        composer_footer_hints: &["gpt-", "claude", "tab to ", "ctrl", "esc"],
        // Read off `codex --help` on codex-cli 0.144.6 (2026-08-06), not from
        // memory. Codex has NO plan mode and no edits-only approval — its
        // vocabulary is `--ask-for-approval {untrusted,on-request,never}` plus
        // `--sandbox {read-only,workspace-write,danger-full-access}`, which is a
        // different axis from Claude Code's. Approximating `accept-edits` as
        // "never ask, sandboxed to the workspace" would be yggterm inventing a
        // security posture the caller never asked for, so those two modes are
        // absent here and refused by name.
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (
                AgentPermissionMode::Bypass,
                &["--dangerously-bypass-approvals-and-sandbox"],
            ),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            ("-m", FlagArity::TakesValue, OverriddenBy::Model),
            (
                "--ask-for-approval",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            ("-a", FlagArity::TakesValue, OverriddenBy::PermissionMode),
            (
                "--sandbox",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            ("-s", FlagArity::TakesValue, OverriddenBy::PermissionMode),
            (
                "--dangerously-bypass-approvals-and-sandbox",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
        ],
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
        // Same binary family as codex, so the same flag vocabulary.
        model_flag: "--model",
        composer_marker: '\u{203a}',
        composer_footer_hints: &["gpt-", "claude", "tab to ", "ctrl", "esc"],
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (
                AgentPermissionMode::Bypass,
                &["--dangerously-bypass-approvals-and-sandbox"],
            ),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            ("-m", FlagArity::TakesValue, OverriddenBy::Model),
            (
                "--ask-for-approval",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            ("-a", FlagArity::TakesValue, OverriddenBy::PermissionMode),
            (
                "--sandbox",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            ("-s", FlagArity::TakesValue, OverriddenBy::PermissionMode),
            (
                "--dangerously-bypass-approvals-and-sandbox",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
        ],
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
        model_flag: "--model",
        composer_marker: '\u{276f}',
        composer_footer_hints: &["claude", "permissions", "shift+tab", "for agents", "ctrl", "esc"],
        // Read off `claude --help` on Claude Code 2.1.223 (2026-08-06). Its
        // `--permission-mode` choices are acceptEdits, auto, bypassPermissions,
        // manual, dontAsk, plan — note there is no longer a `default` value,
        // which is exactly why `Default` emits NOTHING instead of naming one.
        // Bypass goes through `--dangerously-skip-permissions` rather than
        // `--permission-mode bypassPermissions` because the standalone flag has
        // been stable across every CC version the fleet has run.
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (AgentPermissionMode::Plan, &["--permission-mode", "plan"]),
            (
                AgentPermissionMode::AcceptEdits,
                &["--permission-mode", "acceptEdits"],
            ),
            (
                AgentPermissionMode::Bypass,
                &["--dangerously-skip-permissions"],
            ),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            (
                "--permission-mode",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            (
                "--dangerously-skip-permissions",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
            (
                "--allow-dangerously-skip-permissions",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
        ],
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
///
/// Public because it is the workspace's ONE skip rule. The shell's reclaim
/// call-site lock scans `shell.rs` for its own production wiring and would
/// otherwise be satisfied by the test module that quotes every needle it looks
/// for; a second copy of this rule living over there is exactly the divergence
/// this module exists to prevent.
pub fn product_lines(source: &str) -> Vec<(usize, &str)> {
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

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|value| (*value).to_string()).collect()
    }

    // ---- per-launch model + permission mode (delegate launch) -------------

    #[test]
    fn a_model_becomes_the_cli_s_own_model_flag() {
        let options = AgentLaunchOptions {
            model: Some("claude-opus-5".to_string()),
            permission_mode: None,
        };
        assert_eq!(
            options.launch_tokens(SessionKind::ClaudeCode).unwrap(),
            vec!["--model".to_string(), "claude-opus-5".to_string()]
        );
        assert_eq!(
            options.launch_tokens(SessionKind::Codex).unwrap(),
            vec!["--model".to_string(), "claude-opus-5".to_string()]
        );
    }

    #[test]
    fn bypass_maps_to_each_cli_s_own_skip_flag() {
        let options = AgentLaunchOptions {
            model: None,
            permission_mode: Some(AgentPermissionMode::Bypass),
        };
        assert_eq!(
            options.launch_tokens(SessionKind::ClaudeCode).unwrap(),
            vec!["--dangerously-skip-permissions".to_string()]
        );
        assert_eq!(
            options.launch_tokens(SessionKind::Codex).unwrap(),
            vec!["--dangerously-bypass-approvals-and-sandbox".to_string()]
        );
    }

    // `default` means "do not send one", NOT "send the value named default" —
    // CC has already renamed that value out from under us once.
    #[test]
    fn the_default_mode_emits_nothing() {
        let options = AgentLaunchOptions {
            model: None,
            permission_mode: Some(AgentPermissionMode::Default),
        };
        assert!(
            options
                .launch_tokens(SessionKind::ClaudeCode)
                .unwrap()
                .is_empty()
        );
        assert!(options.launch_tokens(SessionKind::Codex).unwrap().is_empty());
    }

    // ⛔ THE REFUSALS. A silently ignored flag is how the model-inheritance
    // trap survived: the launch reported success while the row ran on the
    // user's default tier.
    #[test]
    fn a_model_on_a_shell_kind_is_refused_by_name() {
        let options = AgentLaunchOptions {
            model: Some("claude-opus-5".to_string()),
            permission_mode: None,
        };
        let error = options
            .launch_tokens(SessionKind::Shell)
            .expect_err("--model on a shell must refuse, never no-op");
        assert!(error.contains("--model"), "{error}");
        assert!(error.contains("--kind shell"), "{error}");
        assert!(error.contains("claude-code"), "{error}");
    }

    #[test]
    fn a_permission_mode_on_a_shell_kind_is_refused_by_name() {
        let options = AgentLaunchOptions {
            model: None,
            permission_mode: Some(AgentPermissionMode::Bypass),
        };
        let error = options
            .launch_tokens(SessionKind::Shell)
            .expect_err("--permission-mode on a shell must refuse");
        assert!(error.contains("--permission-mode"), "{error}");
    }

    #[test]
    fn an_empty_launch_on_a_shell_kind_is_fine() {
        assert!(
            AgentLaunchOptions::default()
                .launch_tokens(SessionKind::Shell)
                .unwrap()
                .is_empty(),
            "asking for nothing must stay byte-identical on every kind"
        );
    }

    #[test]
    fn an_empty_model_is_refused_rather_than_inherited() {
        let error = AgentLaunchOptions {
            model: Some("   ".to_string()),
            permission_mode: None,
        }
        .launch_tokens(SessionKind::ClaudeCode)
        .expect_err("an empty --model must refuse");
        assert!(error.contains("--model"), "{error}");

        let error = agent_launch_options_from_args(&args(&["--model"]))
            .expect_err("--model with no value must refuse");
        assert!(error.contains("--model"), "{error}");
        // The trap shape: `cli_flag_value` reads a `--`-prefixed next token as
        // "absent", which would have silently dropped the model.
        let error = agent_launch_options_from_args(&args(&["--model", "--no-activate"]))
            .expect_err("--model swallowed by the next flag must refuse");
        assert!(error.contains("--model"), "{error}");
    }

    #[test]
    fn an_unknown_permission_mode_is_refused_with_the_list() {
        let error = agent_launch_options_from_args(&args(&["--permission-mode", "yolo"]))
            .expect_err("an unknown mode must refuse");
        assert!(error.contains("yolo"), "{error}");
        for (name, _) in AGENT_PERMISSION_MODE_NAMES {
            assert!(error.contains(name), "refusal must list {name}: {error}");
        }
    }

    // Codex genuinely has neither mode (verified against codex-cli 0.144.6), so
    // it says so instead of approximating a security posture.
    #[test]
    fn a_mode_the_cli_cannot_express_is_refused_not_approximated() {
        for mode in [AgentPermissionMode::Plan, AgentPermissionMode::AcceptEdits] {
            let error = AgentLaunchOptions {
                model: None,
                permission_mode: Some(mode),
            }
            .launch_tokens(SessionKind::Codex)
            .expect_err("codex has no plan/accept-edits mode");
            assert!(error.contains(mode.name()), "{error}");
            assert!(error.contains("Codex"), "{error}");
            assert!(error.contains("bypass"), "the refusal must name what it DOES have: {error}");
        }
    }

    #[test]
    fn the_arg_reader_is_the_same_for_both_binaries() {
        let parsed = agent_launch_options_from_args(&args(&[
            "--kind",
            "claude-code",
            "--model",
            "claude-opus-5",
            "--permission-mode",
            "bypass",
        ]))
        .unwrap();
        assert_eq!(parsed.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(parsed.permission_mode, Some(AgentPermissionMode::Bypass));
        // Inline spelling reads the same.
        let inline = agent_launch_options_from_args(&args(&[
            "--model=claude-opus-5",
            "--permission-mode=bypass",
        ]))
        .unwrap();
        assert_eq!(inline, parsed);
        assert!(
            agent_launch_options_from_args(&args(&["--kind", "shell"]))
                .unwrap()
                .is_empty()
        );
    }

    // "Per-launch wins" has to mean the configured flag is REMOVED. Leaving
    // both on the command line makes the CLI's own precedence rule the source
    // of truth, which is a second encoding by definition.
    #[test]
    fn a_per_launch_option_strips_the_configured_flag_it_overrides() {
        let configured = args(&[
            "--model",
            "claude-fable-5",
            "--dangerously-skip-permissions",
            "--verbose",
        ]);
        let model_only = AgentLaunchOptions {
            model: Some("claude-opus-5".to_string()),
            permission_mode: None,
        };
        assert_eq!(
            model_only.strip_overridden(SessionKind::ClaudeCode, &configured),
            args(&["--dangerously-skip-permissions", "--verbose"]),
            "a pinned model strips the configured model and leaves the rest alone"
        );
        let mode_only = AgentLaunchOptions {
            model: None,
            permission_mode: Some(AgentPermissionMode::Plan),
        };
        assert_eq!(
            mode_only.strip_overridden(SessionKind::ClaudeCode, &configured),
            args(&["--model", "claude-fable-5", "--verbose"]),
            "a pinned mode strips the configured permission flags only"
        );
        assert_eq!(
            AgentLaunchOptions::default().strip_overridden(SessionKind::ClaudeCode, &configured),
            configured,
            "asking for nothing must not touch the user's configured args"
        );
    }

    #[test]
    fn stripping_handles_inline_values_and_short_flags() {
        let configured = args(&["--model=gpt-x", "-m", "gpt-y", "--sandbox", "read-only", "-C", "/tmp"]);
        let options = AgentLaunchOptions {
            model: Some("claude-opus-5".to_string()),
            permission_mode: Some(AgentPermissionMode::Bypass),
        };
        assert_eq!(
            options.strip_overridden(SessionKind::Codex, &configured),
            args(&["-C", "/tmp"]),
            "both spellings of the model flag and the sandbox flag must go"
        );
    }

    // Every flag we EMIT must also be one we STRIP, or a second launch could
    // stack two of them.
    #[test]
    fn every_emitted_flag_is_also_an_overridden_flag() {
        for descriptor in AGENT_CLIS {
            let strippable: Vec<&str> = descriptor
                .overridden_flags
                .iter()
                .map(|(flag, _, _)| *flag)
                .collect();
            assert!(
                strippable.contains(&descriptor.model_flag),
                "{}: model_flag {} is not in overridden_flags",
                descriptor.display_name,
                descriptor.model_flag
            );
            for (mode, tokens) in descriptor.permission_modes {
                if let Some(flag) = tokens.first() {
                    assert!(
                        strippable.contains(flag),
                        "{}: {} emits {flag}, which is not in overridden_flags",
                        descriptor.display_name,
                        mode.name()
                    );
                }
            }
        }
    }

    // Every CLI must be able to express "leave it alone", or a caller has no
    // way to ask for the user's own defaults explicitly.
    #[test]
    fn every_agent_cli_supports_the_default_mode() {
        for descriptor in AGENT_CLIS {
            let default = descriptor
                .permission_modes
                .iter()
                .find(|(mode, _)| *mode == AgentPermissionMode::Default)
                .unwrap_or_else(|| panic!("{} has no default mode", descriptor.display_name));
            assert!(
                default.1.is_empty(),
                "{}: the default mode must emit no tokens",
                descriptor.display_name
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
