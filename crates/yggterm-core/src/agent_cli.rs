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

/// Who decides a session's title.
///
/// Data, because the answer used to be `matches!(self, SessionKind::ClaudeCode)`
/// on [`SessionKind::self_generates_copy`] — a hand-list that a second
/// store-authoritative CLI silently falls out of, and then yggterm generates a
/// title for a CLI that already wrote its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleAuthority {
    /// The CLI writes its own title into its own store; yggterm READS it and
    /// only writes back on an explicit user rename (`spec-codex-cc-title-summary`).
    Store,
    /// The CLI records no title of its own; yggterm's LLM chore generates one.
    Generated,
}

/// How yggterm provisions this CLI on a machine it touches
/// (`spec-cli-binary-auto-provisioning`).
///
/// ⛔ User-local installs only — never `sudo`, never a `/usr/local` copy that
/// the CLI's own updater cannot write to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliInstall {
    /// `npm i -g <package>` under the yggterm-owned npm prefix.
    Npm(&'static str),
    /// `uv tool install <package>` — a Python CLI.
    Uv(&'static str),
    /// A vendor installer that writes into `~/.local/bin`. The str is the URL
    /// yggterm fetches and runs.
    ///
    /// ⚠ **The clause that stood here until 2026-08-08 — "yggterm records it so
    /// the provisioner can name what is missing, and does NOT run it
    /// unattended" — is SUPERSEDED by an owner ruling** (`docs/settled-calls.md`):
    /// *"yggterm should auto install, update ALL clis in all connected systems
    /// including localhost."* It is rewritten here rather than left standing
    /// because a reader who finds the old refusal in the type's own
    /// documentation re-derives the refusal, and the ruling loses.
    ///
    /// ⛔ The user-local constraint is NOT relaxed with it: the script is run
    /// with `HOME` intact and no privilege escalation, and a vendor installer
    /// that wants `sudo` or `/usr/local` is a bug report, not an install.
    VendorScript(&'static str),
    /// yggterm cannot FETCH this one — closed-source, licence-gated, or served
    /// only behind a sign-in the daemon does not hold. It detects it and refuses
    /// cleanly.
    ///
    /// ⚠ `Manual` is about ARRIVAL ONLY. It says nothing about whether the CLI
    /// stays current — see [`CliUpdate`], which is a separate axis precisely
    /// because the one `Manual` CLI in the registry updates itself.
    Manual,
}

impl CliInstall {
    /// Whether yggterm fetches this CLI itself, with no human in the loop.
    ///
    /// The ONE predicate the provisioner gates on. It used to be spelled
    /// "does this have an npm package", which answered `false` for a uv or
    /// vendor CLI that yggterm is perfectly able to install — the conflation
    /// the owner's 2026-08-08 ruling struck down.
    pub fn provisions_unattended(self) -> bool {
        match self {
            Self::Npm(_) | Self::Uv(_) | Self::VendorScript(_) => true,
            Self::Manual => false,
        }
    }
}

/// How a CLI that is ALREADY on the machine is kept current.
///
/// A second axis from [`CliInstall`], because arrival and staying-current are
/// answered by different things. Antigravity cannot be fetched by yggterm at
/// all ([`CliInstall::Manual`]) and yet updates itself perfectly (`agy update`,
/// read off its own `--help` on guihost, 2026-08-08); codex arrives from npm and
/// is updated by re-running that same install. Collapsing the two axes would
/// have made "yggterm keeps every CLI current" false for the one CLI that needs
/// no help doing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliUpdate {
    /// Re-run the install method — `npm i -g <pkg>@latest`, `uv tool upgrade
    /// <pkg>`, or the vendor script again (vendor installers upgrade in place).
    Reinstall,
    /// The CLI ships its own updater, and THAT is what runs — it is the only
    /// thing that knows where its own payload lives. `agy update` replaces a
    /// 166 MB self-contained binary no package manager on the machine has ever
    /// seen; an install-method refresh could not touch it.
    SelfCommand(&'static [&'static str]),
}

/// How many populated rows from the bottom the FOOTER classifiers read.
///
/// Footer chrome (`esc to interrupt`, a limit-wait line, a picker's key hints)
/// is by construction the last thing on the screen, so a shallow window is both
/// sufficient and a guard against matching the same words in conversation text
/// scrolled above.
const SCREEN_FOOTER_WINDOW_ROWS: usize = 10;

/// How many populated rows from the bottom the MODAL classifiers read.
///
/// A startup gate is drawn in the middle of an otherwise empty screen, so its
/// text sits further from the bottom than any footer does.
const SCREEN_MODAL_WINDOW_ROWS: usize = 24;

/// Does any of the last `window` POPULATED rows carry one of `phrases`?
///
/// ⛔ **POPULATED, not "the last `window` rows".** The four callers this
/// replaced each spelled `sample.lines().rev().take(10)`, which takes ten LINES
/// and then discards the blank ones — so on any screen whose bottom rows are
/// empty the window contains nothing but blanks and the classifier returns
/// false for a screen that is entirely the thing it was looking for. Their own
/// docstrings all said "the last ten non-empty lines", which is this; the code
/// never did it. It went unnoticed for as long as it did because the only input
/// anyone passed was the RAW screen, where a whole modal arrives as two long
/// lines and there are no trailing blanks to eat the window — the two defects
/// concealed each other, and feeding a correctly rendered grid to the old
/// window would have silently blinded every classifier at once.
fn screen_phrases_match(
    sample: &str,
    phrases: &'static [ScreenWorkingPhrase],
    negations: &'static [&'static str],
    window: usize,
) -> bool {
    sample
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(window)
        .any(|line| {
            let lower = line.to_ascii_lowercase();
            if negations.iter().any(|deny| lower.contains(deny)) {
                return false;
            }
            phrases.iter().any(|phrase| {
                lower.contains(phrase.needle)
                    && (phrase.also_any.is_empty()
                        || phrase.also_any.iter().any(|also| lower.contains(also)))
            })
        })
}

/// One phrase that means "a turn is in flight", as read off this CLI's SCREEN.
///
/// Distinct from [`AgentCliDescriptor::working_footer_hints`], which is scanned
/// only BELOW the composer for the agent-row plane. This one is the whole-screen
/// matcher behind the sidebar dot and the hot-update idle gate — the two
/// consumers that must never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenWorkingPhrase {
    /// Lowercase fragment the line must contain.
    pub needle: &'static str,
    /// When non-empty, the line must ALSO contain one of these. Codex's
    /// `working (` is only a work signal beside `/stop to close` or
    /// `background terminal running`; alone it is prose.
    pub also_any: &'static [&'static str],
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

/// Where a CLI's permission-preset flags came from.
///
/// Provenance is part of the UI, not a footnote
/// (`docs/spec-agent-cli-extra-args-modal.md` §5): a row whose flags were read
/// off a running binary and a row taken from a vendor's own reference must not
/// look the same, and a row nobody has measured must not look like either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionProvenance {
    /// Read off a running binary — `--help`, or a controlled probe of the
    /// parser when `--help` under-reports (qwen hides `--yolo` from its own).
    Measured,
    /// From the vendor's own reference, because the binary is installed on no
    /// host that has been asked. Renders with a "documented, not verified here"
    /// marker.
    Documented,
    /// Neither. The row renders DISABLED with the reason, rather than offering
    /// a guess — the same discipline the descriptor table already applies to
    /// `working_screen_phrases: &[]`.
    Unmeasured(&'static str),
}

/// One permission posture offered for a CLI, in that CLI's own vocabulary.
///
/// ⛔ **They are not five spellings of one idea and must not be flattened into
/// one.** Two CLIs express this as a flag, one as a config file the flag only
/// raises the floor of, one hides the flag from its own `--help`, and one has no
/// permission gate at all. A ladder that pretends otherwise hands the user a
/// flag their CLI ignores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionPreset {
    /// Stable key for this tier, stored nowhere and used to address the tier
    /// from a verb or a test. Unique within one CLI's list.
    pub id: &'static str,
    /// Button text — `Ask each time` · `Sandboxed` · `Skip checks`.
    pub label: &'static str,
    /// The exact flags, or empty for "the CLI's own default, no flags".
    pub args: &'static str,
    /// One sentence, in the CLI's own vocabulary, shown under the box.
    pub explanation: &'static str,
    /// Whether this tier pre-populates a box the user has never set.
    ///
    /// ⚖ A BOOL on the tier rather than a `permission_default: &str` naming one:
    /// an id-by-reference can name a tier that does not exist, and this file has
    /// already paid for one dangling cross-reference class. Exactly one tier per
    /// CLI carries it, and `every_cli_with_presets_has_exactly_one_default`
    /// fails the build otherwise.
    pub is_default: bool,
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
    /// The metadata-rail row that names this CLI's session id — always
    /// `"<display_name> Session"`, locked by
    /// [`the_session_metadata_label_is_the_display_name_plus_session`].
    ///
    /// ⚠ **Transcribed rather than computed, and only because it must be
    /// `&'static str`:** `SessionMetadataEntry::label` is a static string by
    /// design (184 callers, and the rail's readers match on it), so a
    /// `format!` cannot produce one. The lock is what makes the transcription
    /// safe — and it earns its place, because `"Codex Session"` and
    /// `"Claude Code Session"` are not decoration: predicates READ them to
    /// recover a row's session id.
    pub session_metadata_label: &'static str,
    /// The ONE lowercase wire name for this CLI: the `--kind` flag value, the
    /// `session_kind_label` string, and the `icon_kind` the row JSON reports.
    ///
    /// ⚠ It is deliberately NOT the same field as [`Self::wrapper_slug`] or the
    /// scheme strings: the three shipped CLIs carry historical spellings
    /// (`remote-session://` for codex, `resume-cc` for Claude Code) that are on
    /// disk and on the wire and may not be renamed. New CLIs derive everything
    /// from this one slug, which is what makes them free of that debt.
    pub slug: &'static str,
    /// The executable, as invoked on the session's host.
    pub binary_name: &'static str,
    /// How yggterm provisions the binary when a machine lacks it.
    pub install: CliInstall,
    /// How yggterm keeps the binary current once the machine has it.
    pub update: CliUpdate,
    /// The boxed-glyph mark drawn in the sidebar, start page and row JSON —
    /// codex `>_`, Claude Code `*_`, a shell `$_`.
    ///
    /// Data because "which icon?" had THREE answers in the shell (a kind string,
    /// a glyph, and a bespoke component), kept in agreement only by two tests.
    /// The mark is the CLI's identity, so the CLI declares it once.
    ///
    /// Two characters render pixel-identical to the shipped three; three still
    /// fit the rect but are drawn a point smaller (see `TreeIcon`).
    pub icon_glyph: &'static str,
    /// The CLI's brand colour, as the background of a solid control carrying
    /// WHITE text (the start page's "Open this … Session" button is the
    /// canonical instance). `DESIGN.md` § *Agent CLI brand colours* is the
    /// prose SSOT; this field is the one the code reads.
    ///
    /// **Data, because it was a two-arm `match` that answered `accent` for
    /// seven of the nine registered CLIs** — so a Qwen row and an OpenCode row
    /// painted identically and the colour carried no information at all. A
    /// tenth CLI is now a line in this table, not a new branch.
    ///
    /// ⛔ **Every value clears WCAG AA (≥4.5:1) against white**, which is the
    /// constraint that picked the exact shades — the amber this replaced sat at
    /// 3.19:1 and failed AA for normal text at the size the button actually
    /// renders. "Nearest available brand colour" is explicitly acceptable; an
    /// inaccessible one is not. Check a new entry with
    /// `the_brand_colours_clear_wcag_aa_against_white` before adding it.
    pub brand_color: &'static str,
    /// The letter this CLI's "New … Session Here" menu entry WANTS. A
    /// preference, never a guarantee — the KeyTip ladder may deny it.
    pub menu_hint: char,
    /// Who owns this CLI's session title.
    pub title_authority: TitleAuthority,
    /// Whether the CLI accepts a caller-supplied session id at birth (Claude
    /// Code's `--session-id <uuid>`).
    ///
    /// `true` ⇒ the row id IS the transcript id from birth and the remote
    /// identity poll is unnecessary BY DESIGN. `false` ⇒ the CLI mints its own
    /// id and yggterm rebinds `local://<synth>` once it appears (spec §7.5).
    /// This one bit is what decides whether the poll runs — it used to be a
    /// per-CLI code fork.
    pub id_assigned_at_birth: bool,
    /// The token the remote wrapper subcommands are built from:
    /// `resume-<slug>`, `start-<slug>`, `terminate-<slug>`,
    /// `<slug>-session-exists`.
    ///
    /// `None` ⇒ this CLI is LOCAL-ONLY and has no remote arm at all
    /// (`CodexLiteLlm`). Declaring it is what lets the arm matrices stop
    /// hardcoding `matches!(kind, CodexLiteLlm)`.
    pub wrapper_slug: Option<&'static str>,
    /// The scheme this CLI's REMOTE rows are identified by, including its `://`.
    /// `None` for a local-only CLI. Codex's is `remote-session://` for
    /// historical reasons; every new CLI's is `remote-<slug>://`.
    pub remote_row_scheme: Option<&'static str>,
    /// The scheme this CLI's daemon-owned runtime keys use, including its `://`.
    /// `None` for a local-only CLI. New CLIs get `<slug>-runtime://`.
    pub runtime_key_scheme: Option<&'static str>,
    /// Whole-SCREEN phrases meaning a turn is in flight — the sidebar dot and
    /// the hot-update idle gate read these. See [`ScreenWorkingPhrase`].
    ///
    /// ⛔ EMPTY means UNMEASURED, exactly as for `working_footer_hints`: the row
    /// then reports `false` for working because nothing was observed, and that
    /// gap belongs in the descriptor where the next session can see it — not in
    /// a matcher in another crate that nobody thinks to update.
    pub working_screen_phrases: &'static [ScreenWorkingPhrase],
    /// Lines that LOOK like a work signal and are not — codex's completion
    /// summary `Worked for 12s` contains `worked for `, and matched the naive
    /// `working` needle.
    pub working_screen_negations: &'static [&'static str],
    /// Whole-SCREEN phrases meaning the CLI is WAITING ON A USAGE LIMIT — a
    /// third state that is neither working nor idle. A limit-wait screen says
    /// none of the working phrases, so without these it reads as "confirmed
    /// idle" on every daemon-owned surface at once (the metadata Status, the
    /// sidebar dot, `gate-screen`, and the working→done notification edge —
    /// which then fires a false "done" at the exact moment nothing finished).
    ///
    /// ⛔ EMPTY means UNMEASURED, same law as `working_screen_phrases`: the
    /// state then folds into idle for that CLI because nothing was observed,
    /// and the gap belongs here where the next session can see it.
    pub limit_wait_screen_phrases: &'static [ScreenWorkingPhrase],
    /// Whole-SCREEN phrases meaning the CLI has an OWNER-FACING QUESTION PICKER
    /// up — a fourth state that is neither working nor idle nor limit-waiting.
    ///
    /// ⛔ THIS ONE EATS TYPED INPUT, which is what makes it different from the
    /// others. A picker consumes navigation keys only, so a sentence typed at
    /// it produces nothing visible anywhere: the owner experiences total input
    /// block on precisely the row that is asking for him, while the write
    /// transport is perfectly healthy. Worse, the CLI is mid-turn while it
    /// asks, so `working` reads TRUE and every surface says the row is busy
    /// working — a 27-minute wait was misdescribed that way (queue entry: an
    /// owner-facing question picker reads as "working" and eats typed input).
    ///
    /// ⛔ EMPTY means UNMEASURED, the same law as `working_screen_phrases`.
    pub question_picker_screen_phrases: &'static [ScreenWorkingPhrase],
    /// Whole-SCREEN phrases meaning the CLI is running a BACKGROUND AGENT and
    /// is advertising it in its own chrome.
    ///
    /// ⛔ THIS ONE EXISTS TO PREVENT A FALSE POSITIVE, NOT TO REPORT A STATE.
    /// A CLI running a background agent draws a dim line in its EMPTY composer
    /// describing the task, and on the screen plane that is indistinguishable
    /// from a half-typed message nobody sent. Three parties misread it at once:
    /// the owner read it as a robot having typed into his row without sending,
    /// two `\r` probes agreed with him (a lone Enter correctly no-ops on an
    /// empty buffer, so it looked like a stuck draft), and only a printable
    /// character settled it — typing `x` REPLACED the line instead of appending
    /// to it, and the text appeared nowhere in the transcript.
    ///
    /// ⇒ A reader that finds this signature must treat the composer as EMPTY.
    /// Combine with [`Self::screen_shows_working`]: hint present and working
    /// false is IDLE-WITH-A-BACKGROUND-AGENT, which is a healthy row.
    ///
    /// ⛔ EMPTY means UNMEASURED, the same law as `working_screen_phrases`.
    pub background_agent_hint_screen_phrases: &'static [ScreenWorkingPhrase],
    /// Whole-SCREEN phrases meaning the CLI is holding a STARTUP GATE — a
    /// first-run modal that stands BEFORE the composer exists, so the row is
    /// not yet reading input at all.
    ///
    /// ⛔ THIS ONE IS INVISIBLE TO EVERY OTHER INSTRUMENT, which is what makes
    /// it different. Measured 2026-08-21 on a row spawned into a directory this
    /// CLI had not seen: `working`, `question_picker`, `limit_wait` and
    /// `background_agent_hint` all read FALSE, `input-check` answered
    /// `consuming_input:false, wedged:false`, and the process was alive and
    /// ageing — the exact signature of a slow cold start, which is fixed by
    /// waiting, while this one never clears without a keypress. A spawner that
    /// cannot tell the two apart waits forever on a row that will never move.
    ///
    /// ⛔ A GATE IS NOT A PERMISSION PROMPT. A skip-permissions flag does not
    /// skip it, because workspace trust is a different question from tool
    /// permission; it fires per (CLI, directory) and per host, so a brand-new
    /// worktree stops a row while every sibling checkout is fine.
    ///
    /// ⚠ These phrases are BODY text, not footer chrome — which is why this
    /// classifier, unlike the four above, cannot be matched against the raw
    /// screen. A gate is drawn with absolute cursor positioning, so its nine
    /// visible rows arrive as two newline-delimited lines with the words of
    /// adjacent rows fused; see [`AgentCliDescriptor::screen_shows_startup_gate`].
    ///
    /// ⛔ EMPTY means UNMEASURED, the same law as `working_screen_phrases`.
    pub startup_gate_screen_phrases: &'static [ScreenWorkingPhrase],
    /// Whole-SCREEN phrases meaning the CLI is holding a LIMIT/BILLING DIALOG
    /// whose options are NOT equivalent.
    ///
    /// ⛔⛔ THE ONE STATE WHERE A WATCHDOG'S ENTER SPENDS MONEY. When the plan's
    /// limit is reached the CLI parks on a numbered choice — wait, switch
    /// account, or pay per token — and a lone carriage return does not "dismiss"
    /// it: it SELECTS whatever is highlighted. A watchdog that presses Enter at
    /// every prompt it meets will eventually change billing on the owner's
    /// account, and nobody will have decided it.
    ///
    /// ⚠ These phrases OVERLAP `limit_wait_screen_phrases` by design — both
    /// describe a row that has run out of quota. The difference is structural,
    /// not lexical: a dialog carries a selection marker on a numbered option and
    /// a footer does not, which is why the classifier pairs these phrases with
    /// [`crate::screen_state::screen_has_selected_numbered_option`] rather than
    /// trying to separate them by wording.
    ///
    /// ⛔ EMPTY means UNMEASURED, the same law as `working_screen_phrases`.
    pub plan_limit_choice_screen_phrases: &'static [ScreenWorkingPhrase],
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
    /// prompt delivery silently refused to send to it (live, guihost 2026-08-06).
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
    /// Lowercase fragments this CLI draws in its composer footer **only while a
    /// turn is in flight** — Claude Code's `esc to interrupt`.
    ///
    /// ⚖ This is the ONLY honest answer to "is this row working?" that the row
    /// plane can give, and it is per-CLI vocabulary for the same reason
    /// `composer_marker` is: a hardcoded phrase would silently answer `idle`
    /// for every CLI that words it differently, and `idle` is the answer a
    /// caller reads as *"finished, safe to move on"*.
    ///
    /// ⛔ **An EMPTY list means UNMEASURED, and the activity verdict is then
    /// `Unknown` — never `Idle`.** A CLI whose working phrase nobody has
    /// observed must not be reported as quiet; that is the guess that would
    /// turn "paused" into "grinding" (or worse, "done") for the next caller.
    pub working_footer_hints: &'static [&'static str],
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
    /// The stored-settings key this CLI's launch flags live under.
    ///
    /// The slug for every CLI but one. `codex-anything` (`codex-litellm` as an
    /// identifier) is a codex session's flip switch rather than a CLI of its
    /// own — settled 2026-08-08 — so it reads and writes CODEX's box: one CLI,
    /// one box, for both backends. Declaring that here is what stops it being
    /// re-derived as a `matches!(kind, Codex | CodexLiteLlm)` in every place
    /// that resolves a stored value.
    pub extra_args_slug: &'static str,
    /// The permission tiers this CLI offers, SAFEST FIRST, for the launch-flags
    /// modal (`docs/spec-agent-cli-extra-args-modal.md`).
    ///
    /// ⛔ EMPTY means the tiers are unknown, and it is only legitimate beside a
    /// [`PermissionProvenance::Unmeasured`] — the modal then renders the row
    /// disabled with the reason. A non-empty list beside `Unmeasured` is a guess
    /// wearing a measurement's clothes, and the registry lock refuses it.
    ///
    /// ⚠ These are the SAME postures [`Self::permission_modes`] maps for
    /// `--permission-mode`, in a second vocabulary aimed at a human rather than
    /// a delegate — but they are NOT a second encoding of the boundary, because
    /// `every_permission_preset_tier_agrees_with_the_launch_mode_it_names`
    /// checks the bypass tier against the tokens the launch verb would emit.
    pub permission_presets: &'static [PermissionPreset],
    /// Where [`Self::permission_presets`] came from — rendered, not footnoted.
    pub permission_provenance: PermissionProvenance,
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
    /// Files that ARE the store itself — one sqlite db for every session —
    /// home-relative (`.local/share/opencode/opencode.db`). A cli whose
    /// durable store is ONE file cannot express row identity as a path, so
    /// its rows must be keyed by the store SCHEME + session id; a leaf whose
    /// path is one of these files is a stale persisted shape, never a
    /// session. Empty for file-per-session CLIs.
    pub durable_store_files: &'static [&'static str],
    /// WHY this CLI's past sessions are not listed in the cwd tree, when
    /// [`Self::session_store_globs`] is empty.
    ///
    /// `None` ⇒ the store IS scanned. `Some(reason)` ⇒ the gap is DECLARED, and
    /// the reason names the specific obstacle so the next session can close it
    /// instead of rediscovering it. A CLI may be first-class for launch and
    /// resume — which is the product's core promise — while its historical
    /// sessions are not yet enumerable; what is forbidden is being silent about
    /// which of the two is true.
    pub store_scan_gap: Option<&'static str>,
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
    /// Answer "does this CLI's store hold `session_id`?" by a KEYED lookup,
    /// given the home directory to resolve the store under.
    ///
    /// `Some(reader)` ⇒ this CLI keeps an index that can be asked about one id
    /// directly (Muse's `session-index.db`, Antigravity's
    /// `conversation_summaries.db`). The reader itself returns `Option<bool>`:
    /// `None` when the index could not be consulted at all, which must stay
    /// distinct from `Some(false)`.
    ///
    /// `None` ⇒ this CLI has no index, and membership could only be settled by
    /// WALKING the store and parsing every file for the id buried inside it
    /// (codex). ⛔ That is deliberately not offered here: this hook is called on
    /// the resume path, and a full store walk there would put a multi-megabyte
    /// read in front of every launch — the cost that already had to be
    /// engineered out of the Claude Code identity refresh.
    ///
    /// ⚖ So this field says "can membership be settled CHEAPLY and
    /// authoritatively", and only a CLI that answers yes may have a resume
    /// re-routed on its say-so.
    pub store_membership_index: Option<fn(&Path, &str) -> Option<bool>>,
    /// How a RUNNING session of this CLI can be recognised from the files its
    /// process holds open — the route that binds a live row to the id the CLI
    /// actually minted, for a CLI that mints its own
    /// (`id_assigned_at_birth:false`).
    ///
    /// ⚠ MEASURED per CLI, never guessed, because the obvious file is the wrong
    /// one in both observed cases (2026-08-20):
    /// * a live Muse process holds `cron.db` open for SEVERAL session
    ///   directories at once and `.session.lock` only for the one it is running;
    /// * a live Antigravity process holds the SHARED conversation index open
    ///   from launch, and names its own conversation only once a turn has
    ///   happened, through `presence/<id>.lock`.
    /// Picking "some open file under the store" would be a coin flip in the
    /// first case and useless in the second.
    ///
    /// `None` ⇒ this CLI is not identified this way and the route is not tried.
    pub live_session_marker: Option<LiveSessionMarker>,
    /// This CLI's OWN title for one LIVE session, keyed by session id, read
    /// from the CLI's own store — the pickup a [`TitleAuthority::Store`] CLI
    /// depends on entirely, because yggterm refuses to generate copy for one.
    ///
    /// ⛔ **Distinct from [`Self::read_store_entry`], which is keyed by PATH.**
    /// The scanner already holds a store file and asks what is in it; a live row
    /// holds only the id its CLI minted and has to find the file — and for one
    /// of the two wired CLIs the title is not in a session file at all but in a
    /// shared index beside the store.
    ///
    /// ⛔ **The `&Path` is the AGENT STORE home (`~`), never the yggterm home.**
    /// The per-CLI arm this replaced passed `resolve_yggterm_home()`, so every
    /// lookup ran against `~/.yggterm/.gemini/...` — a directory that does not
    /// exist — and reported `no_title_in_store` for the life of the daemon.
    /// Measured on the GUI host 2026-08-21: 96 misses in 91 minutes, all for one
    /// row, whose title the store had held the whole time.
    ///
    /// `None` ⇒ UNMEASURED, the same law as [`Self::working_screen_phrases`]:
    /// this CLI's store layout has not been read off a real machine, so no
    /// lookup is attempted rather than a plausible path being guessed at.
    pub read_live_store_title: Option<fn(&Path, &str) -> Option<String>>,
    /// The ssh half of [`Self::read_live_store_title`]: how a REMOTE row of
    /// this CLI is asked for its own title.
    ///
    /// ⛔⛔ **THE FIELD THAT CLOSES A GAP BETWEEN TWO CHORES.** yggterm titles
    /// live rows from two places — a local chore that reads this machine's
    /// stores off disk, and a remote chore that reads another machine's over
    /// ssh. The local one skipped every `remote-*://` row saying *"that rides
    /// the other chore"*, and the remote one was written for ONE CLI and
    /// refused everything else by kind. A remote row of any CLI but Claude Code
    /// was therefore titled by nothing and kept its birth name for the life of
    /// the session, with no event anywhere saying so.
    ///
    /// ⚠ **It cannot be [`Self::read_live_store_title`] with a different home.**
    /// That reader opens local files (and, for one CLI, a local sqlite index);
    /// there is no such thing as passing it a path on another machine. The
    /// remote arm is necessarily a program that runs THERE, which is what this
    /// field carries.
    ///
    /// `None` ⇒ UNMEASURED, the same law as [`Self::working_screen_phrases`]:
    /// this CLI's store has not been read off a real remote machine, so no
    /// round trip is attempted. The chore reports `skipped_no_reader` for such
    /// a row rather than staying silent about it
    /// ([`crate::cli_plane::CliTitleOutcome`]).
    pub remote_live_store_title: Option<RemoteStoreTitleProbe>,
}

/// How a remote machine is asked for one of its own CLI session titles.
///
/// ⚖ **The script is pure IO; the SEMANTICS stay in Rust.** A probe returns the
/// raw strings the store holds, in the CLI's own precedence order, and
/// [`Self::choose`] decides which of them is a title — using the same
/// predicates the local reader uses. Deciding it in Python would put a second
/// encoding of "what counts as a title" on the far side of an ssh hop, where it
/// could drift from the local reader for a whole release without anything
/// disagreeing out loud.
// ⛔ No `PartialEq`: `choose` is a function pointer and comparing those is
// meaningless (addresses are not unique across codegen units), so a derived
// equality would silently answer wrong. Probes are compared by the CLI that
// owns them, never by value.
#[derive(Debug, Clone, Copy)]
pub struct RemoteStoreTitleProbe {
    /// Python 3 source, fed to `python3 -` on the session's host.
    ///
    /// **argv contract:** the locators first, then a literal `--`, then the
    /// session ids. ⛔ The separator is load-bearing — the predecessor script
    /// took `argv[1]` as the single locator and `argv[2:]` as ids, so a CLI
    /// with two store globs would have had its second glob read as a session
    /// id. It survived only because the one wired CLI declares exactly one.
    ///
    /// **Output contract:** one JSON object per line,
    /// `{"session_id": "…", "candidates": ["…", …]}`, candidates in the CLI's
    /// own precedence order. A session it cannot answer for is simply absent.
    pub script: &'static str,
    /// Which `$HOME`-relative paths the script is handed.
    pub locators: RemoteStoreLocators,
    /// Which of the raw candidates is this CLI's title.
    pub choose: fn(&[String]) -> Option<String>,
}

/// Where a [`RemoteStoreTitleProbe`] looks, expressed so that the answer stays
/// owned by the registry rather than transcribed into a script's argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteStoreLocators {
    /// This CLI's own [`AgentCliDescriptor::session_store_globs`] — for a CLI
    /// whose title lives in the session file itself.
    StoreGlobs,
    /// File names resolved in this CLI's HOME, the directory above its first
    /// store root — for a CLI whose title lives in a shared index BESIDE the
    /// sessions rather than in any one of them.
    ///
    /// ⚠ Names, never paths: the directory comes from the store globs, so
    /// relocating a CLI's store moves its index with it and nothing here has to
    /// be edited.
    CliHomeFiles(&'static [&'static str]),
    /// Both of the above, store globs first — for a CLI whose title lives in a
    /// shared index for OLD sessions and only in the session's own file for
    /// new ones.
    ///
    /// ⛔ **Neither half alone is enough, and the half that reads like the
    /// obvious one is the half that is empty when it matters.** Measured on a
    /// live Antigravity store: of the eight most recently touched
    /// conversations, ZERO had a row in the summaries index and six had no
    /// entry in the history file — while all eight carried a usable prompt in
    /// their own transcript. A probe wired to the index alone therefore
    /// answers "no title in store" for exactly the rows a person is looking
    /// at, which is the same reading as the defect it was written to repair.
    StoreGlobsAndCliHomeFiles(&'static [&'static str]),
    /// One fixed `$HOME`-relative path — for a CLI whose title lives in a
    /// shared DATABASE rather than in session files, where the glob arms have
    /// nothing to resolve (OpenCode declares no store globs; its titles live
    /// in `~/.local/share/opencode/opencode.db`).
    HomeRelative(&'static str),
}

/// How a live session of a CLI is recognised from a path its process holds open.
///
/// Two shapes, because the two CLIs that need this spell identity differently:
/// one names the session's DIRECTORY, the other names the FILE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveSessionMarker {
    /// `<home>/<root>/…/<session-id>/<file_name>` — the id names the directory
    /// the marker sits in (Muse).
    EnclosingDirectory {
        root: &'static str,
        file_name: &'static str,
    },
    /// `<home>/<root>/<session-id>.<extension>` — the id is the marker's own
    /// file stem (Antigravity).
    FileStem {
        root: &'static str,
        extension: &'static str,
    },
}

impl LiveSessionMarker {
    /// The directory, under `home`, that a marker of this shape must live below.
    /// Declared rather than derived from the store globs: Antigravity's presence
    /// locks sit beside its store, not inside it.
    pub fn root_absolute(&self, home: &Path) -> PathBuf {
        match self {
            Self::EnclosingDirectory { root, .. } | Self::FileStem { root, .. } => home.join(root),
        }
    }

    /// The session id `path` names, if it is a marker of this shape.
    pub fn session_id_of(&self, path: &Path) -> Option<String> {
        match self {
            Self::EnclosingDirectory { file_name, .. } => {
                if path.file_name()? != *file_name {
                    return None;
                }
                Some(path.parent()?.file_name()?.to_str()?.to_string())
            }
            Self::FileStem { extension, .. } => {
                if path.extension()? != *extension {
                    return None;
                }
                Some(path.file_stem()?.to_str()?.to_string())
            }
        }
    }
}

/// The longest a store title may be before it stops being a title and starts
/// being the prompt it was copied from.
const STORE_TITLE_MAX_CHARS: usize = 72;

/// A CLI's own store value, reduced to something that can sit on a sidebar row.
///
/// ⛔ NOT every CLI's "title" column is a title. One of them records the FIRST
/// PROMPT verbatim and never updates it, so a row on the desktop wore nine
/// hundred characters of instructions where its name belongs — measured
/// 2026-08-20, on two rows whose store value ran to whole paragraphs.
///
/// The reduction is deliberately dumb and deterministic: first sentence, then a
/// word-boundary clamp. It costs no model call, which matters because the case
/// it exists for is exactly the case where the model is unreachable — and a
/// clamped first clause is never worse than the paragraph it came from, where a
/// discarded title would leave the row wearing a short hash.
pub fn condense_store_title(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= STORE_TITLE_MAX_CHARS && !trimmed.contains(". ") {
        return Some(trimmed.to_string());
    }
    // The first sentence, when there is one — a prompt's opening sentence is
    // what a person would have called the session.
    let first_sentence = trimmed
        .split_inclusive(['.', '!', '?'])
        .next()
        .unwrap_or(trimmed)
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim();
    let candidate = if first_sentence.is_empty() {
        trimmed
    } else {
        first_sentence
    };
    if candidate.chars().count() <= STORE_TITLE_MAX_CHARS {
        return Some(candidate.to_string());
    }
    let mut clamped = String::new();
    for word in candidate.split_whitespace() {
        let projected = if clamped.is_empty() {
            word.chars().count()
        } else {
            clamped.chars().count() + 1 + word.chars().count()
        };
        if projected > STORE_TITLE_MAX_CHARS {
            break;
        }
        if !clamped.is_empty() {
            clamped.push(' ');
        }
        clamped.push_str(word);
    }
    if clamped.is_empty() {
        clamped = candidate.chars().take(STORE_TITLE_MAX_CHARS).collect();
    }
    Some(clamped)
}

impl AgentCliDescriptor {
    /// This CLI's own record for one session file — the ONE door, because the
    /// raw `read_store_entry` pointer is a per-CLI PARSER and this is where its
    /// result becomes a row.
    ///
    /// It exists so that "a store title is clamped to a row label" is asked once
    /// rather than at each of the four places that read a store. See
    /// [`condense_store_title`] for what the clamp is protecting against.
    pub fn store_entry(&self, path: &Path) -> Option<AgentStoreEntry> {
        let mut entry = (self.read_store_entry)(path)?;
        if let Some(raw) = entry.title.take() {
            let condensed = condense_store_title(&raw);
            // The full text is not thrown away — a paragraph is a fine DETAIL
            // line, and it is what the row's detail would otherwise have to be
            // computed from.
            if entry.detail.is_none()
                && condensed.as_deref().is_some_and(|title| title != raw.trim())
            {
                entry.detail = Some(raw.trim().to_string());
            }
            entry.title = condensed;
        }
        Some(entry)
    }

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

    /// The token an existing session id rides on, whichever shape it takes —
    /// `resume`, `--resume`, `--session`, `--conversation`.
    ///
    /// ⚖ For a READER, not a composer: [`Self::resume_tokens`] stays the one
    /// owner of what is actually emitted. This flattens the two shapes to one
    /// string so a diagnostic can compare CLIs side by side without
    /// re-implementing the match — and the flattening deliberately LOSES the
    /// flag/subcommand distinction, which is why it must never be used to build
    /// a command line.
    pub fn resume_selector_token(&self) -> &'static str {
        match self.resume_selector {
            ResumeSelector::Flag(flag) => flag,
            ResumeSelector::Subcommand(subcommand) => subcommand,
        }
    }

    /// Whether this CLI has a remote arm at all. Derived from
    /// [`Self::wrapper_slug`] so "local-only" is declared once instead of being
    /// a `matches!(kind, CodexLiteLlm)` in the scheme lock, the arm matrices and
    /// the wrapper tables.
    pub fn has_remote_arm(&self) -> bool {
        self.wrapper_slug.is_some()
    }

    /// The remote wrapper subcommand that RESUMES an existing session
    /// (`resume-codex`, `resume-cc`, `resume-kimi`). `None` when local-only.
    pub fn resume_subcommand(&self) -> Option<String> {
        self.wrapper_slug.map(|slug| format!("resume-{slug}"))
    }

    /// The remote wrapper subcommand that STARTS a fresh session.
    pub fn start_subcommand(&self) -> Option<String> {
        self.wrapper_slug.map(|slug| format!("start-{slug}"))
    }

    /// The remote wrapper subcommand that CLOSES a session across the ssh hop.
    ///
    /// ⚠ Its absence is a real, user-visible bug shape: a close that never
    /// crosses the hop leaves the remote CLI running with no row to reach it by.
    pub fn terminate_subcommand(&self) -> Option<String> {
        self.wrapper_slug.map(|slug| format!("terminate-{slug}"))
    }

    /// The remote wrapper subcommand that asks whether a saved session exists.
    pub fn session_exists_subcommand(&self) -> Option<String> {
        self.wrapper_slug.map(|slug| format!("{slug}-session-exists"))
    }

    /// The label the "New … Session" menu entries carry, derived from
    /// [`Self::display_name`] so the menu and the metadata rail cannot disagree
    /// about what this CLI is called.
    ///
    /// ⛔ **A MENU STRING, NOT A ROW NAME.** It was both until 2026-08-21, and
    /// that is how every agent row in a three-machine fleet came to be born
    /// `New Antigravity Session` with nothing in it to say which machine it was
    /// on. A row's birth name is [`new_session_birth_title`], which carries the
    /// machine; this reads well in a menu, where the machine is already implied
    /// by the row the menu was opened on.
    pub fn new_session_label(&self) -> String {
        format!("New {} Session", self.display_name)
    }

    /// The tier that pre-populates a box the user has never set.
    ///
    /// `None` for a CLI with no tiers — one that shares another's box, or one
    /// whose flags are [`PermissionProvenance::Unmeasured`]. A caller must treat
    /// that as "offer nothing", never as "offer the first tier": the first tier
    /// is the SAFEST one, and silently arming it would be yggterm choosing a
    /// posture on the user's behalf.
    pub fn default_permission_preset(&self) -> Option<&'static PermissionPreset> {
        self.permission_presets
            .iter()
            .find(|preset| preset.is_default)
    }

    /// Whether this CLI gets a ROW of its own in the launch-flags modal, or
    /// reads another CLI's box.
    pub fn owns_its_extra_args_box(&self) -> bool {
        self.extra_args_slug == self.slug
    }

    /// Whether this CLI's own store is authoritative for the session title.
    pub fn title_is_store_authoritative(&self) -> bool {
        matches!(self.title_authority, TitleAuthority::Store)
    }

    /// Whether this CLI's SCREEN says a turn is in flight.
    ///
    /// Negations are checked first: they describe lines that FAKE a work
    /// signal (codex's `Worked for 12s` completion summary contains the naive
    /// `worked for` needle).
    pub fn screen_shows_working(&self, sample: &str) -> bool {
        screen_phrases_match(
            sample,
            self.working_screen_phrases,
            self.working_screen_negations,
            SCREEN_FOOTER_WINDOW_ROWS,
        )
    }

    /// Whether this CLI's SCREEN says it is waiting on a usage limit — a third
    /// state beside working/idle. Deliberately NOT subject to the working
    /// negations: those describe lines that fake a WORK signal.
    pub fn screen_shows_limit_wait(&self, sample: &str) -> bool {
        screen_phrases_match(
            sample,
            self.limit_wait_screen_phrases,
            &[],
            SCREEN_FOOTER_WINDOW_ROWS,
        )
    }

    /// Whether this CLI's SCREEN is holding an OWNER-FACING QUESTION PICKER —
    /// the state in which typed text goes nowhere because the CLI is reading
    /// navigation keys.
    pub fn screen_shows_question_picker(&self, sample: &str) -> bool {
        screen_phrases_match(
            sample,
            self.question_picker_screen_phrases,
            &[],
            SCREEN_FOOTER_WINDOW_ROWS,
        )
    }

    /// Whether this CLI's SCREEN is advertising a background agent, i.e. the
    /// dim composer line is CHROME and not a draft. See
    /// [`Self::background_agent_hint_screen_phrases`].
    pub fn screen_shows_background_agent_hint(&self, sample: &str) -> bool {
        screen_phrases_match(
            sample,
            self.background_agent_hint_screen_phrases,
            &[],
            SCREEN_FOOTER_WINDOW_ROWS,
        )
    }

    /// Whether this CLI's SCREEN is holding a STARTUP GATE — a first-run modal
    /// standing before the composer exists. See
    /// [`Self::startup_gate_screen_phrases`].
    ///
    /// ⛔ THIS ONE NEEDS A RENDERED GRID, not the raw screen. A gate is painted
    /// with absolute cursor moves rather than newlines, so on the raw byte
    /// stream its nine visible rows arrive as two `\n`-delimited lines with the
    /// words of adjacent rows fused together. Feed it
    /// `session_screen_plain_rows`, never `session_screen_snapshot`.
    ///
    /// ⚠ Its window is deeper than the footer classifiers' because a modal sits
    /// in the MIDDLE of the screen with blank rows beneath it, while footer
    /// chrome is by construction last.
    pub fn screen_shows_startup_gate(&self, sample: &str) -> bool {
        screen_phrases_match(
            sample,
            self.startup_gate_screen_phrases,
            &[],
            SCREEN_MODAL_WINDOW_ROWS,
        )
    }

    /// Whether this CLI's SCREEN carries LIMIT/BILLING DIALOG wording. See
    /// [`Self::plan_limit_choice_screen_phrases`].
    ///
    /// ⛔ NOT SUFFICIENT ON ITS OWN. This answers "are the money words present",
    /// which prose about usage limits also satisfies. The state is only armed
    /// when a selection marker also sits on a numbered option — see
    /// [`crate::screen_state::classify_screen`].
    pub fn screen_shows_plan_limit_choice(&self, sample: &str) -> bool {
        screen_phrases_match(
            sample,
            self.plan_limit_choice_screen_phrases,
            &[],
            SCREEN_MODAL_WINDOW_ROWS,
        )
    }

    /// Tokens for the CLI's own resume PICKER (no session id).
    pub fn resume_picker_tokens(&self) -> Vec<String> {
        match self.resume_selector {
            ResumeSelector::Flag(flag) => vec![flag.to_string()],
            ResumeSelector::Subcommand(sub) => vec![sub.to_string()],
        }
    }

    /// One sentence a human can act on when this CLI's binary is not on the
    /// machine a session is trying to launch it on.
    ///
    /// The descriptor already declares HOW the binary arrives ([`CliInstall`]);
    /// this turns that declaration into the words a refusal shows, so a launch
    /// that cannot run names the CLI *and* the way to fix it. Owned here rather
    /// than at the refusal site because the refusal site is not the thing that
    /// knows — a second copy beside the launcher is exactly how the install
    /// method and the message drift apart.
    pub fn install_instruction(&self) -> String {
        match self.install {
            CliInstall::Npm(package) => format!(
                "yggterm provisions {} from npm ({package}) — an install may be in flight, so retry in a moment.",
                self.binary_name
            ),
            CliInstall::Uv(package) => format!(
                "yggterm provisions {} with uv ({package}) — an install may be in flight, so retry in a moment.",
                self.binary_name
            ),
            CliInstall::VendorScript(url) => format!(
                "yggterm provisions {} with the vendor installer at {url} — an install may be in flight, so retry in a moment.",
                self.binary_name
            ),
            CliInstall::Manual => format!(
                "Install `{}` by hand — yggterm never provisions this CLI.",
                self.binary_name
            ),
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

    /// The `$HOME`-relative directory this CLI keeps its own state in — the
    /// parent of its first store root (`.gemini/antigravity-cli` for a store
    /// rooted at `.gemini/antigravity-cli/conversations`).
    ///
    /// ⚖ Derived, never declared, so a CLI that relocates its store carries its
    /// sibling index with it and no second table has to be edited to agree.
    /// `None` when the store root has no parent segment, or when this CLI
    /// declares no store at all.
    pub fn cli_home_relative(&self) -> Option<&'static str> {
        let root = self.store_roots().into_iter().next()?;
        root.rsplit_once('/').map(|(parent, _leaf)| parent)
    }

    /// The `$HOME`-relative locators this CLI's remote title probe is handed,
    /// as argv, before the `--` separator.
    ///
    /// Empty ⇒ no probe, or a probe whose locators cannot be resolved — either
    /// way the caller must not run the round trip.
    pub fn remote_store_title_locators(&self) -> Vec<String> {
        let Some(probe) = self.remote_live_store_title else {
            return Vec::new();
        };
        match probe.locators {
            RemoteStoreLocators::StoreGlobs => self
                .session_store_globs
                .iter()
                .map(|glob| (*glob).to_string())
                .collect(),
            RemoteStoreLocators::CliHomeFiles(names) => self.cli_home_file_locators(names),
            RemoteStoreLocators::StoreGlobsAndCliHomeFiles(names) => self
                .session_store_globs
                .iter()
                .map(|glob| (*glob).to_string())
                .chain(self.cli_home_file_locators(names))
                .collect(),
            RemoteStoreLocators::HomeRelative(path) => vec![(*path).to_string()],
        }
    }

    /// Named files resolved under [`Self::cli_home_relative`]. Empty when this
    /// CLI declares no store, since then there is no home to resolve against.
    fn cli_home_file_locators(&self, names: &[&'static str]) -> Vec<String> {
        let Some(home) = self.cli_home_relative() else {
            return Vec::new();
        };
        names
            .iter()
            .map(|name| format!("{home}/{name}"))
            .collect()
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

    /// Whether `path` IS one of this CLI's durable store FILES (the one
    /// sqlite db for a one-file CLI) — the stale persisted identity a row
    /// must never carry, since the store holds EVERY session and a path-keyed
    /// consumer cannot tell siblings apart. Home-relative declaration, so
    /// both `/home/u/<entry>` and a bare `<entry>` match.
    pub fn durable_store_file_is(&self, path: &str) -> bool {
        let trimmed = path.trim();
        self.durable_store_files.iter().any(|file| {
            trimmed.ends_with(file.trim_start_matches('/'))
                || trimmed.ends_with(&format!("/{}", file.trim_start_matches('/')))
        })
    }

    /// The one owner of "is this path a stale store-container identity" —
    /// a declared durable store FILE of some CLI. File-per-session CLIs
    /// declare none and never match (their store paths are directories).
    pub fn path_is_durable_store_container(path: &str) -> bool {
        AGENT_CLIS
            .iter()
            .any(|descriptor| descriptor.durable_store_file_is(path))
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
    /// matched, and not an excluded name. A fragment that contains `/` is
    /// matched against the whole `path` (so `"/subagent/"` excludes Muse
    /// sub-agent sessions that live under the parent's directory); otherwise
    /// the match is against the file name only (so `".bak."` excludes Codex
    /// backups without anchoring to a directory).
    pub fn store_path_is_session_file(&self, path: &str) -> bool {
        if self.store_excluded_name_fragments.iter().any(|fragment| {
            if fragment.contains('/') {
                path.contains(fragment)
            } else {
                file_name_of(path).contains(fragment)
            }
        }) {
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
/// The npm dist-tag a CLI installs from, when the vendor's `latest` is not
/// the line the owner chose. OpenCode's v2 binary ships as `@opencode-ai/cli`
/// on the `beta` tag (build-numbered versions) while `latest` there stays one
/// step behind and the UNSCOPED `opencode-ai@beta` is the abandoned v1 line
/// (owner directive 2026-08-26; wrong-package pin fixed 2026-08-28), so the
/// provisioner must resolve the tag against THIS package, not assume the word
/// "latest" nor that the tag exists on every package that ever carried it. A
/// fn over [`SessionKind`] rather than a descriptor field: it is provisioning
/// policy (revisable in one place) and adding a descriptor field would ripple
/// through every registry literal.
pub fn npm_dist_tag(kind: SessionKind) -> Option<&'static str> {
    match kind {
        SessionKind::OpenCode => Some("beta"),
        _ => None,
    }
}

pub const AGENT_CLIS: &[AgentCliDescriptor] = &[
    AgentCliDescriptor {
        kind: SessionKind::Codex,
        display_name: "Codex",
        session_metadata_label: "Codex Session",
        slug: "codex",
        binary_name: "codex",
        install: CliInstall::Npm("@openai/codex"),
        update: CliUpdate::Reinstall,
        icon_glyph: ">_",
        // OpenAI's teal, darkened to clear AA against white (5.47:1).
        brand_color: "#0f766e",
        menu_hint: 'c',
        // Codex records no title of its own; yggterm's LLM chore writes one.
        title_authority: TitleAuthority::Generated,
        // Codex launches bare and discovers its ULID later, so the synthesized
        // `local://<uuid4>` has to be rebound once the transcript appears.
        id_assigned_at_birth: false,
        wrapper_slug: Some("codex"),
        // ⚠ HISTORICAL, and deliberately not `remote-codex://`: this string is
        // in every persisted state file on the fleet. The slug drives new CLIs;
        // it may not retroactively rename a shipped one.
        remote_row_scheme: Some("remote-session://"),
        runtime_key_scheme: Some("codex-runtime://"),
        // Codex prints no `esc to interrupt` on a plain turn; the two shapes
        // below are its BACKGROUND-task indicators, and both need their partner
        // phrase or they match ordinary prose.
        working_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "esc to interrupt",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "working (",
                also_any: &["/stop to close", "background terminal running"],
            },
        ],
        // `Worked for 12s` is codex's COMPLETION summary, not active work.
        working_screen_negations: &["worked for "],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        // ⭐ MEASURED 2026-08-22 from a real codex row spawned into a directory this
        // CLI had never opened. The gate asks `Do you trust the contents of this
        // directory?` over `› 1. Yes, continue` / `2. No, quit`, with `Press enter
        // to continue` beneath.
        //
        // ⛔⛔ THIS EMPTY LIST HAD A CAUSAL CHAIN BEHIND IT, WATCHED END TO END.
        // Unrecognised, the gate classified as `ready` with `may_type: true`, so a
        // delivery verb submitted into it. A picker consumes navigation keys, and
        // one of these options is `No, quit` — so the CLI exited, the daemon
        // relaunched it, and the fresh process came up at the same gate reporting
        // `idle` again. A brief aimed at that row is eaten every time, and nothing
        // anywhere reports a failure.
        //
        // ⚠ Two witnesses on their own rows and no `also_any`, for the same reason
        // as Claude Code's: the gate paints each on its own visible line, so a
        // same-line conjunction would demand an adjacency the screen does not have.
        startup_gate_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "do you trust the contents of this directory",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "no, quit",
                also_any: &[],
            },
        ],
        plan_limit_choice_screen_phrases: &[],
        resume_selector: ResumeSelector::Subcommand("resume"),
        // `codex resume <id>` reopens the session's ORIGINAL cwd unless
        // re-rooted; the cwd tree's whole promise is that a row opens where the
        // tree says it lives.
        resume_re_roots_with_cwd: true,
        model_flag: "--model",
        composer_marker: '\u{203a}',
        composer_footer_hints: &["gpt-", "claude", "tab to ", "ctrl", "esc"],
        // ⛔ UNMEASURED. Codex's in-flight phrase has never been observed on a
        // live working row, so this stays empty and the activity verdict for a
        // codex row is `Unknown` rather than a guess. Fill it from a screen,
        // not from memory.
        working_footer_hints: &[],
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
        extra_args_slug: "codex",
        // ⭐ Codex's real home for this is `~/.codex/config.toml`
        // (`approval_policy`, `sandbox_mode`, per-project trust), and `-c
        // key=value` overrides any of it per launch — so this box is a
        // PER-LAUNCH OVERRIDE of a file the user may also be editing. The modal
        // says so, and never writes that file.
        //
        // Approval policies in this build: `untrusted`, `on-request`, `never`.
        // ⚠ There is no `on-failure` and no `--full-auto`; do not offer either.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "-a untrusted -s read-only",
                explanation: "Runs only trusted commands (ls, cat, sed) unasked and escalates \
                              everything else; the filesystem is read-only.",
                is_default: false,
            },
            PermissionPreset {
                id: "sandboxed",
                label: "Sandboxed",
                args: "-a on-request -s workspace-write",
                explanation: "The model decides when to ask; writes are confined to the \
                              workspace.",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "-s danger-full-access",
                explanation: "No sandbox: model-generated commands run against the whole \
                              machine. Confirmation prompts are still asked — the tier beside \
                              this one is what removes those too.",
                is_default: true,
            },
            // ⚠ A FOURTH tier, and it is not decoration: this is the exact
            // posture `--permission-mode bypass` emits for codex, and without it
            // the modal could not reach a posture a delegate launch can ask for
            // by name. `--dangerously-bypass-hook-trust` is a SEPARATE danger
            // switch (it runs hooks without persisted trust) and is deliberately
            // not folded in here.
            PermissionPreset {
                id: "bypass-all",
                label: "Skip checks and prompts",
                args: "--dangerously-bypass-approvals-and-sandbox",
                explanation: "No sandbox and no confirmation prompts at all. \
                              --dangerously-bypass-hook-trust is a separate switch that also \
                              runs hooks without persisted trust; it is not included here.",
                is_default: false,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        // Codex files sessions by date: `~/.codex/sessions/2026/07/25/
        // rollout-2026-07-25T…-<uuid>.jsonl`, so the depth is not fixed.
        session_store_globs: &[".codex/sessions/**/rollout-*.jsonl"],
        store_excluded_name_fragments: &[".bak."],
        durable_store_files: &[],
        store_scan_gap: None,
        store_home_env_override: Some(crate::ENV_YGGTERM_CODEX_HOME),
        read_store_entry: read_codex_store_entry,
        store_membership_index: None,
        live_session_marker: None,
        // Owner spec 2026-06-06: codex titles are YGGTERM-owned. The 12s
        // title chore serves live codex rows from the cached title or the
        // rollout's first real user prompt (the wrappers skipped).
        read_live_store_title: Some(read_codex_live_store_title),
        remote_live_store_title: Some(CODEX_REMOTE_TITLE_PROBE),
    },
    AgentCliDescriptor {
        kind: SessionKind::CodexLiteLlm,
        display_name: "Codex-LiteLLM",
        session_metadata_label: "Codex-LiteLLM Session",
        slug: "codex-litellm",
        binary_name: "codex-litellm",
        // ⚠ CORRECTED 2026-08-08 from `CliInstall::Manual` / "yggterm never
        // installs it", which was a GUESS and was false. MEASURED on this host:
        // `~/.yggterm/npm/lib/node_modules/@avikalpa/codex-litellm` is under
        // the yggterm-managed npm prefix and `~/.yggterm/npm/bin/codex-litellm`
        // is the binary sessions actually run — the 6-hourly refresh has been
        // provisioning it all along, in the same batch as codex and claude.
        // The wrong value would have quietly stopped that.
        install: CliInstall::Npm("@avikalpa/codex-litellm"),
        update: CliUpdate::Reinstall,
        icon_glyph: ">_",
        // A cool sibling of Codex's teal — same family, because it IS codex
        // behind a proxy, but separable at a glance (5.93:1).
        brand_color: "#0369a1",
        menu_hint: 'z',
        title_authority: TitleAuthority::Generated,
        id_assigned_at_birth: false,
        // ⛔ LOCAL-ONLY, and this is the declaration that says so. It replaces
        // the `matches!(kind, CodexLiteLlm)` that the scheme lock, both arm
        // matrices and the wrapper tables each carried their own copy of.
        wrapper_slug: None,
        remote_row_scheme: None,
        runtime_key_scheme: None,
        working_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "esc to interrupt",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "working (",
                also_any: &["/stop to close", "background terminal running"],
            },
        ],
        working_screen_negations: &["worked for "],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
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
        // ⛔ UNMEASURED. Codex's in-flight phrase has never been observed on a
        // live working row, so this stays empty and the activity verdict for a
        // codex row is `Unknown` rather than a guess. Fill it from a screen,
        // not from memory.
        working_footer_hints: &[],
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
        // ⛔ ONE CLI, ONE BOX — this reads and writes CODEX's stored flags.
        // Settled 2026-08-08: `codex-anything` is a codex session's flip switch,
        // not a CLI of its own, so it must never grow a second box that can hold
        // a different sandbox policy from the codex row beside it. Declaring the
        // shared key here is also what makes it absent from the launch-flags
        // modal without the modal owning a skip-list.
        extra_args_slug: "codex",
        // Empty BECAUSE the box is codex's, not because the tiers are unknown —
        // `every_cli_declares_presets_or_says_why` separates those two cases.
        permission_presets: &[],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        session_store_globs: &[".codex-litellm/sessions/**/rollout-*.jsonl"],
        store_excluded_name_fragments: &[".bak."],
        durable_store_files: &[],
        store_scan_gap: None,
        // No override: only `resolve_codex_home` consults an env var, and it
        // relocates `.codex` alone. Preserving that exactly.
        store_home_env_override: None,
        read_store_entry: read_codex_store_entry,
        store_membership_index: None,
        live_session_marker: None,
        // Owner spec 2026-06-06: codex titles are YGGTERM-owned. The 12s
        // title chore serves live codex rows from the cached title or the
        // rollout's first real user prompt (the wrappers skipped).
        read_live_store_title: Some(read_codex_live_store_title),
        // ⛔ Codex-LiteLLM has no remote arm — its remote-*:// rows never
        // exist, so a probe here would be dead weight the coverage lock
        // rightly refuses.
        remote_live_store_title: None,
    },
    AgentCliDescriptor {
        kind: SessionKind::ClaudeCode,
        display_name: "Claude Code",
        session_metadata_label: "Claude Code Session",
        slug: "claude-code",
        binary_name: "claude",
        install: CliInstall::Npm("@anthropic-ai/claude-code"),
        update: CliUpdate::Reinstall,
        icon_glyph: "*_",
        // Claude's clay, at the darkest step that still reads as the brand.
        // Replaces `#d97706`, which was the shipped value and failed AA at
        // 3.19:1 against the white label it carried (5.18:1).
        brand_color: "#c2410c",
        menu_hint: 'l',
        // CC does the hard work of titling its own sessions and yggterm must
        // RESPECT that, writing back only on an explicit user rename
        // (`spec-codex-cc-title-summary`, user decision 2026-06-06).
        title_authority: TitleAuthority::Store,
        // CC launches with `--session-id <uuid>`, so the row id IS the
        // transcript id from birth and no rebind poll is needed.
        id_assigned_at_birth: true,
        // ⚠ HISTORICAL `cc`, not `claude-code`: `resume-cc` / `start-cc` are in
        // the Connect strings the metadata rail shows the user and in scripts.
        wrapper_slug: Some("cc"),
        remote_row_scheme: Some("remote-cc://"),
        runtime_key_scheme: Some("cc-runtime://"),
        // Measured on guihost 2026-08-07 — see `working_footer_hints` below for the
        // three-row comparison this came from.
        working_screen_phrases: &[ScreenWorkingPhrase {
            needle: "esc to interrupt",
            also_any: &[],
        }],
        working_screen_negations: &[],
        // Measured from the owner's report 2026-08-20: a session mid
        // usage-limit wait paints a footer of the form
        // `Usage limit reached · continuing shortly · esc to cancel` while
        // the working phrase is absent — which is why the state read as
        // "confirmed idle" everywhere until this existed. The also_any
        // guard keeps conversation text that merely MENTIONS a usage limit
        // from arming the state: the wait footer carries its own
        // continuation/cancel wording on the same line.
        limit_wait_screen_phrases: &[ScreenWorkingPhrase {
            needle: "usage limit reached",
            also_any: &["continuing", "esc to cancel"],
        }],
        // ⭐ MEASURED 2026-08-21 by driving a real `claude` in a pty until it
        // raised a one-question picker, and by reading the shipped binary's own
        // literals. The picker's footer renders as
        // `Enter to select · ↑/↓ to navigate · Esc to cancel` for a single
        // question and swaps the middle chord for the literal
        // `Tab/Arrow keys to navigate` when there is more than one — so the
        // needle is the half both spellings share, guarded by the neighbours on
        // the SAME line so a sentence that merely says "navigate" cannot arm it.
        // The review step of a multi-question picker paints no navigate footer
        // at all, hence the second phrase; the third is the CLI's generic
        // select-list helper (`(Use arrow keys)` / `(Use arrow keys to reveal
        // more choices)`), which every menu and permission prompt renders — and
        // all of those eat typed text exactly the same way.
        question_picker_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "to navigate",
                also_any: &["to select", "to cancel"],
            },
            ScreenWorkingPhrase {
                needle: "ready to submit your answers",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "(use arrow keys",
                also_any: &[],
            },
        ],
        // ⭐ MEASURED 2026-08-21 from a real `claude` driven in a pty: the
        // composer's own line reads `❯   · ← 1 agent` while a background agent
        // runs, and the mode footer above it carries `← for agents`. The count
        // varies and the wording around it does not, so the test is the arrow
        // glyph and the word on the SAME line — either alone is common English,
        // and together on one line they are this chrome.
        background_agent_hint_screen_phrases: &[ScreenWorkingPhrase {
            needle: "\u{2190}",
            also_any: &["agent"],
        }],
        // ⭐ MEASURED 2026-08-21 by spawning a row into a directory this CLI
        // had never opened on this host. The gate heads itself
        // `Quick safety check: Is this a project you created or one you trust?`
        // and offers `❯ 1. Yes, I trust this folder` over `2. No, exit`, with
        // `Enter to confirm · Esc to cancel` beneath. The heading is the whole
        // test on its own — no other screen in this CLI carries it — and the
        // option line is kept as a second, independent witness so a reworded
        // heading cannot blind the classifier silently.
        //
        // ⛔ Neither phrase takes an `also_any` guard, deliberately. The gate
        // paints each of these on its OWN visible row, so a same-line
        // conjunction would demand an adjacency the screen does not have.
        startup_gate_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "quick safety check",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "trust this folder",
                also_any: &[],
            },
        ],
        // The wording the fleet's watchdog has matched on since 2026-08-14,
        // moved here from a python tuple so the daemon and the watcher cannot
        // hold different ideas of what a billing dialog looks like. Every entry
        // names the money question itself; the structural guard beside it is
        // what keeps an ordinary sentence about limits from arming the state.
        plan_limit_choice_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "session limit",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "usage limit",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "team account",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "api billing",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "pay per token",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "stop and wait",
                also_any: &[],
            },
        ],
        resume_selector: ResumeSelector::Flag("--resume"),
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_marker: '\u{276f}',
        composer_footer_hints: &["claude", "permissions", "shift+tab", "for agents", "ctrl", "esc"],
        // Measured on guihost 2026-08-07 by comparing three live rows in one
        // snapshot: a working row's footer reads
        // `⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt · ← 1 agent`,
        // an idle row's the same WITHOUT `esc to interrupt`. The owner's paused
        // row proved it: it carried `← 1 agent` and no interrupt hint, and I
        // had called it "grinding" off a liveness probe.
        working_footer_hints: &["esc to interrupt"],
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
        extra_args_slug: "claude-code",
        // Modes in this build: acceptEdits, auto, bypassPermissions, manual,
        // dontAsk, plan. `--allowedTools` / `--disallowedTools` take tool-name
        // lists for a middle ground, and `settings.json` carries a `permissions`
        // block that outlives any single launch.
        //
        // ⚠ Two flags one letter apart: `--dangerously-skip-permissions` IS the
        // bypass; `--allow-dangerously-skip-permissions` only ENABLES it being
        // used. The tier offers the first and the explanation names the second,
        // so nobody pastes it expecting the bypass.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "--permission-mode manual",
                explanation: "Every tool use is confirmed by you.",
                is_default: false,
            },
            PermissionPreset {
                id: "sandboxed",
                label: "Sandboxed",
                args: "--permission-mode acceptEdits",
                explanation: "File edits apply without asking; commands still ask.",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "--dangerously-skip-permissions",
                explanation: "Bypasses all permission checks. Recommended by Anthropic only for \
                              sandboxes with no internet access. Note \
                              --allow-dangerously-skip-permissions is a DIFFERENT flag: it only \
                              permits the bypass, it does not perform it.",
                is_default: true,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        // CC files one flat dir per cwd, the dir name being the cwd with every
        // character outside [A-Za-z0-9-] replaced: `~/.claude/projects/
        // -home-user-gh-yggterm/<session-uuid>.jsonl`. Exactly one level.
        session_store_globs: &[".claude/projects/*/*.jsonl"],
        store_excluded_name_fragments: &[],
        durable_store_files: &[],
        store_scan_gap: None,
        store_home_env_override: None,
        read_store_entry: read_claude_code_store_entry,
        store_membership_index: None,
        live_session_marker: None,
        read_live_store_title: Some(read_claude_code_live_store_title),
        remote_live_store_title: Some(CLAUDE_CODE_REMOTE_TITLE_PROBE),
    },
    // ── The 2026-08-08 intake. Every field below was read off the CLI's own
    // source or its installed binary on this date, never from memory; the
    // provenance is on each row. See `docs/spec-adding-an-agent-cli.md`.
    AgentCliDescriptor {
        kind: SessionKind::Pi,
        display_name: "Pi",
        session_metadata_label: "Pi Session",
        slug: "pi",
        binary_name: "pi",
        install: CliInstall::Npm("@earendil-works/pi-coding-agent"),
        update: CliUpdate::Reinstall,
        // The mathematical constant is pi's own mark.
        icon_glyph: "\u{3c0}_",
        // Nearest available: Pi ships no published brand hex (6.04:1).
        brand_color: "#be185d",
        menu_hint: 'p',
        // No auto-title anywhere in the source; `/name` and `--name` are the
        // only writers, so an untitled pi session has nothing for yggterm to
        // respect and the LLM chore owns it.
        title_authority: TitleAuthority::Generated,
        // `pi --session-id <id>` creates the session if it is missing
        // (`src/main.ts`), the closest analogue to Claude Code's birth id.
        id_assigned_at_birth: true,
        wrapper_slug: Some("pi"),
        remote_row_scheme: Some("remote-pi://"),
        runtime_key_scheme: Some("pi-runtime://"),
        // `Working... (escape to interrupt)` — the message is composed from
        // `defaultWorkingMessage` plus the resolved interrupt binding, so match
        // the stable half. `Thinking...` is the hidden-reasoning variant.
        working_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "working...",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "thinking...",
                also_any: &[],
            },
        ],
        working_screen_negations: &[],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
        resume_selector: ResumeSelector::Flag("--session"),
        // `pi` takes `process.cwd()`; there is no `--cwd`, and `--session-dir`
        // relocates STORAGE, not the working directory.
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_marker: '\u{203a}',
        composer_footer_hints: &["esc", "ctrl", "/help", "tab"],
        working_footer_hints: &["to interrupt"],
        // ⛔⛔ pi HAS NO PERMISSION GATE AT ALL, and that is its DOCUMENTED
        // DESIGN, not a gap in our reading. Its own README: "No permission
        // popups. Run in a container, or build your own confirmation flow with
        // extensions." ⇒ there is nothing to bypass, so `Bypass` stays absent
        // and is refused by name. Re-measured on the installed binary
        // 2026-08-13: `pi --help` offers --tools/--no-tools/--exclude-tools and
        // --approve/--no-approve, and nothing that gates a tool CALL.
        permission_modes: &[(AgentPermissionMode::Default, &[])],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            ("--approve", FlagArity::Standalone, OverriddenBy::PermissionMode),
            ("-a", FlagArity::Standalone, OverriddenBy::PermissionMode),
            (
                "--no-approve",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
            ("-na", FlagArity::Standalone, OverriddenBy::PermissionMode),
        ],
        extra_args_slug: "pi",
        // ⇒ The explanations say the quiet part out loud: EVERY pi session runs
        // its tools unprompted. What these flags actually control is trust of
        // project-local SETTINGS FILES, not tool calls, and a ladder that looked
        // like Claude's without saying so would mislead.
        //
        // ⚠ Each tier's args are RUNNABLE AS WRITTEN. `--tools <names>` is not —
        // a placeholder pasted into a launch is a launch that fails — so the
        // restricted tier offers the complete `--no-tools` and names the
        // narrower flags in prose. Global trust default lives in pi's own
        // settings as `defaultProjectTrust` (ask · never · always).
        permission_presets: &[
            PermissionPreset {
                id: "restricted",
                label: "No tools",
                args: "--no-tools",
                explanation: "Disables all built-in tools and extensions — pi's only real safety \
                              control. For a narrower set use --tools read,grep or \
                              --exclude-tools bash.",
                is_default: false,
            },
            PermissionPreset {
                id: "no-project-trust",
                label: "Ignore project settings",
                args: "--no-approve",
                explanation: "Nothing in the repo can widen what pi may do this run.",
                is_default: false,
            },
            PermissionPreset {
                id: "project-trust",
                label: "Trust project settings",
                args: "--approve",
                explanation: "Trusts project-local settings files for this run. This is NOT a \
                              tool-permission bypass — pi never asks about tool calls either way.",
                is_default: true,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        // `~/.pi/agent/sessions/--<cwd-with-separators-hyphenated>--/
        // <timestamp>_<uuid>.jsonl`; line 1 is the session header carrying
        // `id`, `cwd` and `timestamp`.
        session_store_globs: &[".pi/agent/sessions/*/*.jsonl"],
        store_excluded_name_fragments: &[],
        durable_store_files: &[],
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_pi_store_entry,
        store_membership_index: None,
        live_session_marker: None,
        // The 12-second title chore serves live pi rows from the session's
        // own jsonl (header id == file name uuid — measured 2026-08-30).
        read_live_store_title: Some(read_pi_live_store_title),
        remote_live_store_title: Some(PI_REMOTE_TITLE_PROBE),
    },
    AgentCliDescriptor {
        kind: SessionKind::OpenCode,
        display_name: "OpenCode",
        session_metadata_label: "OpenCode Session",
        slug: "opencode",
        binary_name: "opencode2",
        // ⛔ TWO packages, ONE tag name: `opencode-ai@beta` is the ABANDONED
        // v1-line beta (date-stamped versions, frozen upstream); the v2 line
        // the owner directed (2026-08-26) ships as `@opencode-ai/cli@beta`
        // (build-numbered versions) and installs the binary `opencode2`.
        // Pinning the right tag on the wrong package is how the managed
        // install served a frozen August beta while every terminal ran the
        // 2.0 preview. Naming the wrong one is how provisioning silently
        // installs nothing.
        install: CliInstall::Npm("@opencode-ai/cli"),
        update: CliUpdate::Reinstall,
        icon_glyph: "OC_",
        // Nearest available (7.90:1).
        brand_color: "#4338ca",
        menu_hint: 'o',
        // A title agent EXISTS in the tree but is not wired into the v2 runner
        // (an explicit TODO there), and creation writes the placeholder
        // `New session - <iso>`. So the store is not authoritative today.
        title_authority: TitleAuthority::Generated,
        // ⛔ The CLI REFUSES an unknown `--session <id>` outright; a caller
        // must mint the session over opencode's own RPC first. So yggterm may
        // not assume a birth id.
        id_assigned_at_birth: false,
        wrapper_slug: Some("opencode"),
        remote_row_scheme: Some("remote-opencode://"),
        runtime_key_scheme: Some("opencode-runtime://"),
        working_screen_phrases: &[ScreenWorkingPhrase {
            needle: "esc interrupt",
            also_any: &[],
        }],
        working_screen_negations: &[],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
        resume_selector: ResumeSelector::Flag("--session"),
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_marker: '\u{276f}',
        composer_footer_hints: &["esc", "interrupt", "ctrl", "tab"],
        working_footer_hints: &["esc interrupt", "again to interrupt"],
        // `--auto` re-measured on the installed binary 2026-08-13: "auto-approve
        // permissions that are not explicitly denied (dangerous!)" — opencode's
        // own words. It was absent from this table while the flag existed, so
        // `--permission-mode bypass --kind opencode` was refused on a CLI that
        // can express it.
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (AgentPermissionMode::Bypass, &["--auto"]),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            ("--auto", FlagArity::Standalone, OverriddenBy::PermissionMode),
        ],
        extra_args_slug: "opencode",
        // ⭐ opencode's permission model is a CONFIG FILE and the flag only
        // raises the floor: `opencode.json` takes a `permission` block keyed by
        // tool with values allow · ask · deny (glob patterns inside `bash`), and
        // `--auto` RESPECTS `deny` while overriding `ask`. That is the one case
        // where a user's config still constrains this box's value, so the
        // explanation has to say it.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "",
                explanation: "opencode's own default: each permission is asked.",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "--auto",
                explanation: "Auto-approves every permission that is not explicitly denied — \
                              opencode's own help calls this dangerous. A `deny` in opencode.json \
                              still holds; only `ask` is overridden.",
                is_default: true,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        // ⚠ The default TUI owns the screen and repaints via opentui; there is
        // no scrollback transcript to re-derive. `--mini` is the streaming
        // variant that does replay.
        content_rederives_on_resume: false,
        session_store_globs: &[],
        store_excluded_name_fragments: &[],
        // The ONE durable store file — every session lives in this sqlite db,
        // so row identity is the store scheme + session id, never this path.
        durable_store_files: &[".local/share/opencode/opencode.db"],
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_no_store_entry,
        store_membership_index: Some(opencode_store_index_holds_session),
        live_session_marker: None,
        // opencode2 self-titles prompted sessions in session_v2 (the scanner
        // reads the same column); the chore reads it for live rows too.
        read_live_store_title: Some(read_opencode_live_store_title),
        remote_live_store_title: Some(OPENCODE_REMOTE_TITLE_PROBE),
    },
    AgentCliDescriptor {
        kind: SessionKind::QwenCode,
        display_name: "Qwen Code",
        session_metadata_label: "Qwen Code Session",
        slug: "qwen-code",
        binary_name: "qwen",
        install: CliInstall::Npm("@qwen-code/qwen-code"),
        update: CliUpdate::Reinstall,
        icon_glyph: "Q_",
        // Qwen's violet (7.10:1).
        brand_color: "#6d28d9",
        menu_hint: 'q',
        // Qwen generates and PERSISTS its own title as a `custom_title` record
        // and re-appends it near EOF as the file grows, so a tail scan finds
        // it. yggterm must respect that.
        title_authority: TitleAuthority::Store,
        // `qwen --session-id <uuid>` — but a collision is FATAL, so the caller
        // must mint a fresh uuid, never reuse a row id it already used.
        id_assigned_at_birth: true,
        wrapper_slug: Some("qwen"),
        remote_row_scheme: Some("remote-qwen://"),
        runtime_key_scheme: Some("qwen-runtime://"),
        // ⚠ i18n'd through `t()`, so a non-English locale changes it. The
        // store and the runtime sidecar are the reliable signals; this is the
        // screen fallback.
        working_screen_phrases: &[ScreenWorkingPhrase {
            needle: "esc to cancel",
            also_any: &[],
        }],
        working_screen_negations: &[],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
        resume_selector: ResumeSelector::Flag("--resume"),
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_marker: '\u{276f}',
        composer_footer_hints: &["esc", "ctrl", "qwen", "tab"],
        working_footer_hints: &["esc to cancel"],
        // ⛔ THESE FLAGS ARE HIDDEN FROM `qwen --help`, and the first pass filed
        // them as non-existent because of it. Confirmed by a controlled probe of
        // the PARSER, re-run 2026-08-13: `qwen --approval-mode bogus` answers
        // `Choices: "plan", "default", "auto-edit", "auto", "yolo"`. ⇒ `--help`
        // is not a CLI's contract; probe the binary, then read the docs.
        permission_modes: &[
            // ⚠ Default emits NOTHING, not `--approval-mode default`, and the
            // lock caught the first draft doing the latter: a value we do not
            // need to send is a value that cannot rot when the vendor renames
            // its own choices. The PRESET below spells it out because a human
            // reading a box wants to see which posture is in force; the launch
            // vocabulary does not.
            (AgentPermissionMode::Default, &[]),
            (AgentPermissionMode::Plan, &["--approval-mode", "plan"]),
            (
                AgentPermissionMode::AcceptEdits,
                &["--approval-mode", "auto-edit"],
            ),
            (AgentPermissionMode::Bypass, &["--yolo"]),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            (
                "--approval-mode",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            ("--yolo", FlagArity::Standalone, OverriddenBy::PermissionMode),
            ("-y", FlagArity::Standalone, OverriddenBy::PermissionMode),
        ],
        extra_args_slug: "qwen-code",
        // ⚠ `-s` (sandbox) is an ORTHOGONAL axis, composable with any approval
        // mode — so the sandboxed tier pins an approval mode too rather than
        // shipping a half-statement, and says the composition is available for a
        // custom line. Settings file carries `approvalMode` and `trustedFolders`.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "--approval-mode default",
                explanation: "Every tool call is confirmed. --approval-mode plan is read-only \
                              planning.",
                is_default: false,
            },
            PermissionPreset {
                id: "auto-edit",
                label: "Auto-edit",
                args: "--approval-mode auto-edit",
                explanation: "File edits apply unprompted; commands still ask.",
                is_default: false,
            },
            PermissionPreset {
                id: "sandboxed",
                label: "Sandboxed",
                args: "-s --approval-mode default",
                explanation: "Runs the session inside Qwen's sandbox and still confirms every \
                              tool call. -s composes with ANY approval mode, so add it to a \
                              custom line for sandbox plus auto-edit.",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "--yolo",
                explanation: "Auto-approves everything (equivalent to --approval-mode yolo).",
                is_default: true,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        // `~/.qwen/projects/<cwd-with-non-alnum-hyphenated>/chats/<uuid>.jsonl`.
        // ⚠ The service's own comment says `~/.qwen/tmp/<id>/chats` — that
        // comment is STALE; the code calls `getProjectDir()`, not
        // `getProjectTempDir()`. Read the code, not the comment.
        session_store_globs: &[".qwen/projects/*/chats/*.jsonl"],
        // `<sessionId>.runtime.json` sits beside the transcript in the same
        // directory and is not a transcript.
        store_excluded_name_fragments: &[".runtime."],
        durable_store_files: &[],
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_qwen_store_entry,
        store_membership_index: None,
        live_session_marker: None,
        read_live_store_title: Some(read_qwen_live_store_title),
        remote_live_store_title: Some(QWEN_REMOTE_TITLE_PROBE),
    },
    AgentCliDescriptor {
        kind: SessionKind::Kimi,
        display_name: "Kimi",
        session_metadata_label: "Kimi Session",
        slug: "kimi",
        binary_name: "kimi",
        // A Python CLI; `uv tool install` is what its own getting-started says.
        install: CliInstall::Uv("kimi-cli"),
        update: CliUpdate::Reinstall,
        icon_glyph: "K_",
        // Moonshot's deep blue (8.72:1).
        brand_color: "#1e40af",
        menu_hint: 'k',
        // `state.json` carries `custom_title` with a `title_generated` flag and
        // a 3-attempt cap — the CLI owns it.
        // ⛔ WAS `Store`, OVER A STORE THAT HOLDS NO TITLE — and the two halves of
        // yggterm already disagreed about that.
        //
        // `TitleAuthority::Store` makes `session_accepts_generated_copy` refuse
        // this kind a generated title, on the reasoning that inventing one would
        // disagree forever with the title the CLI wrote. That reasoning needs
        // the CLI to have written one.
        //
        // ⭐ It does not. `startpage::scan_kimi_sessions` — which locates this
        // store perfectly well, reversing its hashed bucket via the CLI's own
        // config — says so in its own comment and falls back to a generated or
        // heuristic title. Measured independently 2026-08-21 on a machine where
        // this CLI has been launched: no key anywhere in a session's files is a
        // title, a cwd or a session id.
        //
        // ⇒ So the SCAN path already treated this CLI as generating, while the
        // LIVE path honoured the declaration and refused to generate. One CLI,
        // two answers to "who names this row", and the live half's answer was
        // "nobody" — the row wore its birth title for the life of the session.
        //
        // ⚠ Its store being empty of titles is why `read_live_store_title` is
        // `None` here and why that is NOT the hole the store-authority lock
        // hunts: there is nothing to read. This flips back only if the CLI
        // starts writing a title, in the same commit as the reader for it.
        title_authority: TitleAuthority::Generated,
        // `kimi -r <unknown-id>` CREATES that session rather than failing, so a
        // caller-supplied id at birth is honoured. Its id is a directory name
        // verbatim, with no format validation.
        id_assigned_at_birth: true,
        wrapper_slug: Some("kimi"),
        remote_row_scheme: Some("remote-kimi://"),
        runtime_key_scheme: Some("kimi-runtime://"),
        // ⚠ Kimi's main turn spinner draws a moon frame with EMPTY text, and
        // its interrupt is Ctrl-C, not esc — there is no "esc to interrupt"
        // affordance to match. These are the per-block spinners.
        working_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "composing...",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "thinking...",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "compacting...",
                also_any: &[],
            },
        ],
        // `Thought for 12s` is the COMPLETION trace, and it contains `thought`,
        // not `thinking...` — kept explicit so a future needle widening cannot
        // silently swallow it.
        working_screen_negations: &["thought for "],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
        // ⭐ CONFIRMED 2026-08-08 against a real `kimi --help` on guihost (yggterm
        // provisioned it via uv the same day): `--session,--resume  -S,-r`. The
        // value was read from source at intake and is now MEASURED — recorded
        // because an agreeing measurement is still a measurement, and the next
        // reader should not have to re-run it to find that out.
        resume_selector: ResumeSelector::Flag("--resume"),
        // `kimi -w <dir>` is how a new session is rooted; resume takes the id
        // and re-derives the work dir from its own metadata.
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_marker: '\u{276f}',
        composer_footer_hints: &["ctrl", "kimi", "/help", "tab"],
        working_footer_hints: &["composing...", "thinking..."],
        // MEASURED from the same `--help`: `--yolo,--yes,--auto-approve  -y`
        // ("Automatically approve all actions"). kimi expresses no plan or
        // accept-edits posture, so neither is mapped onto something close.
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (AgentPermissionMode::Bypass, &["--yolo"]),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            ("--yolo", FlagArity::Standalone, OverriddenBy::PermissionMode),
            ("--yes", FlagArity::Standalone, OverriddenBy::PermissionMode),
            (
                "--auto-approve",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
            ("-y", FlagArity::Standalone, OverriddenBy::PermissionMode),
            ("--afk", FlagArity::Standalone, OverriddenBy::PermissionMode),
        ],
        extra_args_slug: "kimi",
        // ⭐ MEASURED 2026-08-13 on kimi 1.49.0, installed on all three fleet
        // hosts. The extra-args spec filed these as `documented` because the
        // binary was on no host on 2026-08-08 — it is now, so the row loses its
        // "not verified here" marker. ⛔ No sandbox flag exists; none is invented.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "",
                explanation: "Kimi's own default: every tool call is confirmed.",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "--yolo",
                explanation: "Auto-approves all tool calls; you are still reachable for a \
                              question the agent asks. Aliases: -y, --yes, --auto-approve.",
                is_default: true,
            },
            PermissionPreset {
                id: "afk",
                label: "Away from keyboard",
                args: "--afk",
                explanation: "Auto-approves AND auto-dismisses the agent's questions — nothing \
                              can stop to ask you.",
                is_default: false,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        // Resume replays only the last 5 turns to the screen; the full history
        // stays on disk, so the PTY is NOT a faithful re-derivation.
        content_rederives_on_resume: false,
        // ⛔⛔ THE STORE MOVED UNDER US (measured 2026-08-30, kimi 0.27.0):
        // kimi-code writes `~/.kimi-code/sessions/wd_<slug>_<hash>/session_<uuid>/`,
        // one directory per session with `state.json` (title, workDir,
        // createdAt/updatedAt, isCustomTitle) and `agents/main/wire.jsonl`.
        // Everything earlier in this descriptor described `~/.kimi/` — the
        // PREVIOUS kimi's home — which the installed CLI never touches, so
        // yggterm's kimi rows read a dead store.
        session_store_globs: &[".kimi-code/sessions/*/*/state.json"],
        store_excluded_name_fragments: &[],
        durable_store_files: &[],
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_kimi_code_store_entry,
        store_membership_index: None,
        live_session_marker: None,
        // The session's own state.json carries `title` (the first prompt,
        // `isCustomTitle` when the user renamed) — read it for live rows too.
        read_live_store_title: Some(read_kimi_live_store_title),
        remote_live_store_title: Some(KIMI_REMOTE_TITLE_PROBE),
    },
    AgentCliDescriptor {
        kind: SessionKind::Muse,
        display_name: "Muse Code",
        session_metadata_label: "Muse Code Session",
        slug: "muse",
        binary_name: "muse",
        // The vendor installer writes a launcher to ~/.local/bin/muse which
        // then fetches `muse-bin-<version>` beside it — user-local, which is
        // what `spec-cli-binary-auto-provisioning` requires. Credentials land
        // in ~/.config/muse/auth.json.
        install: CliInstall::VendorScript("https://dev.meta.ai/install.sh"),
        update: CliUpdate::Reinstall,
        icon_glyph: "M_",
        // Nearest available (8.24:1).
        brand_color: "#86198f",
        menu_hint: 'm',
        // Muse records no title of its own (like Codex); yggterm's LLM chore writes one.
        title_authority: TitleAuthority::Generated,
        id_assigned_at_birth: false,
        wrapper_slug: Some("muse"),
        remote_row_scheme: Some("remote-muse://"),
        runtime_key_scheme: Some("muse-runtime://"),
        // Measured from Muse Code TUI: shows "esc to interrupt", "esc to cancel", "working...", "thinking..."
        working_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "esc to interrupt",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "esc to cancel",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "working...",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "thinking...",
                also_any: &[],
            },
        ],
        working_screen_negations: &[],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
        // ⭐ MEASURED 2026-08-08 on guihost, from `muse resume --help` on a real
        // install: `muse resume` / `muse resume --last` / `muse resume
        // <session-uuid>`. ⛔ The placeholder here said `Flag("--resume")`,
        // guessed from the other CLIs — a resume built from it would have
        // handed `muse --resume <uuid>` to an arg parser that has no such flag,
        // and EVERY Muse resume would have failed. The click-to-resume handoff
        // is the product; a guessed resume selector breaks exactly that.
        resume_selector: ResumeSelector::Subcommand("resume"),
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_footer_hints: &["esc", "ctrl", "enter", "tab"],
        // ⭐ MEASURED 2026-08-22 from a real muse row: it draws U+27E9 `⟩`, not the
        // U+276F `❯` that seven of the ten descriptors carry. Declared as `❯`, the
        // readiness probe never found this CLI's composer at all, so every row of
        // it reported `consuming_input: false` forever and the delivery verb waited
        // out its whole timeout without ever sending — the SAME failure this field
        // was created for when a hardcoded `›` did it to Claude Code on 2026-08-06.
        // ⇒ The mechanism was made per-CLI; the value stayed a guess.
        composer_marker: '\u{27e9}',
        working_footer_hints: &["esc to interrupt", "esc to cancel"],
        // MEASURED from `muse --help` §Safety: approval and the sandbox are ON
        // by default, and `--yolo` is the one flag that turns both off. Muse
        // expresses no plan/accept-edits posture, so those are absent rather
        // than mapped onto something close.
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (AgentPermissionMode::Bypass, &["--yolo"]),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            ("--yolo", FlagArity::Standalone, OverriddenBy::PermissionMode),
            (
                "--approval-mode",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            (
                "--disable-approval",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
            (
                "--disable-sandbox",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
        ],
        extra_args_slug: "muse",
        // ⭐ MEASURED 2026-08-13 on Muse Code 0.1.0 (0.1.0-R708.1), installed on
        // all three fleet hosts. The extra-args spec filed muse as UNMEASURED
        // and owner-gated; the LOGIN is still his, but the flag surface is not
        // gated behind it and reads off `--help` like any other CLI. ⇒ the row
        // is no longer disabled.
        //
        // ⭐ Muse is the only CLI here whose bypass releases BOTH gates in one
        // flag — its `--yolo` turns off approval AND sandboxing AND trusts the
        // workspace. Antigravity's does not, which is why that row's explanation
        // has to warn about a prompt this one has no equivalent of.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "--approval-mode untrusted",
                explanation: "Every tool call is confirmed and the sandbox stays on.",
                is_default: false,
            },
            PermissionPreset {
                id: "sandboxed",
                label: "Sandboxed",
                args: "--approval-mode on-request",
                explanation: "Muse's own default: the model decides when to ask, and shell \
                              filesystem/network sandboxing stays on \
                              (--sandbox-network defaults to proxy-only).",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "--yolo",
                explanation: "Disables approval AND sandboxing and trusts this workspace for the \
                              run. --disable-approval and --disable-sandbox are the separate \
                              halves if you want only one of them.",
                is_default: true,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        // Muse stores sessions as `~/.local/share/muse/sessions/YYYY/MM/DD/<uuid>/session.jsonl`
        // (XDG_DATA_HOME) plus a SQLite index `~/.local/share/muse/session-index.db`.
        // Verified 2026-08-16 on a headless host: `session-index.db.sessions(workspace_root→cwd, title,
        // updated_at_us)` carries the cwd/title the cwd tree and startpage need, and
        // `route_facts.cwd` in the JSONL is the fallback when the DB is absent.
        session_store_globs: &[".local/share/muse/sessions/**/session.jsonl"],
        store_excluded_name_fragments: &["/subagent/", "/tool-outputs/"],
        durable_store_files: &[],
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_muse_store_entry,
        store_membership_index: Some(muse_store_index_holds_session),
        live_session_marker: Some(LiveSessionMarker::EnclosingDirectory {
            root: ".local/share/muse/sessions",
            file_name: ".session.lock",
        }),
        read_live_store_title: Some(read_muse_live_store_title),
        remote_live_store_title: Some(MUSE_REMOTE_TITLE_PROBE),
    },
    AgentCliDescriptor {
        kind: SessionKind::Antigravity,
        display_name: "Antigravity",
        session_metadata_label: "Antigravity Session",
        slug: "antigravity",
        binary_name: "agy",
        install: CliInstall::Manual,
        // ⭐ MEASURED on guihost 2026-08-08, `agy --help`: `update  Update CLI`.
        // yggterm cannot FETCH agy — a 166 MB self-contained binary served
        // behind a sign-in — but it must not therefore go stale, and the CLI
        // itself is the thing that knows how to replace it.
        update: CliUpdate::SelfCommand(&["update"]),
        icon_glyph: "A_",
        // Google's blue, darkened one step to clear AA (6.95:1).
        brand_color: "#1557b0",
        menu_hint: 'a',
        // Antigravity store writes summaries/previews into conversation_summaries.db.
        title_authority: TitleAuthority::Store,
        id_assigned_at_birth: false,
        wrapper_slug: Some("agy"),
        remote_row_scheme: Some("remote-agy://"),
        runtime_key_scheme: Some("agy-runtime://"),
        // Measured from agy TUI: shows "esc to cancel", "esc to interrupt", "generating...", "thinking...", "working..."
        working_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "esc to cancel",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "esc to interrupt",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "generating...",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "thinking...",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "working...",
                also_any: &[],
            },
        ],
        working_screen_negations: &[],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
        // Read off `agy --help`, v1.0.5 on guihost (2026-08-08): resume is
        // `--conversation <ID>`, and `-c`/`--continue` takes the most recent.
        resume_selector: ResumeSelector::Flag("--conversation"),
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_marker: '>',
        composer_footer_hints: &["shortcuts", "esc", "ctrl", "enter", "tab", "gemini", "?"],
        working_footer_hints: &[
            "esc to cancel",
            "esc to interrupt",
            "generating...",
            "thinking...",
            "working...",
        ],
        // `--dangerously-skip-permissions` is documented in `agy --help` as
        // "Auto-approve all tool permission requests without prompting", and
        // `--mode <accept-edits|plan>` was measured on the same help 2026-08-13
        // — it had been missed, so two postures agy can express were refused by
        // name on a CLI that supports them.
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (AgentPermissionMode::Plan, &["--mode", "plan"]),
            (AgentPermissionMode::AcceptEdits, &["--mode", "accept-edits"]),
            (
                AgentPermissionMode::Bypass,
                &["--dangerously-skip-permissions"],
            ),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            (
                "--dangerously-skip-permissions",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
            (
                "--mode",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            (
                "--sandbox",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
        ],
        extra_args_slug: "antigravity",
        // ⛔⛔ MEASURED 2026-08-13, BOTH ARMS IN ONE RUN, and the result is the
        // reason this explanation is worded the way it is: a row launched with
        // `--dangerously-skip-permissions` STILL stops on agy's workspace-TRUST
        // prompt ("Do you trust the contents of this project?") in a folder it
        // has not seen before. Default arm and bypass arm produced the identical
        // screen. ⇒ agy has TWO gates in series and this flag releases only one.
        //
        // ⭐ Where the other gate's answer lives, found by following it up:
        // agy's own settings file carries a `trustedWorkspaces` LIST OF PATHS
        // beside its `permissions` block. So the prompt is once-per-folder and
        // persistent, not a recurring block — which is why this is a fact about
        // agy rather than a defect in yggterm. ⛔ And it is why yggterm must not
        // "fix" it: writing another CLI's settings file on the user's behalf is
        // the same prohibition codex's `config.toml` already carries.
        // Muse's `--yolo` is the contrast — it trusts the workspace too.
        //
        // ⚠ A row parked on that prompt is, from the sidebar, indistinguishable
        // from a row whose CLI never launched. Do not read "it opened a plain
        // shell" as a launch failure without reading the screen first.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "",
                explanation: "Every tool permission request is prompted.",
                is_default: false,
            },
            PermissionPreset {
                id: "sandboxed",
                label: "Sandboxed",
                args: "--sandbox",
                explanation: "Runs with terminal restrictions enabled.",
                is_default: false,
            },
            PermissionPreset {
                id: "accept-edits",
                label: "Auto-edit",
                args: "--mode accept-edits",
                explanation: "File edits apply unprompted; --mode plan is read-only planning.",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "--dangerously-skip-permissions",
                explanation: "Auto-approves all tool permission requests. It does NOT answer the \
                              workspace-trust prompt a first run in a new folder shows: agy has \
                              no flag for that gate and keeps the answer in its own settings, so \
                              it is asked once per folder and never again.",
                is_default: true,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        // `~/.gemini/antigravity-cli/conversations/<uuid>.db`, with summaries in
        // `~/.gemini/antigravity-cli/conversation_summaries.db`.
        // Also supports brain transcript layout `~/.gemini/antigravity-cli/brain/<uuid>/.system_generated/logs/transcript.jsonl`
        // and legacy `~/.antigravitycli/*.json`.
        session_store_globs: &[
            ".gemini/antigravity-cli/conversations/*.db",
            // MEASURED 2026-08-20: every file on disk is `transcript_full.jsonl`.
            // The old `transcript.jsonl` spelling matched 0 of 497 brain dirs, so the
            // file half of the agy scan had been dead while reading as wired up.
            ".gemini/antigravity-cli/brain/*/.system_generated/logs/transcript_full.jsonl",
            ".antigravitycli/*.json",
        ],
        store_excluded_name_fragments: &["-shm", "-wal"],
        durable_store_files: &[],
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_antigravity_store_entry,
        store_membership_index: Some(antigravity_store_index_holds_session),
        // Measured 2026-08-20: at LAUNCH an agy process holds only the shared
        // index, so a fresh row has nothing to bind to — its conversation does
        // not exist yet. Once a turn has happened it holds
        // `presence/<conversation-id>.lock`, and the id is the file's own stem
        // rather than a directory name.
        live_session_marker: Some(LiveSessionMarker::FileStem {
            root: ".gemini/antigravity-cli/presence",
            extension: "lock",
        }),
        read_live_store_title: Some(read_antigravity_live_store_title),
        remote_live_store_title: Some(ANTIGRAVITY_REMOTE_TITLE_PROBE),
    },
    // ── The 2026-08-13 intake. Every field below was read off the installed
    // binary (`@xai-official/grok` 1.0.3 `1a29d5bc12`, provisioned into the
    // managed npm prefix on this fleet) or off strings in the shipped
    // executable, and the provenance is on the field. Nothing here came from a
    // vendor blog post.
    AgentCliDescriptor {
        kind: SessionKind::GrokBuild,
        // The product names itself "Grok Build TUI" on the first line of its own
        // `--help`; the npm package and the binary are both plain `grok`. The
        // SLUG follows `claude-code`/`qwen-code`: the product name, not the
        // binary, so the binary can be renamed by its vendor without the wire
        // name of a persisted row changing.
        display_name: "Grok Build",
        session_metadata_label: "Grok Build Session",
        slug: "grok-build",
        binary_name: "grok",
        // `bin: {"grok": "bin/grok"}`, Apache-2.0, with per-platform payloads in
        // optionalDependencies (linux/darwin/win32 × x64/arm64) — so one npm
        // name provisions correctly on every host the fleet has.
        install: CliInstall::Npm("@xai-official/grok"),
        // ⛔⛔ IT SHIPS AN UPDATER AND THAT UPDATER IS npm IN DISGUISE, SO IT IS
        // NOT PREFERRED HERE. `grok update` exists and the general rule would
        // pick it over re-running the install method — but MEASURED 2026-08-20,
        // `grok update --check --json` answers, of itself:
        //
        //     {"currentVersion":"1.0.5","latestVersion":"1.0.5",
        //      "updateAvailable":false,"installer":"npm", ...}
        //
        // `"installer":"npm"` is the CLI reporting that its own updater
        // DELEGATES BACK TO npm for an npm-provisioned copy. So preferring it
        // does not escape npm at all; it moves the npm invocation inside a
        // process where yggterm's prefix, staging directory, verification and
        // atomic publish do not apply. Worse, a session inherits
        // `npm_config_prefix` from the managed shell exports, so that inner
        // install would write the SHARED prefix and overwrite the published
        // per-CLI symlink with a plain npm bin link.
        //
        // ⇒ Reinstall through the provisioner's own npm path, which stages into
        // a fresh generation, proves the binary, and publishes atomically. The
        // vendor's real payload mechanism is unaffected: its postinstall still
        // installs `~/.grok/bin/grok-<version>` and swaps its own symlink.
        //
        // ⚠ This is NOT a reversal of the self-updater rule — that rule stands
        // and Antigravity still depends on it. It is the rule declining a
        // wrapper that would give up guarantees for nothing.
        update: CliUpdate::Reinstall,
        icon_glyph: "G_",
        // xAI's black. 21:1 against white text — the highest contrast in the
        // table, and the brand's actual colour rather than a nearest match.
        brand_color: "#000000",
        menu_hint: 'g',
        title_authority: TitleAuthority::Generated,
        // MEASURED: `-s, --session-id <SESSION_ID>` — "Use a specific session
        // UUID for a **new** conversation (must be a valid UUID and must not
        // already exist under the target session directory)". So the fact is
        // true; ⚠ a collision is an ERROR, exactly like qwen, so a caller must
        // mint a fresh uuid and never reuse a row id it has already spent.
        id_assigned_at_birth: true,
        wrapper_slug: Some("grok"),
        remote_row_scheme: Some("remote-grok://"),
        runtime_key_scheme: Some("grok-runtime://"),
        working_screen_phrases: &[
            ScreenWorkingPhrase {
                needle: "esc to cancel",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "esc to interrupt",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "thinking...",
                also_any: &[],
            },
            ScreenWorkingPhrase {
                needle: "working...",
                also_any: &[],
            },
        ],
        working_screen_negations: &[],
        limit_wait_screen_phrases: &[],
        question_picker_screen_phrases: &[],
        background_agent_hint_screen_phrases: &[],
        startup_gate_screen_phrases: &[],
        plan_limit_choice_screen_phrases: &[],
        // MEASURED: `-r, --resume [<SESSION_ID_OR_TITLE>]`.
        resume_selector: ResumeSelector::Flag("--resume"),
        // grok takes the process cwd (`--cwd <CWD>` exists but the launch
        // command already `cd`s), and `--resume` scopes title matching to the
        // current directory — so re-rooting on the command line would be a
        // second encoding of the cd we already did.
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        // ⭐ MEASURED off a LIVE ROW 2026-08-13, and it is why the field was
        // declared unknown first: neither `❯` nor `›` occurs anywhere in the
        // shipped executable's strings, so a static read said "not one of the
        // two this repo knows". The running TUI draws `❯` inside a box-drawn
        // composer. ⇒ a binary's strings can be silent about a glyph its own
        // renderer composes; the screen settles it and nothing else does.
        composer_marker: '\u{276f}',
        // MEASURED from the executable's own strings plus one live screen:
        // `/help for commands` is composer chrome, and the row's footer names
        // the model (`Grok 4`). ⚠ `esc cancel` is NOT listed here: it sits
        // adjacent to the same status line in the binary and could belong to
        // either the idle composer or the in-flight footer, and a hint in the
        // wrong half makes a working row read as a prompt.
        composer_footer_hints: &["/help for commands", "ctrl", "grok"],
        working_footer_hints: &[],
        // MEASURED on the installed binary: `--permission-mode <MODE>` with
        // `[possible values: default, acceptEdits, auto, dontAsk,
        // bypassPermissions, plan]`. ⚠ `auto` and `dontAsk` are NOT mapped —
        // yggterm has four postures and neither of those is one of them, and
        // inventing a correspondence for a security boundary is what the
        // refuse-by-name rule forbids.
        permission_modes: &[
            (AgentPermissionMode::Default, &[]),
            (AgentPermissionMode::Plan, &["--permission-mode", "plan"]),
            (
                AgentPermissionMode::AcceptEdits,
                &["--permission-mode", "acceptEdits"],
            ),
            (
                AgentPermissionMode::Bypass,
                &["--permission-mode", "bypassPermissions"],
            ),
        ],
        overridden_flags: &[
            ("--model", FlagArity::TakesValue, OverriddenBy::Model),
            ("-m", FlagArity::TakesValue, OverriddenBy::Model),
            (
                "--permission-mode",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
            (
                "--always-approve",
                FlagArity::Standalone,
                OverriddenBy::PermissionMode,
            ),
            (
                "--sandbox",
                FlagArity::TakesValue,
                OverriddenBy::PermissionMode,
            ),
        ],
        extra_args_slug: "grok-build",
        // ⭐ Grok Build's `--permission-mode` vocabulary is Claude Code's, token
        // for token. That is a real convenience and also a trap: `--allow` /
        // `--deny` carry "compat alias: --allowedTools / --disallowedTools", so
        // a user pasting a Claude line into this box gets something that mostly
        // works, which is the state in which a difference goes unnoticed.
        //
        // ⚠ `--sandbox <PROFILE>` TAKES A VALUE, so there is no runnable bare
        // sandbox tier to offer; the profile lives in `.grok/sandbox.toml` and
        // is named in prose instead of shipped as a half-written flag.
        permission_presets: &[
            PermissionPreset {
                id: "ask",
                label: "Ask each time",
                args: "--permission-mode default",
                explanation: "Every tool use is confirmed by you. --permission-mode plan is \
                              read-only planning.",
                is_default: false,
            },
            PermissionPreset {
                id: "accept-edits",
                label: "Auto-edit",
                args: "--permission-mode acceptEdits",
                explanation: "File edits apply without asking; commands still ask.",
                is_default: false,
            },
            PermissionPreset {
                id: "skip",
                label: "Skip checks",
                args: "--permission-mode bypassPermissions",
                explanation: "Auto-approves every tool execution (--always-approve is the same \
                              posture spelled as a flag). For a middle ground grok takes --allow \
                              and --deny rules, and a sandbox profile named with \
                              --sandbox <PROFILE>.",
                is_default: true,
            },
        ],
        permission_provenance: PermissionProvenance::Measured,
        content_rederives_on_resume: true,
        // ⭐ MEASURED 2026-08-13 against real sessions, and the gap this replaces
        // was WRONG about needing the owner: a fleet host was already signed in.
        // ⚠ `summary.json` alone, deliberately — the session directory also holds
        // `chat_history.jsonl`, `events.jsonl` and `updates.jsonl`, and globbing
        // those would yield three entries for one session.
        session_store_globs: &[".grok/sessions/*/*/summary.json"],
        // The `.lock` siblings are not matched by the glob, so nothing to exclude.
        store_excluded_name_fragments: &[],
        durable_store_files: &[],
        // grok reads `GROK_SANDBOX` for the sandbox profile; nothing in its help
        // or its strings relocates the HOME, which stays `~/.grok`.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_grok_build_store_entry,
        store_membership_index: None,
        live_session_marker: None,
        // Grok keeps summary.json per session directory; the chore reads the
        // session's own summary for live rows.
        read_live_store_title: Some(read_grok_live_store_title),
        remote_live_store_title: Some(GROK_REMOTE_TITLE_PROBE),
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

/// The Muse data directory owning a transcript (`.../muse` beside
/// `session-index.db`).  Derive it from the path being scanned instead of the
/// process HOME: fleet scans and fixtures can point at another user's store,
/// and consulting our own index silently cross-wires the two stores.
pub(crate) fn muse_data_root_from_session_path(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        (ancestor.file_name()?.to_str()? == "sessions")
            .then(|| ancestor.parent())
            .flatten()
            .filter(|parent| parent.file_name().and_then(|name| name.to_str()) == Some("muse"))
            .map(Path::to_path_buf)
    })
}

/// Whether the transcript itself proves that Muse accepted a user intent.
/// This is stronger than `session-index.db.prompt_count`: live evidence shows
/// that counter can remain zero after hundreds of records and real prompts.
pub(crate) fn muse_session_contains_accepted_user_intent(path: &Path) -> bool {
    use std::io::{BufRead, BufReader};
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    BufReader::new(file).lines().flatten().any(|line| {
        serde_json::from_str::<serde_json::Value>(&line)
            .ok()
            .and_then(|value| {
                value
                    .get("payload_type")
                    .and_then(|payload_type| payload_type.as_str())
                    .map(|payload_type| payload_type == "runtime.user_intent.accepted")
            })
            .unwrap_or(false)
    })
}

fn muse_title_from_session_jsonl(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    // Do not cap this at the startup prelude.  A measured Muse transcript had
    // its first real user intent at record 688 while the index still claimed
    // zero prompts; the old 64-line ceiling made the durable row untitled.
    for line in reader.lines().flatten() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            // Muse's user prompt lives under payload.model_messages[*].content[*].text
            // for payload_type runtime.user_intent.accepted / materialized. Do not
            // stop at the first accepted envelope: a launch-purpose envelope can
            // be low-signal while the next accepted envelope is the real task.
            let pt = value.get("payload_type").and_then(|v| v.as_str()).unwrap_or("");
            if pt == "runtime.user_intent.accepted" || pt == "runtime.user_intent.materialized" {
                let model_texts = value
                    .get("payload")
                    .and_then(|p| p.get("model_messages"))
                    .and_then(|m| m.as_array())
                    .into_iter()
                    .flat_map(|messages| messages.iter())
                    .filter_map(|message| message.get("content").and_then(|c| c.as_array()))
                    .flat_map(|content| content.iter())
                    .filter_map(|content| content.get("text").and_then(|text| text.as_str()));
                for text in model_texts {
                    if let Some(title) = usable_muse_prompt_title(text) {
                        return Some(title);
                    }
                }
                // Fallback inside refill_blocks
                let refill_texts = value
                    .get("payload")
                    .and_then(|p| p.get("refill_blocks"))
                    .and_then(|r| r.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|block| block.get("text").and_then(|text| text.as_str()));
                for text in refill_texts {
                    if let Some(title) = usable_muse_prompt_title(text) {
                        return Some(title);
                    }
                }
            }
        }
    }
    None
}

fn usable_muse_prompt_title(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    let condensed = crate::best_effort_title_from_context(trimmed).or_else(|| {
        let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();
        if !first_line.is_empty() && first_line.len() <= 120 {
            Some(first_line.to_string())
        } else {
            Some(trimmed.chars().take(80).collect())
        }
    })?;
    (!crate::looks_like_generated_fallback_title(&condensed)
        && !crate::looks_like_low_signal_generated_copy(&condensed))
        .then_some(condensed)
}

/// Codex keeps no title in its own transcript — the generated-copy store
/// answers for those, which is why `title` is always `None` here and the
/// scanner layers a resolver on top.
fn read_codex_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let (session_id, cwd) = crate::read_codex_session_identity_fields(path)
        .ok()
        .flatten()?;
    let db_title = dirs::home_dir().and_then(|home| {
        let db_path = home.join(".yggterm/session-titles.db");
        if db_path.exists() {
            let conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_URI
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            ).ok()?;
            let mut stmt = conn.prepare("SELECT title FROM session_titles WHERE session_id = ?1 LIMIT 1").ok()?;
            let mut rows = stmt.query(rusqlite::params![session_id]).ok()?;
            let row = rows.next().ok()??;
            let title: Option<String> = row.get(0).ok();
            title.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        } else {
            None
        }
    });
    // ⛔ ONE tail read for the whole entry. `extract_tail_context` seeks a
    // multi-megabyte window to the end of the transcript and `serde_json`-parses
    // every line in it; this used to happen TWICE per file per scan — once for
    // the title fallback and unconditionally again for the detail — over a
    // corpus of hundreds of files, on an 8 s poll.
    let tail_context = crate::titles::extract_tail_context(path).ok();
    let title = db_title
        .filter(|t| !crate::looks_like_generated_fallback_title(t) && !crate::looks_like_low_signal_generated_copy(t))
        .or_else(|| {
            tail_context
                .as_deref()
                .and_then(crate::titles::heuristic_title_from_context)
                .filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
                .filter(|s| !s.contains("/home/"))
        });
    let detail = tail_context
        .filter(|context| !context.trim().is_empty())
        .filter(|c| !crate::looks_like_low_signal_generated_copy(c) && !crate::looks_like_generated_fallback_title(c))
        .filter(|c| !c.contains("/home/.yggterm/clipboard"));
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title,
        detail,
    })
}

/// The reader for a CLI whose store yggterm cannot enumerate yet.
///
/// It is wired to a descriptor whose `session_store_globs` is EMPTY, so nothing
/// ever calls it — but a descriptor field may not be left unset, and a function
/// that says why in one line beats a `None` that says nothing. The reason lives
/// on the descriptor's `store_scan_gap`.
fn read_no_store_entry(_path: &Path) -> Option<AgentStoreEntry> {
    None
}

/// Read the FIRST line of a JSONL file as JSON.
///
/// Bounded: a session transcript's first line is its header for every CLI that
/// writes one, and a store scan must never pull a multi-megabyte transcript into
/// memory to learn a uuid.
fn read_first_jsonl_object(path: &Path) -> Option<serde_json::Value> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    let mut reader = BufReader::new(file);
    // A header line longer than this is not a header.
    for _ in 0..8 {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line.trim().is_empty() {
            continue;
        }
        return serde_json::from_str(line.trim()).ok();
    }
    None
}

/// `pi` — line 1 is the session header: `{"type":"session","id":…,"cwd":…}`.
///
/// The title is NOT in the header: it arrives later as a `session_info` entry
/// carrying `name`. Scanning the whole file for the newest one would make the
/// cwd-tree scan O(transcript), so the store contributes identity only and the
/// title chore owns the copy — which is what `TitleAuthority::Generated` says.
fn read_pi_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let header = read_first_jsonl_object(path)?;
    if header.get("type").and_then(|value| value.as_str()) != Some("session") {
        return None;
    }
    let session_id = header.get("id")?.as_str()?.to_string();
    let cwd = header.get("cwd")?.as_str()?.to_string();
    let db_title = dirs::home_dir().and_then(|home| {
        let db_path = home.join(".yggterm/session-titles.db");
        if db_path.exists() {
            let conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_URI
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            ).ok()?;
            let mut stmt = conn.prepare("SELECT title FROM session_titles WHERE session_id = ?1 LIMIT 1").ok()?;
            let mut rows = stmt.query(rusqlite::params![session_id]).ok()?;
            let row = rows.next().ok()??;
            let title: Option<String> = row.get(0).ok();
            title.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        } else {
            None
        }
    });
    // ⛔ ONE tail read for the whole entry. `extract_tail_context` seeks a
    // multi-megabyte window to the end of the transcript and `serde_json`-parses
    // every line in it; this used to happen TWICE per file per scan — once for
    // the title fallback and unconditionally again for the detail — over a
    // corpus of hundreds of files, on an 8 s poll.
    let tail_context = crate::titles::extract_tail_context(path).ok();
    let title = db_title
        .filter(|t| !crate::looks_like_generated_fallback_title(t) && !crate::looks_like_low_signal_generated_copy(t))
        .or_else(|| {
            tail_context
                .as_deref()
                .and_then(crate::titles::heuristic_title_from_context)
                .filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
                .filter(|s| !s.contains("/home/"))
        });
    let detail = tail_context
        .filter(|context| !context.trim().is_empty())
        .filter(|c| !crate::looks_like_low_signal_generated_copy(c) && !crate::looks_like_generated_fallback_title(c));
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title,
        detail,
    })
}

/// `qwen` — every record carries `sessionId` and `cwd`, so the first one
/// answers identity without walking the file.
fn read_qwen_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let first = read_first_jsonl_object(path)?;
    let session_id = first.get("sessionId")?.as_str()?.to_string();
    let cwd = first.get("cwd")?.as_str()?.to_string();
    if session_id.is_empty() || cwd.is_empty() {
        return None;
    }
    // Qwen persists `custom_title` as a record appended later and re-appended near EOF.
    // The identity scan must surface it so Store authority respects it directly
    // (reference: codex's Generated vs claude's Store). Tail is bounded: last 64k.
    let title = read_qwen_custom_title_tail(path);
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title: title.clone(),
        detail: title,
    })
}

/// The LIVE half of Qwen's title: find this session's own chat file under the
/// agent store home, then read the title out of it with the same tail parser
/// the identity scan uses.
///
/// ⛔ **This existed as a hole, not as an absence.** `read_live_store_title:
/// None` is documented to mean UNMEASURED — this CLI's store layout has never
/// been read off a real machine, so nothing is guessed at. But Qwen's layout
/// HAS been read: `read_qwen_store_entry` parses its `sessionId` and `cwd`, and
/// `read_qwen_custom_title_tail` knows the title is a `custom_title` record
/// re-appended near EOF. So `None` was claiming unmeasured about a store two
/// functions above it already decode, while the descriptor ALSO declared
/// `TitleAuthority::Store` — which makes yggterm refuse to generate a title.
/// A live Qwen row was therefore refused a generated title and offered no
/// stored one: titled by nothing, for the life of the session.
///
/// ⚖ Only the LOOKUP is new here. The format is the existing parsers', and it
/// is not re-derived — the fixture test exercises finding the right file for an
/// id, which is the only part this function decides.
fn read_qwen_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    let descriptor = agent_cli_descriptor(SessionKind::QwenCode)?;
    for root in descriptor.store_roots_absolute(home) {
        // `<root>/<project>/chats/<file>.jsonl`. The project directory encodes a
        // cwd, so it cannot be derived from the id and every project is walked.
        let Ok(projects) = std::fs::read_dir(&root) else {
            continue;
        };
        for project in projects.flatten() {
            let chats = project.path().join("chats");
            let Ok(files) = std::fs::read_dir(&chats) else {
                continue;
            };
            // ⭐ Cheapest discriminator first: a file NAMED for the session.
            // Falling through to parsing is not an optimisation detail — it is
            // the correctness half, because unlike Claude Code's store the
            // filename here is not contractually the id, and a lookup that
            // assumed it were would answer nothing for a store that names its
            // files any other way.
            let mut fallback = Vec::new();
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                    continue;
                }
                if path.file_stem().and_then(|value| value.to_str()) == Some(session_id) {
                    return read_qwen_custom_title_tail(&path);
                }
                fallback.push(path);
            }
            for path in fallback {
                let Some(first) = read_first_jsonl_object(&path) else {
                    continue;
                };
                if first.get("sessionId").and_then(|value| value.as_str()) == Some(session_id) {
                    return read_qwen_custom_title_tail(&path);
                }
            }
        }
    }
    None
}

fn read_qwen_custom_title_tail(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    let file_len = meta.len();
    // Tail 64k — enough for final custom_title record without scanning multi-MB file.
    let tail_start = file_len.saturating_sub(64 * 1024);
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(tail_start)).ok()?;
    let mut last_title: Option<String> = None;
    for line in reader.lines().flatten() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value.get("type").and_then(|v| v.as_str()) == Some("custom_title") {
                if let Some(t) = value.get("title").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
                    // Qwen re-appends same title near EOF; last wins.
                    last_title = Some(t.to_string());
                } else if let Some(t) = value.get("customTitle").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
                    last_title = Some(t.to_string());
                }
            }
        }
    }
    last_title
        .filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
}

/// `agy` — one flat JSON object per conversation.
///
/// `name` is the CLI's own title, and on a conversation that has not been named
/// yet it is still the working directory. Handing that back as a title would
/// make every fresh row read as the home directory, so a name equal to the cwd is treated
/// as absent — the same judgement the cwd-derived placeholder gets elsewhere.
/// Grok Build files one DIRECTORY per session, bucketed by working directory:
/// `~/.grok/sessions/<percent-encoded-cwd>/<session-uuid>/summary.json`, beside
/// `chat_history.jsonl`, `events.jsonl` and a `system_prompt.txt`.
///
/// ⭐ The bucket name is the cwd PERCENT-ENCODED, so unlike kimi's MD5-of-cwd
/// buckets it is reversible — but nothing here decodes it, because
/// `summary.json` carries `info.cwd` as a plain absolute path and `info.id` as
/// the session uuid. **Read the file, not the path**: the directory name is a
/// second encoding of a value the file already states, and the two could only
/// ever disagree.
///
/// Measured 2026-08-13 against real sessions on a signed-in host: `info.id` is a
/// 36-char uuid that equals its own directory name, `info.cwd` is absolute, and
/// `updated_at` is RFC-3339 with nanoseconds. The glob targets `summary.json`
/// alone so one session yields exactly one entry.
fn read_grok_build_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let info = value.get("info")?;
    let session_id = info.get("id")?.as_str()?.trim().to_string();
    let cwd = info.get("cwd")?.as_str()?.trim().to_string();
    if session_id.is_empty() || cwd.is_empty() {
        return None;
    }
    // ⭐ A SUMMARY IS NOT A TITLE, AND grok WRITES BOTH. Its own binary
    // documents them as a pair — "`session_summary` and `generated_title` —
    // the session summary and its model-generated title" — so the title is
    // preferred here and the summary is the fallback. Reading only the summary
    // (as this did) would name a row with a paragraph when grok had a title
    // for it.
    //
    // ⚠ NEITHER field is a placeholder, and neither is guaranteed present.
    // Grok generates them asynchronously and carries a log line for exactly
    // the case observed here — "session closed before its title was
    // generated". Both sessions measured (2026-08-14, a signed-in fleet host)
    // ran two turns, fired no `session_summary_generated` event, and their
    // `summary.json` had an empty `session_summary` and no `generated_title`
    // key at all.
    //
    // ⇒ That is why `title_authority` stays `Generated`: not because the CLI
    // never fills these, but because it often has not filled them YET, and a
    // `Store` authority would leave those rows nameless. When grok does write
    // one, it is used.
    let title = ["generated_title", "session_summary"]
        .into_iter()
        .find_map(|field| {
            value
                .get(field)
                .or_else(|| info.get(field))
                .and_then(|found| found.as_str())
                .map(str::trim)
                .filter(|found| !found.is_empty())
                .map(ToOwned::to_owned)
        });
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title,
        detail: None,
    })
}

/// The first `limit` CHARACTERS of `text`, trimmed.
///
/// ⚖ Characters, not bytes: a title is displayed, so its length is what a
/// reader counts, and a byte slice through the middle of a character is a
/// panic rather than a short title.
fn first_chars(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((at, _)) => text[..at].trim().to_string(),
        None => text.trim().to_string(),
    }
}

pub fn clean_agy_prompt_first_line(raw: &str) -> Option<String> {
    let mut text = raw.trim();
    if let Some(idx) = text.find("<USER_REQUEST>") {
        let after = &text[idx + "<USER_REQUEST>".len()..];
        text = after.split("</USER_REQUEST>").next().unwrap_or(after).trim();
    }
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty()
            || l.starts_with("```")
            || l.starts_with('#')
            || l.starts_with("<ADDITIONAL_METADATA>")
            || l.starts_with("<USER_SETTINGS_CHANGE>")
            || l.starts_with("<CONTEXT_SUMMARY>")
            || l.starts_with("<SYSTEM_MESSAGE>")
            || l.starts_with("{{ CHECKPOINT")
            || l.starts_with("<USER_REQUEST>")
        {
            continue;
        }
        if !crate::looks_like_generated_fallback_title(l) {
            // ⛔ `l[..120]` PANICS when byte 120 lands inside a character, and
            // this line is a person's own prompt — the one string in the title
            // path most likely to carry an accent, a dash or an emoji. It
            // survived only because the sources reaching here were short,
            // curated index entries; the remote probe now also carries raw
            // transcript prompts, which are long free-form text, so the latent
            // case became a reachable one. Count CHARACTERS.
            return Some(first_chars(l, 120));
        }
    }
    None
}

/// [`AgentCliDescriptor::read_live_store_title`] for Claude Code: CC names each
/// transcript `<session-id>.jsonl`, so the id finds the file and the file
/// carries the title CC wrote (including a mid-session `/rename`).
fn read_claude_code_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    let projects = agent_cli_descriptor(SessionKind::ClaudeCode)?
        .store_roots_absolute(home)
        .into_iter()
        .next()?;
    let jsonl = crate::local_cc_session_jsonl_path_in(&projects, session_id)?;
    crate::read_cc_session_title(&jsonl).ok().flatten()
}

/// [`AgentCliDescriptor::read_live_store_title`] for Antigravity, which keeps
/// the answer in three places and none of them is the session file: the shared
/// `conversation_summaries.db`, then `history.jsonl`, then the conversation's
/// own brain transcript.
fn read_antigravity_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    crate::read_antigravity_session_title(home, session_id)
        .ok()
        .flatten()
}

/// [`AgentCliDescriptor::read_live_store_title`] for Muse.
/// Looks in `~/.local/share/muse/session-index.db` (sessions table: workspace_root, title, updated_at_us)
/// and falls back to `~/.local/share/muse/sessions/**/<session_id>/session.jsonl`.
/// The cached yggterm-side title for a session id (`~/.yggterm/session-titles.db`
/// — the LLM/heuristic chore's own output), filtered the way every reader must
/// filter it: a poisoned or placeholder-shaped cache row is NOT a title.
fn cached_session_title(session_id: &str) -> Option<String> {
    let db_path = dirs::home_dir()?.join(".yggterm/session-titles.db");
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let mut stmt = conn
        .prepare("SELECT title FROM session_titles WHERE session_id = ?1 LIMIT 1")
        .ok()?;
    let mut rows = stmt.query(rusqlite::params![session_id]).ok()?;
    let row = rows.next().ok()??;
    let title: Option<String> = row.get(0).ok();
    title
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| !crate::looks_like_generated_fallback_title(s))
        .filter(|s| !crate::looks_like_low_signal_generated_copy(s))
}

/// The FIRST REAL USER PROMPT in a codex rollout — the title codex sessions
/// deserve (owner spec 2026-06-06: yggterm owns codex titles; the transcript's
/// own opening prompt is the honest one).
///
/// ⛔ The rollout's first `role:"user"` item is NOT the prompt: codex writes
/// the AGENTS.md/instructions block and environment context as user messages
/// first (measured 2026-08-30 — a rollout whose first user item is the whole
/// fleet steer file). Skip those wrappers; take the first user text that is
/// neither, clean it to one line, and refuse fallback/low-signal shapes.
fn codex_first_real_user_prompt(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for (index, line) in reader.lines().enumerate() {
        // The real prompt is near the top; a bound keeps this cheap no matter
        // how long the rollout grows.
        if index > 400 {
            return None;
        }
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("response_item") {
            continue;
        }
        let payload = value.get("payload")?;
        if payload.get("role").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        let text = payload
            .get("content")
            .and_then(|content| content.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("text"))
                    .filter_map(|text| text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        let Some(text) = text else { continue };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        let is_wrapper = lower.starts_with("# agents.md")
            || lower.contains("<user_instructions>")
            || lower.contains("<environment_context>")
            || lower.contains("<permissions")
            || lower.contains("<turn_context>")
            || lower.starts_with("<instructions>");
        if is_wrapper {
            continue;
        }
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with('<')
                || line.starts_with("```")
            {
                continue;
            }
            let candidate: String = {
                let mut end = line
                    .char_indices()
                    .nth(120)
                    .map(|(offset, _)| offset)
                    .unwrap_or(line.len());
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                line[..end].trim_end().to_string()
            };
            if candidate.is_empty()
                || crate::looks_like_generated_fallback_title(&candidate)
                || crate::looks_like_low_signal_generated_copy(&candidate)
            {
                continue;
            }
            return Some(candidate);
        }
    }
    None
}

/// Find one file under `root` (bounded depth) whose FILE NAME ends with
/// `suffix`. Read-dir only — no file is opened to match.
fn find_file_by_suffix(root: &Path, depth: u8, suffix: &str) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    let mut directories = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            directories.push(path);
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(suffix))
        {
            return Some(path);
        }
    }
    // Newest-first would be nicer, but correctness first: any match IS the
    // session's own file — ids are unique per CLI store.
    for directory in directories {
        if let Some(found) = find_file_by_suffix(&directory, depth - 1, suffix) {
            return Some(found);
        }
    }
    None
}

/// [`AgentCliDescriptor::read_live_store_title`] for Codex and Codex-LiteLLM:
/// the cached yggterm title (LLM chore output, filtered), else the rollout's
/// first real user prompt. This is what lets the 12-second title chore serve
/// LIVE codex rows — before it they kept their birth names for the whole
/// session, because no reader existed to ask the transcript.
fn read_codex_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    if let Some(title) = cached_session_title(session_id) {
        return Some(title);
    }
    let descriptor = agent_cli_descriptor(SessionKind::Codex)?;
    let sessions_root = descriptor
        .store_roots_absolute(home)
        .into_iter()
        .next()?;
    let rollout = find_file_by_suffix(&sessions_root, 4, &format!("-{session_id}.jsonl"))?;
    title_without_fallbacks(codex_first_real_user_prompt(&rollout))
}

/// [`AgentCliDescriptor::read_live_store_title`] for OpenCode: the v2 store's
/// own title column (opencode2 self-titles every prompted session — measured
/// 2026-08-30), v1 table as a legacy tail.
fn read_opencode_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    let db_path = home.join(".local/share/opencode/opencode.db");
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    for table in ["session_v2", "session"] {
        let Ok(mut stmt) = conn.prepare(&format!(
            "SELECT title FROM {table} WHERE id = ?1 LIMIT 1"
        )) else {
            continue;
        };
        let mut rows = stmt.query(rusqlite::params![session_id]).ok()?;
        if let Ok(Some(row)) = rows.next() {
            let title: Option<String> = row.get(0).ok();
            if let Some(title) = title
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .filter(|s| !crate::looks_like_generated_fallback_title(s))
                .filter(|s| !crate::looks_like_low_signal_generated_copy(s))
            {
                return Some(title);
            }
        }
    }
    None
}

/// [`AgentCliDescriptor::read_live_store_title`] for Pi: the session jsonl's
/// own store entry (header id == file name uuid — measured 2026-08-30), whose
/// title is already extracted and filtered by the scan reader.
fn read_pi_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    let descriptor = agent_cli_descriptor(SessionKind::Pi)?;
    let sessions_root = descriptor
        .store_roots_absolute(home)
        .into_iter()
        .next()?;
    let session_file = find_file_by_suffix(&sessions_root, 2, &format!("{session_id}.jsonl"))?;
    let entry = read_pi_store_entry(&session_file)?;
    title_without_fallbacks(entry.title)
}

/// [`AgentCliDescriptor::read_live_store_title`] for Grok Build: the session
/// directory's own store entry (`summary.json`'s session summary; grok keeps
/// one directory per session under its URL-encoded cwd roots).
fn read_grok_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    let descriptor = agent_cli_descriptor(SessionKind::GrokBuild)?;
    let sessions_root = descriptor
        .store_roots_absolute(home)
        .into_iter()
        .next()?;
    // `.grok/sessions/<encoded-cwd>/<id>/summary.json` — the directory named
    // for the session sits one level below the glob root.
    let session_dir = find_dir_by_name(&sessions_root, 2, session_id)?;
    let entry = read_grok_build_store_entry(&session_dir.join("summary.json"))?;
    title_without_fallbacks(entry.title)
}

/// A candidate title that has survived every fallback/placeholder shape check.
/// One filter chain, so a reader cannot forget one arm.
fn title_without_fallbacks(title: Option<String>) -> Option<String> {
    title
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| !crate::looks_like_generated_fallback_title(s))
        .filter(|s| !crate::looks_like_low_signal_generated_copy(s))
}

/// Depth-bounded directory search: the directory whose NAME is exactly
/// `name`. Read-dir only — nothing is opened to match.
fn find_dir_by_name(root: &Path, depth: u8, name: &str) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    let mut directories = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name().to_str() == Some(name) {
                return Some(path);
            }
            directories.push(path);
        }
    }
    for directory in directories {
        if let Some(found) = find_dir_by_name(&directory, depth - 1, name) {
            return Some(found);
        }
    }
    None
}

/// [`read_store_entry`] for Kimi Code 0.27+: the session directory's
/// `state.json` carries everything — `title` (the first prompt;
/// `isCustomTitle` when renamed), `workDir`, `createdAt`/`updatedAt` — and
/// the session id is the grandparent directory's own name (`session_<uuid>`).
fn read_kimi_code_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let session_dir = path.parent()?.file_name()?.to_str()?.to_string();
    if session_dir.is_empty() {
        return None;
    }
    let cwd = value
        .get("workDir")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // Recency from the state.json FILE: kimi rewrites it every turn, so the
    // mtime is the session's own clock without a second RFC3339 parse.
    Some(AgentStoreEntry {
        session_id: session_dir,
        cwd: if cwd.is_empty() {
            dirs::home_dir()?.display().to_string()
        } else {
            cwd
        },
        modified_epoch_ms: modified_epoch_ms_of(path),
        title: title_without_fallbacks(title),
        detail: None,
    })
}

/// [`AgentCliDescriptor::read_live_store_title`] for Kimi: the session
/// directory's `state.json` `title` (the first prompt; `isCustomTitle` when
/// the user renamed). Measured 2026-08-30 against kimi 0.27.0's real store.
fn read_kimi_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    let sessions_root = home.join(".kimi-code/sessions");
    if !sessions_root.exists() {
        return None;
    }
    let session_dir = find_dir_by_name(&sessions_root, 2, session_id)?;
    let entry = read_kimi_code_store_entry(&session_dir.join("state.json"))?;
    title_without_fallbacks(entry.title)
}

fn read_muse_live_store_title(home: &Path, session_id: &str) -> Option<String> {
    let db_path = home.join(".local/share/muse/session-index.db");
    if db_path.exists() {
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT title, workspace_root FROM sessions WHERE session_id=?1",
            ) {
                if let Ok(mut rows) = stmt.query(rusqlite::params![session_id]) {
                    if let Ok(Some(row)) = rows.next() {
                        let title: Option<String> = row.get(0).ok();
                        let ws: Option<String> = row.get(1).ok();
                        let ws = ws.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                        let title = title
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty() && s != session_id)
                            .filter(|s| !crate::looks_like_generated_fallback_title(s))
                            .filter(|s| !crate::looks_like_low_signal_generated_copy(s));
                        let title = match (&ws, &title) {
                            (Some(ws), Some(t)) if t == ws => None,
                            _ => title,
                        };
                        if let Some(t) = title {
                            return Some(t);
                        }
                    }
                }
            }
        }
    }

    let sessions_root = home.join(".local/share/muse/sessions");
    if sessions_root.exists() {
        if let Some(jsonl_path) = find_muse_session_jsonl_in(&sessions_root, session_id, 0) {
            if let Some(t) = muse_title_from_session_jsonl(&jsonl_path) {
                if !crate::looks_like_generated_fallback_title(&t)
                    && !crate::looks_like_low_signal_generated_copy(&t)
                {
                    return Some(t);
                }
            }
        }
    }
    None
}

fn find_muse_session_jsonl_in(dir: &Path, session_id: &str, depth: usize) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some(session_id) {
                let jsonl = path.join("session.jsonl");
                if jsonl.exists() {
                    return Some(jsonl);
                }
            }
            if let Some(found) = find_muse_session_jsonl_in(&path, session_id, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

/// The ssh probe for Claude Code — the shape every other CLI's probe follows.
///
/// Head (512 KB) catches the early `ai-title`; the tail window catches a late
/// `custom-title`, because a `/rename` appends at the END of a large transcript,
/// past any head cap. Candidates come back newest-first with custom before ai,
/// which is the precedence Claude Code itself displays.
const CLAUDE_CODE_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: CLAUDE_CODE_REMOTE_TITLE_SCRIPT,
    locators: RemoteStoreLocators::StoreGlobs,
    choose: first_non_empty_candidate,
};

const CLAUDE_CODE_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sys
from pathlib import Path
HEAD_BYTES = 512 * 1024
TAIL_BYTES = 128 * 1024

def titles_from_lines(lines):
    custom = None
    ai = None
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except Exception:
            continue
        kind = record.get('type', '')
        if kind == 'custom-title':
            value = (record.get('customTitle') or '').strip()
            if value:
                custom = value
        elif kind == 'ai-title':
            value = (record.get('aiTitle') or '').strip()
            if value:
                ai = value
    return custom, ai

# argv: <store glob>... -- <session id>...  The separator is load-bearing; a CLI
# may declare more than one store glob and a glob must never be read as an id.
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
split = argv.index('--')
globs = [value for value in argv[:split] if value.strip()]
ids = [value for value in argv[split + 1:] if value.strip()]
if not globs or not ids:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
# The glob names FILES; a session's transcript is `<id>.jsonl` under it, so match
# on the stem rather than re-deriving the CLI's project-directory encoding.
by_id = {}
for glob in globs:
    for candidate in home.glob(glob):
        by_id.setdefault(candidate.stem, candidate)
for session_id in ids:
    found = by_id.get(session_id)
    if found is None:
        continue
    try:
        size = found.stat().st_size
        with open(found, encoding='utf-8', errors='ignore') as handle:
            head = handle.read(HEAD_BYTES).splitlines()
        tail = []
        if size > HEAD_BYTES:
            with open(found, 'rb') as handle:
                handle.seek(max(0, size - TAIL_BYTES))
                raw = handle.read().decode('utf-8', errors='ignore')
            # First chunk is likely a partial line; drop it.
            tail = raw.splitlines()[1:]
    except Exception:
        continue
    custom_head, ai_head = titles_from_lines(head)
    custom_tail, ai_tail = titles_from_lines(tail)
    candidates = [value for value in (custom_tail, custom_head, ai_tail, ai_head) if value]
    if candidates:
        print(json.dumps({'session_id': session_id, 'candidates': candidates},
                         ensure_ascii=False))
"#;

/// The ssh probe for Antigravity — the CLI the filed defect was reported
/// against.
///
/// ⚠ **Its title is not in its session file**, which is why the local reader
/// needed a bespoke arm and why this probe reads an INDEX rather than a
/// transcript. The order below mirrors that reader exactly: the summaries index
/// first (title, then preview), then the CLI's own history log.
///
/// ⚠ The third local fallback — parsing the conversation's brain transcript —
/// is deliberately NOT in the remote probe. It is a multi-megabyte read per
/// session on a machine we reached over ssh, and the two index reads above it
/// answer for every conversation that has had a turn. A remote row that only
/// the transcript could name reports `no_title_in_store`, which is true and
/// cheap; guessing it would have cost a store walk per tick.
/// The codex/codex-litellm remote title script: find the session's rollout by
/// file-name suffix and answer the FIRST REAL USER PROMPT — the same skip the
/// local reader does (`codex_first_real_user_prompt`): the rollout's first
/// user item is the AGENTS.md/instructions wrapper, never the prompt.
const CODEX_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sys
from pathlib import Path
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
split = argv.index('--')
globs = [v for v in argv[:split] if v.strip()]
ids = [v for v in argv[split + 1:] if v.strip()]
if not ids:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
WRAPPER_MARKS = ('<user_instructions>', '<environment_context>', '<permissions', '<turn_context>')

def first_real_prompt(path):
    try:
        with open(path, 'r', encoding='utf-8', errors='ignore') as fh:
            for i, line in enumerate(fh):
                if i > 400:
                    return None
                try:
                    v = json.loads(line)
                except Exception:
                    continue
                if v.get('type') != 'response_item':
                    continue
                p = v.get('payload') or {}
                if p.get('role') != 'user':
                    continue
                c = p.get('content')
                if not isinstance(c, list):
                    continue
                text = '\n'.join(x.get('text', '') for x in c if isinstance(x, dict))
                t = text.strip()
                if not t:
                    continue
                low = t.lower()
                if low.startswith('# agents.md') or low.startswith('<instructions>'):
                    continue
                if any(m in low for m in WRAPPER_MARKS):
                    continue
                for ln in text.splitlines():
                    ln = ln.strip()
                    if not ln or ln.startswith(('#', '<', '`')):
                        continue
                    return ln[:120]
    except Exception:
        return None
    return None

for sid in ids:
    found = None
    for g in globs:
        try:
            matches = home.glob(g)
        except Exception:
            continue
        for p in matches:
            if p.is_file() and p.name.endswith('-' + sid + '.jsonl'):
                t = first_real_prompt(p)
                if t:
                    found = t
                    break
        if found:
            break
    if found:
        print(json.dumps({'session_id': sid, 'candidates': [found]}, ensure_ascii=False))
"#;

const CODEX_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: CODEX_REMOTE_TITLE_SCRIPT,
    locators: RemoteStoreLocators::StoreGlobs,
    choose: first_non_empty_candidate,
};

/// Pi's remote twin: the session jsonl's own first user message (transcript
/// shape: `type:"message"`, `message.role:"user"`, `message.content[].text`).
const PI_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sys
from pathlib import Path
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
split = argv.index('--')
globs = [v for v in argv[:split] if v.strip()]
ids = [v for v in argv[split + 1:] if v.strip()]
if not ids or not globs:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
wanted = set(ids)

def first_prompt(path):
    try:
        with open(path, 'r', encoding='utf-8', errors='ignore') as fh:
            for i, line in enumerate(fh):
                if i > 400:
                    return None
                try:
                    v = json.loads(line)
                except Exception:
                    continue
                if v.get('type') != 'message':
                    continue
                m = v.get('message') or {}
                if m.get('role') != 'user':
                    continue
                c = m.get('content')
                if not isinstance(c, list):
                    continue
                text = '\n'.join(x.get('text', '') for x in c if isinstance(x, dict))
                for ln in text.splitlines():
                    ln = ln.strip()
                    if not ln or ln.startswith(('#', '<', '`')):
                        continue
                    return ln[:120]
    except Exception:
        return None
    return None

for g in globs:
    try:
        matches = home.glob(g)
    except Exception:
        continue
    for p in matches:
        if not p.is_file():
            continue
        name = p.stem
        sid = name.rsplit('_', 1)[-1]
        if sid in wanted:
            t = first_prompt(p)
            if t:
                print(json.dumps({'session_id': sid, 'candidates': [t]}, ensure_ascii=False))
"#;

const PI_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: PI_REMOTE_TITLE_SCRIPT,
    locators: RemoteStoreLocators::StoreGlobs,
    choose: first_non_empty_candidate,
};

/// Grok's remote twin: the session directory's `summary.json` — its own
/// model-generated title first, the session summary as the fallback, exactly
/// like the local reader.
const GROK_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sys
from pathlib import Path
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
split = argv.index('--')
globs = [v for v in argv[:split] if v.strip()]
ids = [v for v in argv[split + 1:] if v.strip()]
if not ids or not globs:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
wanted = set(ids)

def clean(value):
    if not isinstance(value, str):
        return None
    t = value.strip()
    return t or None

for g in globs:
    try:
        matches = home.glob(g)
    except Exception:
        continue
    for p in matches:
        try:
            v = json.load(open(p, 'r', encoding='utf-8', errors='ignore'))
        except Exception:
            continue
        info = v.get('info') or {}
        sid = info.get('id') or p.parent.name
        if sid not in wanted:
            continue
        candidates = [clean(v.get('generated_title')), clean(v.get('session_summary'))]
        candidates = [c for c in candidates if c]
        if candidates:
            print(json.dumps({'session_id': sid, 'candidates': candidates}, ensure_ascii=False))
"#;

const GROK_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: GROK_REMOTE_TITLE_SCRIPT,
    locators: RemoteStoreLocators::StoreGlobs,
    choose: first_non_empty_candidate,
};

/// OpenCode's remote twin: the shared opencode.db's own title column
/// (session_v2, then the v1 table). The db path is fixed relative to $HOME —
/// the descriptor declares no store globs, so the locators list is empty and
/// this script never uses argv's locator half.
/// Kimi's remote twin: the session directory's `state.json` — its own
/// `title` (the first prompt; `isCustomTitle` when renamed), matched by the
/// session directory's name, exactly like the local reader.
const KIMI_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sys
from pathlib import Path
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
globs = [v for v in argv[:argv.index('--')] if v.strip()]
ids = [v for v in argv[argv.index('--') + 1:] if v.strip()]
if not ids or not globs:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
wanted = set(ids)
seen = set()
for g in globs:
    try:
        matches = home.glob(g)
    except Exception:
        continue
    for p in matches:
        try:
            v = json.load(open(p, 'r', encoding='utf-8', errors='ignore'))
        except Exception:
            continue
        sid = p.parent.name
        if sid not in wanted or sid in seen:
            continue
        title = v.get('title')
        if isinstance(title, str) and title.strip():
            seen.add(sid)
            print(json.dumps({'session_id': sid, 'candidates': [title.strip()]}, ensure_ascii=False))
"#;

const OPENCODE_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sqlite3, sys
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
ids = [v for v in argv[argv.index('--') + 1:] if v.strip()]
if not ids:
    sys.exit(0)
db = os.path.expanduser('~/.local/share/opencode/opencode.db')
if not os.path.exists(db):
    sys.exit(0)
conn = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
cur = conn.cursor()
for sid in ids:
    row = None
    for table in ('session_v2', 'session'):
        try:
            cur.execute(f'SELECT title FROM {table} WHERE id = ?', (sid,))
            row = cur.fetchone()
        except Exception:
            row = None
        if row and row[0] and str(row[0]).strip():
            break
    if row and row[0]:
        title = str(row[0]).strip()
        if title.lower().startswith('new session - '):
            continue
        print(json.dumps({'session_id': sid, 'candidates': [title]}, ensure_ascii=False))
conn.close()
"#;

const OPENCODE_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: OPENCODE_REMOTE_TITLE_SCRIPT,
    locators: RemoteStoreLocators::HomeRelative(".local/share/opencode/opencode.db"),
    choose: first_non_empty_candidate,
};

const KIMI_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: KIMI_REMOTE_TITLE_SCRIPT,
    locators: RemoteStoreLocators::StoreGlobs,
    choose: first_non_empty_candidate,
};

const QWEN_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {    script: QWEN_REMOTE_TITLE_SCRIPT,
    // The title lives in the session's OWN chat file; Qwen keeps no shared index
    // beside it, so there is nothing to union in.
    locators: RemoteStoreLocators::StoreGlobs,
    choose: first_non_empty_candidate,
};

const QWEN_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sys
from pathlib import Path
TAIL_BYTES = 64 * 1024

# argv: <home-relative glob>... -- <session id>...
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
split = argv.index('--')
globs = [value for value in argv[:split] if value.strip()]
ids = [value for value in argv[split + 1:] if value.strip()]
if not globs or not ids:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
wanted = set(ids)

def title_from_tail(path):
    # The title is a `custom_title` record that this CLI re-appends near EOF, so
    # the LAST one wins and only the tail has to be read.
    try:
        size = path.stat().st_size
        with open(path, 'rb') as handle:
            handle.seek(max(0, size - TAIL_BYTES))
            raw = handle.read().decode('utf-8', errors='ignore')
    except Exception:
        return None
    lines = raw.splitlines()
    if size > TAIL_BYTES and lines:
        # First chunk is likely a partial line; drop it.
        lines = lines[1:]
    found = None
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except Exception:
            continue
        if record.get('type') != 'custom_title':
            continue
        for key in ('title', 'customTitle'):
            value = record.get(key)
            if value and str(value).strip():
                found = str(value).strip()
    return found

def session_id_of(path):
    # Cheapest first: a file NAMED for the session. The name is not
    # contractually the id here, so a miss falls through to the first record
    # rather than being reported as "no such session".
    if path.stem in wanted:
        return path.stem
    try:
        with open(path, encoding='utf-8', errors='ignore') as handle:
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except Exception:
                    return None
                value = record.get('sessionId')
                return value if value in wanted else None
    except Exception:
        return None
    return None

candidates = {session_id: [] for session_id in ids}
for glob in globs:
    try:
        matches = sorted(home.glob(glob))
    except Exception:
        continue
    for match in matches:
        session_id = session_id_of(match)
        if session_id is None:
            continue
        title = title_from_tail(match)
        if title:
            candidates[session_id].append(title)

for session_id in ids:
    found = candidates.get(session_id) or []
    if found:
        print(json.dumps({'session_id': session_id, 'candidates': found}, ensure_ascii=False))
"#;

const ANTIGRAVITY_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: ANTIGRAVITY_REMOTE_TITLE_SCRIPT,
    // ⛔ The store globs LEAD, because they are the only source a new
    // conversation has: `conversation_summaries.db` is written late (and by the
    // measurement above, often not at all for the rows on screen), while the
    // transcript exists from the first turn. Ranking inside the script keeps
    // the CHOSEN order the local reader's — index title, index preview,
    // history, then transcript — so the two halves cannot answer differently.
    locators: RemoteStoreLocators::StoreGlobsAndCliHomeFiles(&[
        "conversation_summaries.db",
        "history.jsonl",
    ]),
    choose: first_agy_prompt_line_candidate,
};

const ANTIGRAVITY_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sqlite3, sys
from pathlib import Path

# argv: <home-relative locator>... -- <session id>...
# A locator is either a literal file (the shared index) or a glob naming the
# per-conversation stores. Both are needed: see the registry's note on why the
# index alone answers for nothing on a fresh conversation.
argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
split = argv.index('--')
locators = [value for value in argv[:split] if value.strip()]
ids = [value for value in argv[split + 1:] if value.strip()]
if not locators or not ids:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
wanted = set(ids)
# rank -> the local reader's own precedence: index title, index preview,
# history display, transcript prompt. Ranking here rather than relying on
# locator order keeps the remote answer identical to the local one.
RANK_TITLE, RANK_PREVIEW, RANK_HISTORY, RANK_TRANSCRIPT = 0, 1, 2, 3
candidates = {session_id: [] for session_id in ids}

def offer(session_id, rank, value):
    if session_id in candidates and value and str(value).strip():
        candidates[session_id].append((rank, str(value)))

def open_read_only(path):
    # Never take a write lock on a store yggterm does not own. `mode=ro` still
    # wants to recover a hot WAL, which a read-only opener cannot do, so fall
    # back to `immutable=1` — that reads the main database file alone and may
    # miss the newest rows, which is a stale title rather than no title.
    for uri in (f'file:{path}?mode=ro', f'file:{path}?immutable=1'):
        try:
            return sqlite3.connect(uri, uri=True, timeout=2.0)
        except Exception:
            continue
    return None

def read_summaries(path):
    conn = open_read_only(path)
    if conn is None:
        return
    try:
        placeholders = ','.join('?' * len(ids))
        rows = conn.execute(
            'SELECT conversation_id, title, preview FROM conversation_summaries '
            'WHERE conversation_id IN (%s);' % placeholders, ids).fetchall()
        for conversation_id, title, preview in rows:
            offer(conversation_id, RANK_TITLE, title)
            offer(conversation_id, RANK_PREVIEW, preview)
    except Exception:
        pass
    finally:
        try:
            conn.close()
        except Exception:
            pass

def read_history(path):
    try:
        with open(path, encoding='utf-8', errors='ignore') as handle:
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except Exception:
                    continue
                offer(record.get('conversationId'), RANK_HISTORY, record.get('display'))
    except Exception:
        return

def transcript_id(path):
    # The conversation id is a DIRECTORY segment of the transcript path, not
    # its stem — the stem is the same word for every conversation. Matching on
    # the parts is exact and needs no knowledge of the store's depth.
    for part in path.parts:
        if part in wanted:
            return part
    return None

def read_transcript(path):
    session_id = transcript_id(path)
    if session_id is None:
        return
    try:
        with open(path, encoding='utf-8', errors='ignore') as handle:
            for index, line in enumerate(handle):
                if index > 20:
                    break
                line = line.strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except Exception:
                    continue
                if record.get('type') != 'USER_INPUT':
                    continue
                content = record.get('content')
                if content and str(content).strip():
                    offer(session_id, RANK_TRANSCRIPT, content)
                    return
    except Exception:
        return

for locator in locators:
    if any(character in locator for character in '*?['):
        try:
            matches = sorted(home.glob(locator))
        except Exception:
            continue
        for match in matches:
            # ⛔ A DELIBERATE, NAMED SKIP, not an oversight. A glob may also name
            # this CLI's per-conversation `.db` and its legacy `.json` layout.
            # Their schemas have not been measured, and a decoder written
            # against a guessed schema returns EMPTY rather than an error — it
            # would read as "this conversation has no title" forever. The
            # transcript is the source that has been measured, so it is the one
            # decoded; the others are listed so a future measurement has a
            # place to land.
            if match.suffix != '.jsonl':
                continue
            read_transcript(match)
        continue
    path = home / locator
    if not path.exists():
        continue
    if path.suffix == '.db':
        read_summaries(path)
    else:
        read_history(path)

for session_id in ids:
    found = [value for _, value in sorted(candidates.get(session_id) or [],
                                          key=lambda pair: pair[0])]
    if found:
        print(json.dumps({'session_id': session_id, 'candidates': found}, ensure_ascii=False))
"#;

/// [`RemoteStoreTitleProbe::choose`] for a CLI whose store hands back finished
/// titles: the first non-empty candidate, in the order the probe emitted them.
fn first_non_empty_candidate(candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .map(|candidate| candidate.trim())
        .find(|candidate| !candidate.is_empty())
        .map(str::to_string)
}

/// [`RemoteStoreTitleProbe::choose`] for Antigravity: its index holds whole
/// PROMPTS, not titles, so the same cleaner the local reader applies decides
/// which candidate is usable.
fn first_agy_prompt_line_candidate(candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .find_map(|candidate| clean_agy_prompt_first_line(candidate))
}

const MUSE_REMOTE_TITLE_PROBE: RemoteStoreTitleProbe = RemoteStoreTitleProbe {
    script: MUSE_REMOTE_TITLE_SCRIPT,
    locators: RemoteStoreLocators::StoreGlobsAndCliHomeFiles(&["session-index.db"]),
    choose: first_muse_title_candidate,
};

fn first_muse_title_candidate(candidates: &[String]) -> Option<String> {
    candidates.iter().find_map(|candidate| {
        let trimmed = candidate.trim();
        if trimmed.is_empty() || trimmed.starts_with('/') {
            return None;
        }
        // Muse's store candidates are raw prompts as often as finished DB
        // titles. A raw prompt beginning `please ...` is intentionally
        // low-signal AS A TITLE, but it is excellent title input. The local
        // durable reader condenses first; rejecting first made the ssh reader
        // answer `no_title_in_store` for the same conversation.
        let condensed = crate::best_effort_title_from_context(trimmed).or_else(|| {
            let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();
            if !first_line.is_empty() && first_line.len() <= 120 {
                Some(first_line.to_string())
            } else {
                Some(trimmed.chars().take(80).collect())
            }
        })?;
        (!crate::looks_like_generated_fallback_title(&condensed)
            && !crate::looks_like_low_signal_generated_copy(&condensed))
            .then_some(condensed)
    })
}

const MUSE_REMOTE_TITLE_SCRIPT: &str = r#"
import json, os, sqlite3, sys
from pathlib import Path

argv = sys.argv[1:]
if '--' not in argv:
    sys.exit(0)
split = argv.index('--')
locators = [value for value in argv[:split] if value.strip()]
ids = [value for value in argv[split + 1:] if value.strip()]
if not locators or not ids:
    sys.exit(0)
home = Path(os.path.expanduser('~'))
wanted = set(ids)
candidates = {session_id: [] for session_id in ids}

def offer(session_id, value):
    if session_id in candidates and value and str(value).strip():
        candidates[session_id].append(str(value).strip())

def read_db(path):
    if not path.exists():
        return
    for uri in (f'file:{path}?mode=ro', f'file:{path}?immutable=1'):
        try:
            conn = sqlite3.connect(uri, uri=True, timeout=2.0)
            break
        except Exception:
            conn = None
    if conn is None:
        return
    try:
        placeholders = ','.join('?' * len(ids))
        rows = conn.execute(
            'SELECT session_id, title, workspace_root FROM sessions '
            'WHERE session_id IN (%s);' % placeholders, ids).fetchall()
        for session_id, title, ws in rows:
            if title and title.strip() and title.strip() != session_id and title.strip() != (ws or '').strip():
                offer(session_id, title)
    except Exception:
        pass
    finally:
        try:
            conn.close()
        except Exception:
            pass

def read_jsonl(path):
    session_id = path.parent.name
    if session_id not in wanted:
        return
    try:
        with open(path, encoding='utf-8', errors='ignore') as handle:
            # Muse writes a large startup/lifecycle prelude before the first
            # accepted user intent. The local reader deliberately scans until
            # that intent; the remote reader must answer the same question.
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except Exception:
                    continue
                pt = record.get('payload_type', '')
                if pt in ('runtime.user_intent.accepted', 'runtime.user_intent.materialized'):
                    payload = record.get('payload') or {}
                    msgs = payload.get('model_messages') or []
                    for message in msgs:
                        for content in (message.get('content') or []):
                            text = content.get('text')
                            if text and str(text).strip():
                                offer(session_id, str(text).strip())
                    for block in (payload.get('refill_blocks') or []):
                        text = block.get('text')
                        if text and str(text).strip():
                            offer(session_id, str(text).strip())
    except Exception:
        pass

for locator in locators:
    if any(character in locator for character in '*?['):
        try:
            matches = sorted(home.glob(locator))
        except Exception:
            continue
        for match in matches:
            if match.name == 'session.jsonl':
                read_jsonl(match)
        continue
    path = home / locator
    if not path.exists():
        continue
    if path.suffix == '.db':
        read_db(path)

for session_id in ids:
    found = candidates.get(session_id) or []
    if found:
        print(json.dumps({'session_id': session_id, 'candidates': found}, ensure_ascii=False))
"#;


fn read_antigravity_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        // Layout: ~/.gemini/antigravity-cli/brain/<uuid>/.system_generated/logs/transcript_full.jsonl
        let session_id = path
            .parent()?
            .parent()?
            .parent()?
            .file_name()?
            .to_str()?
            .to_string();
        if session_id.is_empty() || session_id == "transcript" {
            return None;
        }

        let mut cwd = None;
        let mut title = None;
        let mut detail = None;

        // Try reading matching entry from conversation_summaries.db if available
        if let Some(gemini_dir) = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let db_path = gemini_dir.join("conversation_summaries.db");
            if db_path.exists() {
                if let Ok(conn) = rusqlite::Connection::open_with_flags(
                    &db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_URI
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                ) {
                    if let Ok(mut stmt) = conn.prepare(
                        "SELECT title, preview, workspace_uris FROM conversation_summaries WHERE conversation_id = ?1;",
                    ) {
                        if let Ok(mut rows) = stmt.query(rusqlite::params![session_id]) {
                            if let Ok(Some(row)) = rows.next() {
                                let t: String = row.get(0).unwrap_or_default();
                                let p: String = row.get(1).unwrap_or_default();
                                let uris: String = row.get(2).unwrap_or_default();
                                let t = t.trim();
                                let p = p.trim();
                                if !t.is_empty() {
                                    title = Some(t.to_string());
                                } else if !p.is_empty() {
                                    title = Some(p.to_string());
                                }
                                if let Some(parsed_cwd) = crate::parse_antigravity_workspace_uris(&uris) {
                                    cwd = Some(parsed_cwd);
                                }
                            }
                        }
                    }
                }
            }

            // Try reading matching entry from history.jsonl if available
            let history_path = gemini_dir.join("history.jsonl");
            if let Ok(file) = std::fs::File::open(&history_path) {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(file);
                for line in reader.lines().flatten() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                        if value.get("conversationId").and_then(|v| v.as_str()) == Some(&session_id) {
                            if cwd.is_none() {
                                if let Some(ws) = value.get("workspace").and_then(|v| v.as_str()) {
                                    if !ws.trim().is_empty() {
                                        cwd = Some(ws.trim().to_string());
                                    }
                                }
                            }
                            if let Some(display) = value.get("display").and_then(|v| v.as_str()) {
                                if !display.trim().is_empty() {
                                    if detail.is_none() {
                                        detail = Some(display.trim().to_string());
                                    }
                                    if title.is_none() {
                                        title = clean_agy_prompt_first_line(display);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Also check first few lines of transcript.jsonl for prompt and workspace URI
        if let Ok(file) = std::fs::File::open(path) {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(file);
            for (idx, line) in reader.lines().flatten().enumerate() {
                if idx > 20 {
                    break;
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    if value.get("type").and_then(|v| v.as_str()) == Some("USER_INPUT") {
                        if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
                            if detail.is_none() {
                                let prompt = if let Some(idx) = content.find("<USER_REQUEST>") {
                                    let after = &content[idx + "<USER_REQUEST>".len()..];
                                    after.split("</USER_REQUEST>").next().unwrap_or(after).trim()
                                } else {
                                    content.trim()
                                };
                                if !prompt.is_empty() {
                                    detail = Some(prompt.to_string());
                                    if title.is_none() {
                                        title = clean_agy_prompt_first_line(content);
                                    }
                                }
                            }
                            if cwd.is_none() && content.contains("[URI] -> [CorpusName]:") {
                                if let Some(idx) = content.find("[URI] -> [CorpusName]:") {
                                    let after = &content[idx + "[URI] -> [CorpusName]:".len()..];
                                    for l in after.lines() {
                                        let l = l.trim();
                                        if let Some((ws, _)) = l.split_once(" -> ") {
                                            if !ws.trim().is_empty() {
                                                cwd = Some(ws.trim().to_string());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if title.is_none() {
            if let Some(d) = detail.as_deref() {
                title = clean_agy_prompt_first_line(d).or_else(|| crate::best_effort_title_from_context(d));
            }
        }

        let cwd = cwd.or_else(|| dirs::home_dir().map(|h| h.to_string_lossy().to_string()))?;
        return Some(AgentStoreEntry {
            session_id,
            cwd,
            modified_epoch_ms: modified_epoch_ms_of(path),
            title,
            detail,
        });
    }

    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        let raw = std::fs::read_to_string(path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let session_id = value.get("id")?.as_str()?.to_string();
        let cwd = value
            .get("projectResources")
            .and_then(|r| r.get("resources"))
            .and_then(|r| r.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|resource| {
                    resource
                        .get("gitFolder")?
                        .get("folderUri")?
                        .as_str()?
                        .strip_prefix("file://")
                        .map(|path| path.to_string())
                })
            })
            .or_else(|| dirs::home_dir().map(|h| h.to_string_lossy().to_string()))?;
        if session_id.is_empty() || cwd.is_empty() {
            return None;
        }
        let title = value
            .get("name")
            .and_then(|name| name.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty() && *name != cwd)
            .map(|name| name.to_string());
        return Some(AgentStoreEntry {
            session_id,
            cwd,
            modified_epoch_ms: modified_epoch_ms_of(path),
            title,
            detail: None,
        });
    }

    let session_id = if path.file_name().and_then(|n| n.to_str()) == Some("transcript.jsonl") {
        let mut p = path.parent();
        while let Some(parent) = p {
            let name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name != "logs" && name != ".system_generated" && !name.is_empty() {
                break;
            }
            p = parent.parent();
        }
        p.and_then(|d| d.file_name()).and_then(|n| n.to_str())?.to_string()
    } else {
        path.file_stem()?.to_str()?.to_string()
    };
    if session_id.is_empty() || session_id == "transcript" || session_id.ends_with("-shm") || session_id.ends_with("-wal") {
        return None;
    }
    let home = dirs::home_dir()?;
    let db_path = home.join(".gemini/antigravity-cli/conversation_summaries.db");
    let mut title = None;
    let mut cwd = None;
    if db_path.exists() {
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT title, preview, workspace_uris FROM conversation_summaries WHERE conversation_id = ?1;",
            ) {
                if let Ok(mut rows) = stmt.query(rusqlite::params![session_id]) {
                    if let Ok(Some(row)) = rows.next() {
                        let t: String = row.get(0).unwrap_or_default();
                        let p: String = row.get(1).unwrap_or_default();
                        let uris: String = row.get(2).unwrap_or_default();
                        let t = t.trim();
                        let p = p.trim();
                        if !t.is_empty() {
                            title = Some(t.to_string());
                        } else if !p.is_empty() {
                            title = Some(p.to_string());
                        }
                        cwd = crate::parse_antigravity_workspace_uris(&uris);
                    }
                }
            }
        }
    }
    let title = title
        .filter(|t| !crate::looks_like_generated_fallback_title(t) && !crate::looks_like_low_signal_generated_copy(t))
        .or_else(|| {
            crate::titles::extract_tail_context(path)
                .ok()
                .and_then(|ctx| crate::titles::heuristic_title_from_context(&ctx))
                .filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
                .filter(|s| !s.contains("/home/"))
        });
    let detail = crate::titles::extract_tail_context(path)
        .ok()
        .filter(|context| !context.trim().is_empty())
        .filter(|c| !crate::looks_like_low_signal_generated_copy(c) && !crate::looks_like_generated_fallback_title(c));
    let cwd = cwd.unwrap_or_else(|| home.display().to_string());
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title,
        detail,
    })
}

/// Open one of a CLI's SQLite indexes read-only, without ever creating it.
///
/// ⛔ Read-only AND non-creating on purpose: these are the CLI's own live
/// databases. Opening one read-write would take a lock the CLI is using, and a
/// default `open()` on a missing path CREATES an empty database — which would
/// then answer "no such session" authoritatively forever.
fn open_cli_index_readonly(db_path: &Path) -> Option<rusqlite::Connection> {
    if !db_path.exists() {
        return None;
    }
    rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

/// Does Muse's session index hold `session_id`?
///
/// Muse keys `sessions.session_id` on the same uuid that names the session's
/// DIRECTORY, so one indexed lookup settles what would otherwise be a walk of
/// `sessions/YYYY/MM/DD/*/session.jsonl`.
fn muse_store_index_holds_session(home: &Path, session_id: &str) -> Option<bool> {
    let conn = open_cli_index_readonly(&home.join(".local/share/muse/session-index.db"))?;
    let mut stmt = conn
        .prepare("SELECT 1 FROM sessions WHERE session_id = ?1;")
        .ok()?;
    stmt.exists(rusqlite::params![session_id]).ok()
}

/// Does opencode's own SQLite store hold `session_id`?
///
/// ⛔ Reads `session_v2` FIRST and falls back to the v1-era `session` table:
/// the v2 preview writes new sessions to `session_v2` and stops writing
/// `session` (measured 2026-08-29 — `session` held 3 stale rows while the
/// service served 11, so a `session`-only probe answered "absent" for a REAL
/// `ses_…` id and a live row was refused as "no longer available on this
/// machine"). `None` = this host cannot answer (no DB, unreadable) — callers
/// must never read it as absence.
pub fn opencode_store_index_holds_session(home: &Path, session_id: &str) -> Option<bool> {
    if session_id.trim().is_empty() {
        return None;
    }
    let conn = open_cli_index_readonly(&home.join(".local/share/opencode/opencode.db"))?;
    for table in ["session_v2", "session"] {
        let present = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                rusqlite::params![table],
                |row| row.get::<_, i64>(0),
            )
            .ok()?;
        if present == 0 {
            continue;
        }
        let sql = format!("SELECT 1 FROM {table} WHERE id = ?1;");
        if let Ok(mut stmt) = conn.prepare(&sql) {
            match stmt.exists(rusqlite::params![session_id]) {
                Ok(true) => return Some(true),
                Ok(false) => continue,
                Err(_) => return None,
            }
        }
    }
    Some(false)
}

/// Does Antigravity hold `session_id`?
///
/// ⛔ NOT via `conversation_summaries.db`, which is the obvious answer and the
/// WRONG one. Measured 2026-08-20: a conversation created by the CLI gets a
/// `brain/<id>/` directory and a `conversations/<id>.db` immediately, and is
/// STILL ABSENT from `conversation_summaries.db` afterwards — yet
/// `agy --conversation <id>` resumes it without complaint. Asking the summaries
/// table would therefore report a live, resumable conversation as missing, and a
/// caller acting on that would re-birth over it.
///
/// The per-conversation artefacts are the authority, and they are a path check
/// rather than a query: the file NAME is the id, exactly as Claude Code's is.
/// The summaries table is kept only as an additional yes, never as a no.
fn antigravity_store_index_holds_session(home: &Path, session_id: &str) -> Option<bool> {
    // A session id is used to build a path here, so anything that could escape
    // the directory is refused rather than sanitized.
    if session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
    {
        return None;
    }
    let root = home.join(".gemini/antigravity-cli");
    if !root.exists() {
        // Antigravity has never run here, so this host cannot testify at all.
        return None;
    }
    if root.join("conversations").join(format!("{session_id}.db")).exists()
        || root.join("brain").join(session_id).exists()
    {
        return Some(true);
    }
    if let Some(conn) = open_cli_index_readonly(&root.join("conversation_summaries.db")) {
        if let Ok(mut stmt) =
            conn.prepare("SELECT 1 FROM conversation_summaries WHERE conversation_id = ?1;")
        {
            if stmt.exists(rusqlite::params![session_id]) == Ok(true) {
                return Some(true);
            }
        }
    }
    Some(false)
}

fn read_muse_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    // Exclude subagent and tool-output sessions — they live under the parent
    // session's directory and share no cwd/title of their own; including them
    // would 10× the durable count with title-less rows.
    let path_str = path.display().to_string();
    if path_str.contains("/subagent/") || path_str.contains("/tool-outputs/") {
        return None;
    }
    // Muse lays out `~/.local/share/muse/sessions/YYYY/MM/DD/<uuid>/session.jsonl`
    // so the session_id is the parent directory name, not the file stem.
    let session_id = path
        .parent()?
        .file_name()?
        .to_str()?
        .trim()
        .to_string();
    if session_id.is_empty() {
        return None;
    }
    // Validate that it looks like a UUID (36 chars with dashes) — avoid
    // picking up tui-history.jsonl or other files that happen to match globs.
    if session_id.len() < 8 || !session_id.contains('-') {
        return None;
    }
    let data_root = muse_data_root_from_session_path(path)?;
    let home = data_root
        .ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some(".local"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(dirs::home_dir)?;
    // Prefer the SQLite index for cwd/title/mtime — it is the same source
    // `muse resume` lists from, and it contains the workspace_root and
    // already-extracted title without scanning multi-MB JSONL.
    let (db_cwd, db_title, db_updated_ms) = 'db_block: {
        let db_path = data_root.join("session-index.db");
        if db_path.exists() {
            if let Ok(conn) = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_URI
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            ) {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT workspace_root, title, updated_at_us FROM sessions WHERE session_id=?1",
                ) {
                    if let Ok(mut rows) = stmt.query(rusqlite::params![session_id]) {
                        if let Ok(Some(row)) = rows.next() {
                            let ws: Option<String> = row.get(0).ok();
                            let title: Option<String> = row.get(1).ok();
                            let updated_us: Option<i64> = row.get(2).ok();
                            let mtime = updated_us
                                .filter(|v| *v > 0)
                                .map(|v| (v / 1000) as u128)
                                .unwrap_or_else(|| modified_epoch_ms_of(path));
                            let cwd = ws
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty());
                            let title = title
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty() && s != &session_id)
                                .filter(|s| !crate::looks_like_generated_fallback_title(s))
                                .filter(|s| !crate::looks_like_low_signal_generated_copy(s));
                            // A title that equals the workspace_root is the placeholder
                            // Muse writes when it has not yet titled the session.
                            let title = match (&cwd, &title) {
                                (Some(cwd), Some(t)) if t == cwd => None,
                                _ => title,
                            };
                            break 'db_block (cwd, title, mtime);
                        }
                    }
                }
            }
        }
        (None, None, modified_epoch_ms_of(path))
    };
    // Fallback cwd from route_facts if DB absent or empty.
    let cwd = db_cwd.or_else(|| {
        use std::io::{BufRead, BufReader};
        let file = std::fs::File::open(path).ok()?;
        let reader = BufReader::new(file);
        for line in reader.lines().flatten().take(16) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if value.get("payload_type").and_then(|v| v.as_str()) == Some("runtime.session.route_facts") {
                    if let Some(cwd) = value
                        .get("payload")
                        .and_then(|p| p.get("record"))
                        .and_then(|r| r.get("cwd"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        return Some(cwd.to_string());
                    }
                }
            }
        }
        None
    }).unwrap_or_else(|| home.display().to_string());

    // If DB title is missing or looks generated, try Muse-native title
    // extraction before the Codex-shaped heuristic. Muse's JSONL is
    // payload_type runtime.user_intent.accepted with model_messages[0].content[0].text,
    // not a Codex rollout — extract_tail_context would return empty and the
    // session would fall back to the short id (the `1230f99` bug).
    let muse_jsonl_title = if db_title.is_none() {
        muse_title_from_session_jsonl(path)
            .filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
    } else {
        None
    };
    let effective_title = db_title.clone().filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s)).or_else(|| muse_jsonl_title.clone()).or_else(|| {
        crate::titles::extract_tail_context(path).ok().and_then(|ctx| crate::titles::heuristic_title_from_context(&ctx)).filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
    });
    let effective_detail = db_title.clone().filter(|s| !crate::looks_like_low_signal_generated_copy(s)).or(muse_jsonl_title);
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: db_updated_ms,
        title: effective_title,
        detail: effective_detail,
    })
}

fn read_claude_code_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    let (session_id, cwd) = crate::read_cc_session_identity_fields(path)
        .ok()
        .flatten()?;
    // Filter raw-path / shorthash titles that the harness flags as weird:
    // a first-human text of "/home/user/.yggterm/clipboard/..." is not a title.
    let raw_title = crate::read_cc_session_title(path).ok().flatten();
    let title = raw_title
        .filter(|t| !t.trim().is_empty())
        .filter(|t| !crate::looks_like_generated_fallback_title(t))
        .filter(|t| !crate::looks_like_low_signal_generated_copy(t))
        .filter(|t| !t.contains("/home/") && !t.starts_with('/'));
    // If title is raw path, try heuristic from tail context as fallback.
    let title = title.or_else(|| {
        crate::titles::extract_tail_context(path)
            .ok()
            .and_then(|ctx| crate::titles::heuristic_title_from_context(&ctx))
            .filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
            .filter(|s| !s.contains("/home/"))
    });
    let detail = crate::read_cc_session_context(path)
        .ok()
        .filter(|context| !context.trim().is_empty())
        .filter(|c| !crate::looks_like_low_signal_generated_copy(c) && !crate::looks_like_generated_fallback_title(c))
        .filter(|c| !c.contains("/home/.yggterm/clipboard"));
    Some(AgentStoreEntry {
        session_id,
        cwd,
        modified_epoch_ms: modified_epoch_ms_of(path),
        title,
        detail,
    })
}

/// The descriptor for `kind`, or `None` for a non-agent kind.
pub fn agent_cli_descriptor(kind: SessionKind) -> Option<&'static AgentCliDescriptor> {
    AGENT_CLIS.iter().find(|descriptor| descriptor.kind == kind)
}

/// The ONE lowercase wire name for a session kind — the `--kind` value, the
/// `session_kind_label` string in telemetry, the row JSON's `icon_kind`.
///
/// The agent half is `descriptor.slug` rather than nine hand-written arms,
/// because that slug is the same string the flag parser accepts: spelling them
/// separately is how a kind could be LABELLED `pi` and yet be unparseable as
/// `--kind pi`.
///
/// It lives here rather than in `yggterm-server` because the copy layer in
/// [`crate::titles`] must recognise a title composed out of one, and a second
/// copy of the vocabulary over there is exactly the drift this crate exists to
/// prevent. `yggterm-server` re-exports it, so its call sites are unchanged.
pub fn session_kind_label(kind: SessionKind) -> &'static str {
    if let Some(descriptor) = agent_cli_descriptor(kind) {
        return descriptor.slug;
    }
    match kind {
        SessionKind::Shell => "shell",
        // Historical spelling: `ssh`, not `ssh-shell`. It is on disk and on the
        // wire, so it stays hand-written next to the kinds that have no slug.
        SessionKind::SshShell => "ssh",
        SessionKind::Document => "document",
        // Unreachable: every remaining kind has a descriptor and returned above.
        _ => "shell",
    }
}

/// Every lowercase wire name a session kind can wear, so a reader of a title
/// can ask whether a token is one without owning a list of its own.
pub fn session_kind_label_is_known(token: &str) -> bool {
    if AGENT_CLIS.iter().any(|descriptor| descriptor.slug == token) {
        return true;
    }
    [
        SessionKind::Shell,
        SessionKind::SshShell,
        SessionKind::Document,
    ]
    .iter()
    .any(|kind| session_kind_label(*kind) == token)
}

/// The brand colour for `kind`, or `None` for a non-agent kind (a plain shell,
/// a document) which paints in the theme accent instead.
///
/// See [`AgentCliDescriptor::brand_color`] for the accessibility constraint on
/// the values.
pub fn agent_cli_brand_color(kind: SessionKind) -> Option<&'static str> {
    agent_cli_descriptor(kind).map(|descriptor| descriptor.brand_color)
}

/// What the start page's open control on a row of `kind` should SAY.
///
/// **The CLI is named because the page is a recovery surface**: it is reached
/// when the sidebar has failed, and at that moment "Open" tells the reader
/// nothing about which of nine CLIs they are about to resume. The verb used to
/// be a three-arm `match` that named two CLIs and answered a bare `"Open"` for
/// the other seven.
///
/// `None` — a plain shell or a document — keeps the generic verb, because there
/// is no CLI to name.
pub fn agent_cli_open_session_label(kind: Option<SessionKind>) -> String {
    match kind.and_then(agent_cli_descriptor) {
        Some(descriptor) => format!("Open this {} Session", descriptor.display_name),
        None => "Open".to_string(),
    }
}

/// What a row of `kind` is CALLED AT BIRTH, before its CLI has titled itself.
///
/// ⛔ **A birth name describes WHAT THE ROW IS, and nothing else.** It used to
/// be `format!("{} {}", row.label, slug)` — the label of whichever row the
/// context menu happened to be opened on, plus the CLI slug — which is a name
/// for the SPAWNER, not the spawned. Right-clicking a session to start a
/// neighbour therefore minted a near-copy of that session's title, and two
/// sidebar rows read almost identically until the CLI got around to titling
/// itself. On a forty-row sidebar that is the difference between an instrument
/// and a wall of text.
///
/// The composer that did it was named for the case it was written for — a
/// GROUP row, where `row.label` is a folder name and `"widgets codex"` reads
/// sensibly. Nothing stopped it being handed a session row, and the menu that
/// does so is the one people actually use.
///
/// ⚠ This is a PLACEHOLDER by contract. The row renames itself the moment its
/// first real title arrives, by whichever mechanism owns titles for that kind
/// ([`TitleAuthority`]) — so this string's job is to be unmistakable for the
/// few seconds it is on screen, not to be durable.
///
/// ⛔ **AND IT NAMES THE MACHINE, because a row plane spanning several hosts is
/// what this product is.** The rule is `New {machine} {what it is}` for every
/// row that is born unnamed — an agent CLI, a libyggterm app, a terminal — and
/// it is ONE rule with ONE owner. It was three: a shell was titled by its
/// working directory (so a terminal wore an absolute path), an agent CLI got
/// `New {display_name} Session` with no machine in it, and an app row wore
/// whichever MENU ITEM had launched it. Three rows born within a second of each
/// other therefore agreed about nothing, and none of them said where they were.
pub fn new_session_birth_title(kind: SessionKind, machine: Option<&str>) -> String {
    new_row_birth_title(machine, session_kind_birth_noun(kind))
}

/// What a row of `kind` IS, in the words a person would use — the tail of
/// [`new_session_birth_title`].
///
/// Split out because a libyggterm APP row is a `SessionKind::Shell` whose noun
/// is its app's label, not "Terminal": the kind alone cannot answer, and the
/// caller that knows (the one holding the row's `app:<name>:<verb>` stamp)
/// composes with [`new_row_birth_title`] directly.
pub fn session_kind_birth_noun(kind: SessionKind) -> &'static str {
    match agent_cli_descriptor(kind) {
        Some(descriptor) => descriptor.display_name,
        None => match kind {
            // Both shells are "Terminal": the machine name already says which
            // host it is on, so "SSH Terminal" on a row that reads
            // `New oc Terminal` would be saying it twice.
            SessionKind::Shell | SessionKind::SshShell => "Terminal",
            SessionKind::Document => "Document",
            // Unreachable while every agent kind has a descriptor, which
            // `SessionKind::is_agent` derives from this very registry — so a
            // new kind reaching here is a missing registration, not a name to
            // invent. Stay generic rather than guess a product name.
            _ => "Session",
        },
    }
}

/// Is `title` a name [`new_row_birth_title`] composed — `New [{machine}] {noun}`?
///
/// ⛔ **THE RECOGNISER MUST MOVE WITH THE COMPOSER, and this is why it is
/// derived rather than listed.** Every gate that asks "may this row still be
/// re-titled" runs through `looks_like_generated_fallback_title`, which was a
/// table of literal strings — `"new antigravity session"`, `"new terminal"`,
/// and so on. Put a machine name in the middle and every one of those literals
/// stops matching, so the chore would read a two-second-old placeholder as a
/// real title and leave it on the row for ever. Changing the composer without
/// this would have traded a name with no machine in it for a name that never
/// gets replaced.
///
/// Tight on purpose: `New ` prefix, then AT MOST one token of machine before a
/// noun this build actually composes with. A real generated title is not this
/// shape.
pub fn is_new_row_birth_title(title: &str) -> bool {
    let Some(rest) = title.trim().strip_prefix("New ") else {
        return false;
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return false;
    }
    birth_nouns().any(|noun| {
        // ⚠ CASE-SENSITIVE on the noun, and that is what keeps this tight. The
        // composer always emits the registry's own spelling (`Terminal`,
        // `Claude Code`), so a real generated title that happens to end in the
        // same word — "New async terminal" — does not match, while
        // `New box Terminal` does.
        let Some(head) = rest.strip_suffix(noun) else {
            return false;
        };
        // Nothing before the noun (no machine), or exactly one token (the
        // machine name). Two or more is somebody's sentence, not a birth name.
        match head.trim() {
            "" => head.is_empty(),
            machine => !machine.contains(char::is_whitespace),
        }
    })
}

/// Every noun the birth rule can compose with — the registry's display names
/// plus the non-agent kinds'. `SessionKind::ALL` drives the second half so a new
/// kind is covered by adding it to the enum, not by remembering this function.
fn birth_nouns() -> impl Iterator<Item = &'static str> {
    AGENT_CLIS
        .iter()
        .map(|descriptor| descriptor.display_name)
        .chain(
            SessionKind::ALL
                .iter()
                .filter(|kind| !kind.is_agent())
                .map(|kind| session_kind_birth_noun(*kind)),
        )
}

/// The birth name itself: `New {machine} {what}`.
///
/// ⚠ An absent or blank machine degrades to `New {what}` rather than emitting a
/// double space or the word "unknown". A caller that cannot name the host is
/// giving a worse answer, not a wrong one, and a stray gap in a sidebar row is
/// a defect a reader blames on the row rather than on the caller.
pub fn new_row_birth_title(machine: Option<&str>, what: &str) -> String {
    let what = what.trim();
    match machine.map(str::trim).filter(|value| !value.is_empty()) {
        Some(machine) => format!("New {machine} {what}"),
        None => format!("New {what}"),
    }
}

/// Which CLI's store `path` lives under, if any. The store roots are mutually
/// exclusive by construction (`/.codex/sessions/` is not a substring of
/// `/.codex-litellm/sessions/`), and
/// [`agent_cli_store_roots_are_mutually_exclusive`] holds a new CLI to that.
/// The kind slug a ROW REPORTS for this CLI — `icon_kind` in the row JSON, and
/// the `data-tree-icon-kind` attribute the sidebar draws from.
///
/// ⛔⛔ **IT IS NOT ALWAYS [`AgentCliDescriptor::slug`], AND THAT IS THE WHOLE
/// REASON THIS EXISTS.** The codex family reports the historical `"session"` —
/// it predates there being a second CLI, it ships on the wire, and it may not be
/// renamed. Every other CLI reports its slug.
///
/// ⚠ The fleet's Python matched a row's kind against the store table's KEYS,
/// which are slugs, so it never narrowed for the codex family: **410 of 742 live
/// rows on 2026-08-22**. It was right only because session ids are unique across
/// stores, and that luck runs out the moment a caller needs to know WHETHER IT
/// LOOKED — which the reap does, immediately before destroying a row.
///
/// ⇒ So the mapping is stated ONCE, here, next to the registry it derives from,
/// and both the sidebar that produces the string and the lock that ratifies the
/// fleet's copy of it read this rather than restating the match.
pub fn row_icon_kind(kind: SessionKind) -> Option<&'static str> {
    let descriptor = agent_cli_descriptor(kind)?;
    Some(match kind {
        // ⛔ HISTORICAL. Both codex variants wear the same mark, so this slug does
        //    not identify which of the two a row is — a caller narrowing by it
        //    must keep both candidates.
        SessionKind::Codex | SessionKind::CodexLiteLlm => "session",
        _ => descriptor.slug,
    })
}

/// Whether yggterm should SUPPRESS this CLI's mouse-tracking DECSETs
/// (1000/1002/1003/1006) at the client, so the wheel translates to cursor
/// keys (alternate scroll) and clicks stay inert instead of emitting mouse
/// reports the CLI never acts on.
///
/// ⛔ MEASURED, not guessed (owner, 2026-09-02): the opencode v2 TUI arms
/// 1003 + 1006 (the ACT V mouse-mode probe witnesses the DECSETs live) and
/// then does nothing with wheel or click reports — the owner navigates it
/// with the keyboard because the mouse is dead on every surface. Suppressing
/// the arm lets the client's alternate-scroll own the pointer instead. A CLI
/// that genuinely handles mouse reports must NOT be listed here; flip this
/// off the day upstream opencode handles its own mouse mode.
pub fn suppresses_mouse_tracking(kind: SessionKind) -> bool {
    matches!(kind, SessionKind::OpenCode)
}

/// The wheel sequences for a suppressed-mouse CLI's alternate scroll, and how
/// much wheel delta one emitted key is worth.
///
/// ⛔ MEASURED PER CLI, never assumed (owner, 2026-09-02): the first cut sent
/// cursor keys — the standard alternate-scroll answer — and did nothing for
/// opencode, because opencode does not bind arrows to message scrolling at
/// all. Its own keybind table (read from the installed binary) binds
/// `messages_page_up: pageup` / `messages_page_down: pagedown`, with line
/// scrolls on ctrl+alt+y/e. So opencode's wheel is PageUp/PageDown, one key
/// per ~120 px of accumulated wheel (a trackpad fires dozens of small deltas;
/// one page per raw event would fling the transcript away).
///
/// Return: (wheel-up sequence, wheel-down sequence, accumulate-px; 0 = emit
/// per event, line-precise — the cursor-keys default).
pub fn alternate_scroll_keys(kind: SessionKind) -> (&'static str, &'static str, u32) {
    match kind {
        SessionKind::OpenCode => ("\x1b[5~", "\x1b[6~", 120),
        _ => ("\x1b[A", "\x1b[B", 0),
    }
}

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

/// ✅ EMPTY since 2026-08-08 — `selected_path_should_expand_ancestors` was
/// widened from the codex family to EVERY registered store, which closed the
/// one row this table held. The both-directions contract is what forced the
/// deletion in the same change: a recorded hole that no longer reproduces fails
/// the build, so the table can never go stale-green.
pub const KNOWN_STORE_PREDICATE_HOLES: &[StorePredicateHole] = &[];

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
        // A CLI with a declared `store_scan_gap` has no store PATH to probe —
        // `example_store_path` degenerates to the bare home, and asserting a
        // predicate against `/home/example/` measures nothing. The gap is
        // already locked by `every_agent_cli_declares_a_store`.
        if descriptor.session_store_globs.is_empty() {
            continue;
        }
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

    /// ⛔ A LIMIT-WAIT IS NOT IDLE. A session waiting out a usage limit paints
    /// a footer with no working phrase, so every daemon-owned surface called
    /// it "confirmed idle" at once and the working→done edge fired a false
    /// "done". The phrases are per-CLI descriptor data like the working set.
    #[test]
    fn a_usage_limit_wait_is_a_third_state_not_idle() {
        let cc = agent_cli_descriptor(SessionKind::ClaudeCode).expect("cc descriptor");
        let wait_screen = "some earlier output\n\
             Usage limit reached · continuing shortly · esc to cancel";
        assert!(cc.screen_shows_limit_wait(wait_screen));
        assert!(
            !cc.screen_shows_working(wait_screen),
            "the wait footer carries no working phrase — that gap is the bug"
        );
        // Conversation text that merely MENTIONS the limit is not the state:
        // the also_any guard requires the wait footer's own wording on the
        // same line.
        let prose = "the user asked what usage limit reached means\n\u{276f} ";
        assert!(!cc.screen_shows_limit_wait(prose));
        // A genuinely working screen is working, not limit-waiting.
        let working = "thinking…\nesc to interrupt";
        assert!(cc.screen_shows_working(working));
        assert!(!cc.screen_shows_limit_wait(working));
    }

    /// ⛔ A STORE COLUMN CALLED `title` IS NOT ALWAYS A TITLE.
    ///
    /// One shipped CLI records the FIRST PROMPT there and never updates it, so
    /// two rows on the desktop wore whole paragraphs of instructions where
    /// their names belong (measured 2026-08-20). The clamp is deterministic and
    /// model-free on purpose: the case it exists for is the case where the
    /// model is unreachable.
    #[test]
    fn a_store_title_that_is_really_a_prompt_is_clamped_to_a_row_label() {
        let prompt = "Read the campaign notes first. I want the profiling work \
                      finished on the desktop host, and the two tracing tools \
                      brought up to the same interface so one can drive the other.";
        let condensed = condense_store_title(prompt).expect("a label comes back");
        assert_eq!(condensed, "Read the campaign notes first");
        assert!(condensed.chars().count() <= STORE_TITLE_MAX_CHARS);

        // A title that is already a title is returned UNTOUCHED — the clamp
        // must not quietly rewrite the CLIs that get this right.
        assert_eq!(
            condense_store_title("Daemon Lifecycle Leak Audit").as_deref(),
            Some("Daemon Lifecycle Leak Audit")
        );
        // …including one that ends in a full stop but is still one clause.
        assert_eq!(
            condense_store_title("  Fix the resume path.  ").as_deref(),
            Some("Fix the resume path.")
        );
        // A single sentence too long to be a label is cut on a WORD boundary,
        // never mid-word: a truncated word reads as corruption.
        let one_long_sentence = "Investigate why the background refresh keeps \
                                 re-resolving every scanned session on the \
                                 desktop host every few seconds forever";
        let condensed = condense_store_title(one_long_sentence).expect("a label");
        assert!(condensed.chars().count() <= STORE_TITLE_MAX_CHARS);
        assert!(one_long_sentence.split_whitespace().collect::<Vec<_>>().starts_with(
            &condensed.split_whitespace().collect::<Vec<_>>()[..]
        ));
        assert_eq!(condense_store_title("   "), None);
    }

    /// ⭐ A SUMMARY IS NOT A TITLE. grok writes both — its own binary documents
    /// them as "`session_summary` and `generated_title` — the session summary
    /// and its model-generated title" — and this reader took the summary,
    /// which would name a row with a paragraph whenever grok had a real title
    /// for it. Preference order, and the empty-is-absent rule, pinned here.
    #[test]
    fn the_grok_reader_prefers_the_title_over_the_summary() {
        let dir = std::env::temp_dir().join(format!(
            "ygg-grok-reader-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let write = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).expect("fixture");
            path
        };
        let entry = |path: &std::path::Path| {
            read_grok_build_store_entry(path).expect("the fixture is a readable grok session")
        };

        // Both present ⇒ the TITLE wins.
        let both = write(
            "both.json",
            r#"{"info":{"id":"3f2a10bc-77d4-4e19-9a52-c1e8b0d6af73","cwd":"/w/p"},
                "generated_title":"Tidy the parser",
                "session_summary":"A long paragraph about tidying the parser."}"#,
        );
        assert_eq!(entry(&both).title.as_deref(), Some("Tidy the parser"));

        // Title absent ⇒ the summary is the fallback, not nothing.
        let summary_only = write(
            "summary.json",
            r#"{"info":{"id":"3f2a10bc-77d4-4e19-9a52-c1e8b0d6af73","cwd":"/w/p"},
                "session_summary":"A long paragraph about tidying the parser."}"#,
        );
        assert_eq!(
            entry(&summary_only).title.as_deref(),
            Some("A long paragraph about tidying the parser.")
        );

        // ⚠ The state actually observed on a signed-in host: a short session
        // that closed before grok generated either. EMPTY IS ABSENT — a blank
        // string must not become a blank row name.
        let neither = write(
            "neither.json",
            r#"{"info":{"id":"3f2a10bc-77d4-4e19-9a52-c1e8b0d6af73","cwd":"/w/p"},
                "session_summary":"   "}"#,
        );
        assert_eq!(entry(&neither).title, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

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
    /// Exactly one default tier per CLI that offers tiers at all — the invariant
    /// that lets `default_permission_preset` return an Option instead of a
    /// guess, and that a `permission_default: &str` naming a preset id could not
    /// have given (a dangling name is a compile-time-invisible bug).
    #[test]
    fn every_cli_with_presets_has_exactly_one_default() {
        for descriptor in AGENT_CLIS {
            if descriptor.permission_presets.is_empty() {
                continue;
            }
            let defaults = descriptor
                .permission_presets
                .iter()
                .filter(|preset| preset.is_default)
                .count();
            assert_eq!(
                defaults, 1,
                "{}: exactly one permission preset must be the default, found {defaults}",
                descriptor.display_name
            );
        }
    }

    /// ⛔ An EMPTY preset list is only honest for a CLI that reads another's box,
    /// or one whose flags nobody has measured. Anywhere else it is a CLI that
    /// silently offers the user nothing, which is how seven CLIs went a week
    /// without a way to receive a permission flag.
    ///
    /// And the converse: a NON-empty list beside `Unmeasured` is a guess wearing
    /// a measurement's clothes.
    #[test]
    fn every_cli_declares_presets_or_says_why() {
        for descriptor in AGENT_CLIS {
            let unmeasured = matches!(
                descriptor.permission_provenance,
                PermissionProvenance::Unmeasured(_)
            );
            if descriptor.permission_presets.is_empty() {
                assert!(
                    !descriptor.owns_its_extra_args_box() || unmeasured,
                    "{}: owns its own launch-flags box and offers no tiers — either give it \
                     tiers or declare PermissionProvenance::Unmeasured with the reason",
                    descriptor.display_name
                );
                continue;
            }
            assert!(
                !unmeasured,
                "{}: declares tiers AND says they are unmeasured. Measure them, or ship none.",
                descriptor.display_name
            );
            if let PermissionProvenance::Unmeasured(reason) = descriptor.permission_provenance {
                assert!(
                    reason.split_whitespace().count() >= 6,
                    "{}: an unmeasured reason must name the obstacle, not wave at it",
                    descriptor.display_name
                );
            }
        }
    }

    /// ⛔⛔ THE LOCK ON THE GAP BETWEEN THE TWO TITLE CHORES.
    ///
    /// yggterm titles a live row from two places: a local chore reading this
    /// machine's stores off disk, and a remote chore reading another machine's
    /// over ssh. A CLI that has a measured LOCAL reader and a remote arm is a
    /// CLI whose rows can exist on both sides of that seam — and if only the
    /// local half is wired, its `remote-*://` rows are titled by NOBODY: the
    /// local chore skips them for their scheme, the remote chore has nothing to
    /// run, and the row keeps its birth name for the life of the session.
    ///
    /// That is not hypothetical. It shipped, for every CLI but one, and was
    /// found by a person looking at a sidebar rather than by any instrument.
    #[test]
    fn a_cli_that_can_be_titled_locally_and_has_a_remote_arm_can_be_titled_remotely() {
        for descriptor in AGENT_CLIS {
            if descriptor.read_live_store_title.is_none() || !descriptor.has_remote_arm() {
                continue;
            }
            assert!(
                descriptor.remote_live_store_title.is_some(),
                "{}: its local store reader is measured and it has a remote arm, so its \
                 `remote-*://` rows exist and nothing would title them",
                descriptor.display_name
            );
        }
    }

    /// The converse fork: a remote probe with no local reader would mean the
    /// same CLI answers "what is this session called" one way on this machine
    /// and another way over ssh, with no shared predicate to keep them honest.
    #[test]
    fn a_remote_title_probe_never_ships_without_its_local_reader() {
        for descriptor in AGENT_CLIS {
            if descriptor.remote_live_store_title.is_none() {
                continue;
            }
            assert!(
                descriptor.read_live_store_title.is_some(),
                "{}: a remote title probe with no local reader is two answers to one \
                 question, free to drift",
                descriptor.display_name
            );
            assert!(
                descriptor.has_remote_arm(),
                "{}: a remote probe for a local-only CLI can never run",
                descriptor.display_name
            );
        }
    }

    /// ⛔ THE SEPARATOR IS LOAD-BEARING. The predecessor script read `argv[1]`
    /// as its single locator and `argv[2:]` as session ids, which is only safe
    /// while every wired CLI declares exactly one store glob. Antigravity
    /// declares three. A script that does not split on `--` would read a glob
    /// as a session id and answer for nothing, silently.
    #[test]
    fn every_remote_title_probe_splits_its_argv_on_the_separator() {
        for descriptor in AGENT_CLIS {
            let Some(probe) = descriptor.remote_live_store_title else {
                continue;
            };
            assert!(
                probe.script.contains("'--'"),
                "{}: its remote probe does not honour the `--` argv separator",
                descriptor.display_name
            );
            assert!(
                probe.script.contains("candidates"),
                "{}: its remote probe must return raw CANDIDATES and leave the choice \
                 to `choose`, so the title predicate is not re-encoded in Python",
                descriptor.display_name
            );
        }
    }

    /// A probe whose locators do not resolve can never run, and the failure is
    /// an empty argv rather than an error — so it is asserted here instead of
    /// being discovered as a title that never lands.
    #[test]
    fn every_remote_title_probe_resolves_at_least_one_locator() {
        for descriptor in AGENT_CLIS {
            if descriptor.remote_live_store_title.is_none() {
                continue;
            }
            let locators = descriptor.remote_store_title_locators();
            assert!(
                !locators.is_empty(),
                "{}: its remote probe resolves no locators, so the round trip would \
                 ask the remote machine about nothing",
                descriptor.display_name
            );
            for locator in &locators {
                assert!(
                    !locator.starts_with('/') && !locator.starts_with('~'),
                    "{}: locator {locator:?} must be $HOME-relative — the script expands \
                     it against the REMOTE home",
                    descriptor.display_name
                );
            }
        }
    }

    /// ⛔ A LONG PROMPT WITH A MULTI-BYTE CHARACTER ON THE CUT USED TO PANIC.
    ///
    /// The cleaner truncated with `l[..120]`, which is a BYTE index, and the
    /// string it truncates is a person's own prompt — the likeliest string in
    /// the whole title path to carry an accent or an emoji. Nothing had hit it
    /// because the sources reaching the cleaner were short curated index
    /// entries; widening the remote probe to raw transcript prompts made the
    /// latent case reachable.
    #[test]
    fn a_long_prompt_is_truncated_on_a_character_and_never_panics() {
        // The cut falls inside the two-byte character, which is the panicking
        // case, and again inside a four-byte one.
        for filler in ['e', 'x'] {
            for tail in ["é", "🙂"] {
                let prompt = format!("{}{tail} and the prompt continues", String::from(filler).repeat(119));
                let cleaned =
                    clean_agy_prompt_first_line(&prompt).expect("a plain prompt is a title");
                assert!(
                    cleaned.chars().count() <= 120,
                    "truncation must count characters: {} chars",
                    cleaned.chars().count()
                );
                assert!(
                    prompt.starts_with(&cleaned),
                    "the title must be a prefix of the prompt it came from"
                );
            }
        }
    }

    /// Write a Qwen chat store. `file_stem` is deliberately a parameter: the
    /// file name is NOT contractually the session id for this CLI, and a lookup
    /// that assumed it were would answer nothing for a store that names its
    /// files any other way.
    fn write_qwen_fixture(home: &std::path::Path, file_stem: &str, session_id: &str, title: &str) {
        let chats = home.join(".qwen/projects/-home-user-project/chats");
        std::fs::create_dir_all(&chats).expect("fixture chats directory");
        let first = serde_json::json!({ "sessionId": session_id, "cwd": "/home/user/project" });
        // The title is re-appended near EOF by this CLI, and an earlier one is
        // superseded — so the fixture carries both and the LAST must win.
        let stale = serde_json::json!({ "type": "custom_title", "title": "an older name" });
        let current = serde_json::json!({ "type": "custom_title", "title": title });
        let body = format!("{first}\n{stale}\n{{\"type\":\"message\"}}\n{current}\n");
        std::fs::write(chats.join(format!("{file_stem}.jsonl")), body).expect("fixture chat");
    }

    /// ⛔ THE LOOKUP IS THE ONLY PART THIS FUNCTION DECIDES, SO IT IS THE PART
    /// UNDER TEST. The record format comes from the two parsers that already
    /// decode this store; what is new is finding the right file for an id.
    #[test]
    fn a_live_qwen_row_is_titled_from_its_own_chat_file() {
        let home = std::env::temp_dir().join(format!("yggterm-qwen-live-{}", uuid::Uuid::new_v4()));
        let session_id = "b3d17e02-5c48-4a91-8f60-2d7c1a9e4b35";
        // Named for something OTHER than the id, so the fallback that parses the
        // first record is what has to find it.
        write_qwen_fixture(&home, "chat-0007", session_id, "Port the CSV importer");
        let found = read_qwen_live_store_title(&home, session_id);
        let named_for_the_id = {
            let other = std::env::temp_dir().join(format!("yggterm-qwen-live-{}", uuid::Uuid::new_v4()));
            write_qwen_fixture(&other, session_id, session_id, "Port the CSV importer");
            let hit = read_qwen_live_store_title(&other, session_id);
            let _ = std::fs::remove_dir_all(&other);
            hit
        };
        // ⚠ A plainly invented id that no fixture writes. Not an all-zero uuid:
        // its twelve-zero run trips the privacy guard's identity-number pattern.
        let miss = read_qwen_live_store_title(&home, "5e2a9c71-0b34-4d8f-9a16-c703e85b2d49");
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(
            found,
            Some("Port the CSV importer".to_string()),
            "a chat file not named for its session was not found, so a store that \
             names its files anything else is titled by nothing"
        );
        assert_eq!(named_for_the_id, Some("Port the CSV importer".to_string()));
        assert_eq!(miss, None, "an id with no chat file must miss, not borrow another row's title");
    }

    /// The ssh half of the same store, run the way the daemon runs it.
    #[test]
    fn a_remote_qwen_row_is_titled_by_its_probe_script() {
        let home = std::env::temp_dir().join(format!("yggterm-qwen-probe-{}", uuid::Uuid::new_v4()));
        let session_id = "b3d17e02-5c48-4a91-8f60-2d7c1a9e4b35";
        write_qwen_fixture(&home, "chat-0007", session_id, "Port the CSV importer");

        let descriptor = agent_cli_descriptor(SessionKind::QwenCode).expect("Qwen is registered");
        let probe = descriptor
            .remote_live_store_title
            .expect("Qwen declares a remote probe");
        let mut args: Vec<String> = descriptor.remote_store_title_locators();
        args.push("--".to_string());
        args.push(session_id.to_string());
        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(probe.script)
            .args(&args)
            .env("HOME", &home)
            .output()
            .expect("python3 is needed to exercise a remote store probe");
        let _ = std::fs::remove_dir_all(&home);
        assert!(
            output.status.success(),
            "the probe script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_else(|| panic!("the probe answered for nothing: {stdout:?}"));
        let value: serde_json::Value = serde_json::from_str(line).expect("JSON lines");
        assert_eq!(value["session_id"], session_id);
        let candidates: Vec<String> = value["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .map(|candidate| candidate.as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(
            (probe.choose)(&candidates),
            Some("Port the CSV importer".to_string()),
            "the re-appended title must supersede the earlier one: {candidates:?}"
        );
    }

    /// ⛔⛔ A REMOTE PROBE MUST BE ABLE TO OPEN THE SESSION'S OWN FILE.
    ///
    /// A shared index is written when the CLI gets round to it; the session's
    /// own store exists from its first turn. So a probe wired to indexes alone
    /// is blind for precisely as long as a row is new — which is the whole of
    /// the window in which a person notices an untitled row and reports it.
    /// Measured on a live Antigravity store: eight of the eight most recently
    /// touched conversations were absent from the index and present in their
    /// own transcript.
    ///
    /// ⇒ Whatever else a probe reads, it reads the store globs too.
    #[test]
    fn every_remote_title_probe_can_reach_the_sessions_own_store() {
        for descriptor in AGENT_CLIS {
            if descriptor.remote_live_store_title.is_none() {
                continue;
            }
            let locators = descriptor.remote_store_title_locators();
            for glob in descriptor.session_store_globs {
                assert!(
                    locators.iter().any(|locator| locator == glob),
                    "{}: its remote probe never opens {glob:?}, so a conversation the \
                     shared index has not caught up with is answered for by nothing — \
                     which reads exactly like a store with no title in it",
                    descriptor.display_name
                );
            }
        }
    }

    /// ⭐ The index-beside-the-store shape, derived rather than transcribed: the
    /// directory comes from the CLI's own store globs, so relocating its store
    /// moves its index with it and no second table has to agree.
    #[test]
    fn a_home_file_locator_is_resolved_under_the_clis_own_store_directory() {
        let descriptor = agent_cli_descriptor(SessionKind::Antigravity)
            .expect("Antigravity is a registered CLI");
        let home = descriptor
            .cli_home_relative()
            .expect("its store root has a parent");
        let locators = descriptor.remote_store_title_locators();
        // ⚠ Only the HOME-FILE half is anchored here. The union also carries
        // this CLI's store globs, and one of those is a legacy layout under a
        // different directory entirely — asserting over every locator would
        // make this lock a test of the store list rather than of the
        // index-beside-the-store derivation it is named for.
        let store_globs: Vec<&str> = descriptor.session_store_globs.to_vec();
        for locator in locators
            .iter()
            .filter(|locator| !store_globs.contains(&locator.as_str()))
        {
            assert!(
                locator.starts_with(&format!("{home}/")),
                "{locator:?} is not under the CLI home {home:?} the registry derives"
            );
        }
        assert!(
            locators
                .iter()
                .any(|locator| locator.ends_with("conversation_summaries.db")),
            // ⚠ Corrected once measured: the index is where an OLD title lives,
            // and it is empty for a conversation created minutes ago. It is
            // still required — it is the only place an owner's rename lands —
            // but it is the last word, not the first.
            "the summaries index carries the renamed title and must be asked: {locators:?}"
        );
    }

    /// The `choose` half, tested where it belongs — in Rust, against the same
    /// cleaner the local reader uses.
    #[test]
    fn the_title_choice_is_made_in_rust_not_in_the_remote_script() {
        let cc = agent_cli_descriptor(SessionKind::ClaudeCode)
            .and_then(|descriptor| descriptor.remote_live_store_title)
            .expect("Claude Code declares a remote probe");
        assert_eq!(
            (cc.choose)(&["  ".to_string(), "Fix the login race".to_string()]),
            Some("Fix the login race".to_string()),
            "a blank candidate must not win over a real one"
        );
        assert_eq!((cc.choose)(&[]), None);

        let agy = agent_cli_descriptor(SessionKind::Antigravity)
            .and_then(|descriptor| descriptor.remote_live_store_title)
            .expect("Antigravity declares a remote probe");
        // Its index holds whole PROMPTS, so the first candidate is a wrapper the
        // local reader strips. Answering with it verbatim would put a fenced
        // code block in the sidebar.
        assert_eq!(
            (agy.choose)(&["<USER_REQUEST>\nRefactor the CSV import\n</USER_REQUEST>".to_string()]),
            Some("Refactor the CSV import".to_string())
        );
    }

    /// Build a fixture Antigravity home and run one CLI's remote probe against
    /// it, exactly as the daemon does: locators, `--`, then ids.
    ///
    /// ⛔ **The script is the half no other test could reach.** Every lock
    /// above reads the registry — which locators resolve, that the argv
    /// separator is honoured — and every one of them was GREEN while the
    /// script read two stores that are empty for a live row. A probe is only
    /// wired when something has run it against a store shaped like the real
    /// one.
    #[test]
    fn a_codex_live_title_skips_the_instructions_wrapper() {
        // Measured 2026-08-30: a codex rollout's FIRST role:"user" item is the
        // AGENTS.md/instructions block, not the prompt. The live-title reader
        // must skip the wrapper or every codex row gets titled with the fleet
        // steer file.
        let home =
            std::env::temp_dir().join(format!("yggterm-codex-title-{}", uuid::Uuid::new_v4()));
        let dir = home.join(".codex/sessions/2026/08/30");
        std::fs::create_dir_all(&dir).unwrap();
        let id = "01a05190-b999-79d1-9f13-1c12abcdef01";
        let wrapper = r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /home/user/proj\n\n<INSTRUCTIONS>\nfleet laws live here\n</INSTRUCTIONS>"}}]}"##;
        let prompt = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the agy restart bug please"}]}}"#;
        let meta = r#"{"type":"session_meta","payload":{"id":"SESSION","cwd":"/home/user/proj"}}"#;
        std::fs::write(
            dir.join(format!("rollout-2026-08-30T12-00-00-{id}.jsonl")),
            format!("{meta}\n{wrapper}\n{prompt}\n"),
        )
        .unwrap();

        // The cache lookup consults the machine's real session-titles.db; a
        // fresh random id cannot be cached there, so the rollout arm answers.
        assert_eq!(
            read_codex_live_store_title(&home, id).as_deref(),
            Some("Fix the agy restart bug please"),
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_opencode_live_title_reads_the_v2_store_and_refuses_placeholders() {
        let home =
            std::env::temp_dir().join(format!("yggterm-oc-title-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join(".local/share/opencode")).unwrap();
        let db = home.join(".local/share/opencode/opencode.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE session_v2 (id text PRIMARY KEY, title text)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2 VALUES ('ses_ok', 'Continuing the fleet build')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2 VALUES ('ses_new', 'New session - 2026-08-30T10:00:00Z')",
            [],
        )
        .unwrap();

        assert_eq!(
            read_opencode_live_store_title(&home, "ses_ok").as_deref(),
            Some("Continuing the fleet build"),
            "opencode2 self-titles prompted sessions; the chore must read it"
        );
        assert_eq!(
            read_opencode_live_store_title(&home, "ses_new"),
            None,
            "a never-prompted session's placeholder is not a title"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_kimi_live_title_reads_the_session_state_json() {
        // kimi 0.27 moved to ~/.kimi-code and writes a real per-session
        // state.json (title = first prompt, workDir, isCustomTitle). The old
        // integration watched ~/.kimi — a home the installed CLI never
        // touches — which is why kimi rows had no store title at all.
        let home =
            std::env::temp_dir().join(format!("yggterm-kimi-title-{}", uuid::Uuid::new_v4()));
        let session_dir = home
            .join(".kimi-code/sessions/wd_user-proj_ab12cd34")
            .join("session_6c9d662b-d553-4d4e-a4f8-e10aeb810bbf");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("state.json"),
            r#"{"title":"Fix the kimi store drift","workDir":"/home/user/proj","isCustomTitle":false,"createdAt":"2026-08-30T18:32:21.620Z","updatedAt":"2026-08-30T18:40:00.000Z"}"#,
        )
        .unwrap();
        assert_eq!(
            read_kimi_live_store_title(&home, "session_6c9d662b-d553-4d4e-a4f8-e10aeb810bbf")
                .as_deref(),
            Some("Fix the kimi store drift"),
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_grok_live_title_reads_the_session_directory_summary() {
        let home =
            std::env::temp_dir().join(format!("yggterm-grok-title-{}", uuid::Uuid::new_v4()));
        let session_dir = home
            .join(".grok/sessions/%2Fhome%2Fuser%2Fproj")
            .join("01a041db-f8f6-7743-ad18-abfcbe13a9b1");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("summary.json"),
            r#"{"info":{"id":"01a041db-f8f6-7743-ad18-abfcbe13a9b1","cwd":"/home/user/proj"},"generated_title":"Parser Edge Case Hardening","session_summary":"Hardened the parser edge cases.","num_messages":2}"#,
        )
        .unwrap();
        assert_eq!(
            read_grok_live_store_title(&home, "01a041db-f8f6-7743-ad18-abfcbe13a9b1").as_deref(),
            Some("Parser Edge Case Hardening"),
            "grok's own generated title travels to the live row"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Build a fixture Antigravity home and run one CLI's remote probe against
    /// it, exactly as the daemon does: locators, `--`, then ids.
    ///
    /// ⛔ **The script is the half no other test could reach.** Every lock
    /// above reads the registry — which locators resolve, that the argv
    /// separator is honoured — and every one of them was GREEN while the
    /// script read two stores that are empty for a live row. A probe is only
    /// wired when something has run it against a store shaped like the real
    /// one.
    fn run_agy_probe(fixture_home: &std::path::Path, ids: &[&str]) -> Vec<String> {
        let descriptor =
            agent_cli_descriptor(SessionKind::Antigravity).expect("Antigravity is registered");
        let probe = descriptor
            .remote_live_store_title
            .expect("Antigravity declares a remote probe");
        let mut args: Vec<String> = descriptor.remote_store_title_locators();
        args.push("--".to_string());
        args.extend(ids.iter().map(|id| (*id).to_string()));
        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(probe.script)
            .args(&args)
            .env("HOME", fixture_home)
            .output()
            // ⛔ Loud, never skipped. A gate that passes on a machine which
            // cannot run it reports the same thing as a gate that passed.
            .expect("python3 is needed to exercise a remote store probe");
        assert!(
            output.status.success(),
            "the probe script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.to_string())
            .filter(|line| !line.trim().is_empty())
            .collect()
    }

    /// Write the three Antigravity stores. `index` and `history` are optional
    /// so a test can reproduce the shape a fresh conversation actually has.
    fn write_agy_fixture(
        home: &std::path::Path,
        conversation_id: &str,
        transcript_prompt: &str,
        index: Option<(&str, &str)>,
        history: Option<&str>,
    ) {
        let cli_home = home.join(".gemini/antigravity-cli");
        let logs = cli_home
            .join("brain")
            .join(conversation_id)
            .join(".system_generated/logs");
        std::fs::create_dir_all(&logs).expect("fixture transcript directory");
        let record = serde_json::json!({
            "type": "USER_INPUT",
            "content": transcript_prompt,
        });
        std::fs::write(logs.join("transcript_full.jsonl"), format!("{record}\n"))
            .expect("fixture transcript");

        let db = cli_home.join("conversation_summaries.db");
        let connection = rusqlite::Connection::open(&db).expect("fixture index");
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS conversation_summaries \
                 (conversation_id TEXT, title TEXT, preview TEXT, workspace_uris TEXT);",
                [],
            )
            .expect("fixture index schema");
        if let Some((title, preview)) = index {
            connection
                .execute(
                    "INSERT INTO conversation_summaries \
                     (conversation_id, title, preview, workspace_uris) VALUES (?1, ?2, ?3, '');",
                    rusqlite::params![conversation_id, title, preview],
                )
                .expect("fixture index row");
        }
        drop(connection);

        let history_line = history
            .map(|display| {
                serde_json::json!({ "conversationId": conversation_id, "display": display })
                    .to_string()
            })
            .unwrap_or_else(|| {
                serde_json::json!({ "conversationId": "another-conversation", "display": "x" })
                    .to_string()
            });
        std::fs::write(cli_home.join("history.jsonl"), format!("{history_line}\n"))
            .expect("fixture history");
    }

    /// ⛔⛔ THE MEASUREMENT THAT MOVED THIS PROBE. On a live Antigravity store,
    /// of the eight most recently touched conversations **zero** had a row in
    /// `conversation_summaries.db` and six had no `history.jsonl` entry — while
    /// all eight carried a usable prompt in their own transcript.
    ///
    /// A probe reading only the two shared files therefore answers
    /// `no_title_in_store` for exactly the rows a person is looking at, which
    /// is indistinguishable from the defect it was written to repair. This is
    /// that row, and it must come back titled.
    #[test]
    fn a_fresh_remote_agy_conversation_is_titled_from_its_own_transcript() {
        let home = std::env::temp_dir().join(format!("yggterm-agy-probe-{}", uuid::Uuid::new_v4()));
        let conversation_id = "4f0d5a2b-1c73-4c8e-9f21-6b0a7d3e5c14";
        write_agy_fixture(
            &home,
            conversation_id,
            "<USER_REQUEST>\nRewrite the invoice exporter\n</USER_REQUEST>",
            None,
            None,
        );
        let lines = run_agy_probe(&home, &[conversation_id]);
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(
            lines.len(),
            1,
            "the conversation exists only as a transcript and was answered for by nothing: \
             {lines:?}"
        );
        let value: serde_json::Value =
            serde_json::from_str(&lines[0]).expect("the probe answers in JSON lines");
        assert_eq!(value["session_id"], conversation_id);
        let candidates: Vec<String> = value["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .map(|candidate| candidate.as_str().unwrap_or_default().to_string())
            .collect();
        let probe = agent_cli_descriptor(SessionKind::Antigravity)
            .and_then(|descriptor| descriptor.remote_live_store_title)
            .expect("Antigravity declares a remote probe");
        assert_eq!(
            (probe.choose)(&candidates),
            Some("Rewrite the invoice exporter".to_string()),
            "the transcript prompt did not survive the same cleaner the local reader uses"
        );
    }

    /// The precedence half: where the shared index HAS answered, it still wins.
    /// The transcript is the fallback that was missing, not a new authority —
    /// promoting it would make a remote row's title differ from the same row's
    /// title read locally, which is the drift the two halves exist to avoid.
    #[test]
    fn an_indexed_agy_conversation_still_prefers_the_index_title() {
        let home = std::env::temp_dir().join(format!("yggterm-agy-probe-{}", uuid::Uuid::new_v4()));
        let conversation_id = "8c1e7f40-92ab-4d55-bb03-1e6f2a9c4d77";
        write_agy_fixture(
            &home,
            conversation_id,
            "<USER_REQUEST>\nthe transcript prompt\n</USER_REQUEST>",
            Some(("Renamed by the owner", "an auto summary")),
            Some("a history display"),
        );
        let lines = run_agy_probe(&home, &[conversation_id]);
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(lines.len(), 1, "{lines:?}");
        let value: serde_json::Value = serde_json::from_str(&lines[0]).expect("JSON lines");
        let candidates: Vec<String> = value["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .map(|candidate| candidate.as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("Renamed by the owner"),
            "the local reader's precedence is index title, index preview, history, \
             transcript — the remote answer must be ordered the same way: {candidates:?}"
        );
        assert!(
            candidates.iter().any(|candidate| candidate.contains("the transcript prompt")),
            "the transcript must still be offered as the last fallback: {candidates:?}"
        );
    }

    /// Tier ids are unique within a CLI — they address a tier from a verb and a
    /// test, and a duplicate would make `--preset skip` ambiguous.
    #[test]
    fn permission_preset_ids_are_unique_within_a_cli() {
        for descriptor in AGENT_CLIS {
            let mut seen = std::collections::BTreeSet::new();
            for preset in descriptor.permission_presets {
                assert!(
                    seen.insert(preset.id),
                    "{}: two permission presets share the id {:?}",
                    descriptor.display_name,
                    preset.id
                );
            }
        }
    }

    /// ⛔⛔ THE ONE THAT KEEPS THIS FROM BECOMING A SECOND ENCODING OF A SECURITY
    /// BOUNDARY. The modal's tiers and `--permission-mode`'s mapping are two
    /// vocabularies for the same postures — one aimed at a human, one at a
    /// delegate — and they must not be able to disagree about what BYPASS means.
    ///
    /// The invariant is that the posture `--permission-mode bypass` produces is
    /// OFFERED in the modal — not that it is the pre-populated one. Those are
    /// different questions and conflating them is what the first draft did:
    /// codex's least-checks tier is `-s danger-full-access` (no sandbox, prompts
    /// still asked) while its bypass mode is
    /// `--dangerously-bypass-approvals-and-sandbox` (no sandbox AND no prompts),
    /// and those are two real postures, so the modal shows both.
    ///
    /// ⭐ The lock earned its place immediately: it caught that codex was
    /// offering only one of the two, so a user reading the modal could not
    /// reach the posture a delegate launch could ask for by name.
    #[test]
    fn every_bypass_mode_is_offered_as_a_tier() {
        for descriptor in AGENT_CLIS {
            let Some(bypass) = descriptor
                .permission_modes
                .iter()
                .find(|(mode, _)| *mode == AgentPermissionMode::Bypass)
                .map(|(_, tokens)| *tokens)
            else {
                continue;
            };
            if descriptor.permission_presets.is_empty() {
                continue;
            }
            let offered = descriptor.permission_presets.iter().any(|preset| {
                let tier_tokens = preset.args.split_whitespace().collect::<Vec<_>>();
                bypass.iter().all(|token| tier_tokens.contains(token))
            });
            assert!(
                offered,
                "{}: --permission-mode bypass emits {bypass:?}, and no tier in the modal \
                 carries it. Two answers to what bypass means for this CLI is the SSOT \
                 violation this repo's own law forbids.",
                descriptor.display_name,
            );
        }
    }

    /// A tier's args must be RUNNABLE AS WRITTEN. A placeholder like
    /// `--tools <names>` pasted into a launch is a launch that dies at the PTY,
    /// and the first draft of pi's row shipped exactly that.
    #[test]
    fn no_permission_preset_ships_a_placeholder_for_a_value() {
        for descriptor in AGENT_CLIS {
            for preset in descriptor.permission_presets {
                assert!(
                    !preset.args.contains('<') && !preset.args.contains('>'),
                    "{} / {}: preset args {:?} carry a placeholder. Ship a complete flag and \
                     name the parameterised one in the explanation instead.",
                    descriptor.display_name,
                    preset.id,
                    preset.args
                );
            }
        }
    }

    /// Every explanation is a SENTENCE in the CLI's own vocabulary, because it
    /// is the whole reason the modal beats nine text boxes.
    #[test]
    fn every_permission_preset_explains_itself() {
        for descriptor in AGENT_CLIS {
            for preset in descriptor.permission_presets {
                assert!(
                    preset.explanation.split_whitespace().count() >= 5,
                    "{} / {}: an explanation shorter than a sentence explains nothing",
                    descriptor.display_name,
                    preset.id
                );
                assert!(
                    !preset.label.trim().is_empty(),
                    "{} / {}: a tier needs a button label",
                    descriptor.display_name,
                    preset.id
                );
            }
        }
    }

    /// Every `extra_args_slug` names a REGISTERED CLI's slug. A typo here would
    /// silently give one CLI a box nothing else reads or writes.
    #[test]
    fn every_extra_args_slug_names_a_registered_cli() {
        for descriptor in AGENT_CLIS {
            assert!(
                AGENT_CLIS
                    .iter()
                    .any(|other| other.slug == descriptor.extra_args_slug),
                "{}: extra_args_slug {:?} is not any registered CLI's slug",
                descriptor.display_name,
                descriptor.extra_args_slug
            );
            // …and a CLI that borrows a box must borrow from one that OWNS its
            // own, never from another borrower — one hop, no chains to resolve.
            if !descriptor.owns_its_extra_args_box() {
                let owner = AGENT_CLIS
                    .iter()
                    .find(|other| other.slug == descriptor.extra_args_slug)
                    .expect("checked above");
                assert!(
                    owner.owns_its_extra_args_box(),
                    "{} borrows {}'s box, which is itself borrowed",
                    descriptor.display_name,
                    owner.display_name
                );
            }
        }
    }

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

    /// The metadata-rail label is the display name plus one word, and nothing
    /// else — so the field that has to be transcribed (a `&'static str` cannot
    /// be `format!`ed) cannot drift from the name it is made of.
    ///
    /// ⚠ Not cosmetic. `"Codex Session"` and `"Claude Code Session"` are READ by
    /// predicates that recover a row's session id from its rail, so a label that
    /// drifted from its CLI would make those rows unidentifiable — silently, and
    /// only for the CLI whose spelling moved.
    #[test]
    fn the_session_metadata_label_is_the_display_name_plus_session() {
        for descriptor in AGENT_CLIS {
            assert_eq!(
                descriptor.session_metadata_label,
                format!("{} Session", descriptor.display_name),
                "{:?}: the rail label must be its display name plus \" Session\"",
                descriptor.kind,
            );
        }
    }

    /// Relative luminance, per WCAG 2.1.
    fn relative_luminance(hex: &str) -> f64 {
        let hex = hex.trim_start_matches('#');
        let channel = |offset: usize| {
            let raw = u8::from_str_radix(&hex[offset..offset + 2], 16)
                .expect("a brand colour must be six hex digits") as f64
                / 255.0;
            if raw <= 0.03928 {
                raw / 12.92
            } else {
                ((raw + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
    }

    /// ⛔ EVERY KIND IS BORN NAMING ITS MACHINE, and none is born naming a path.
    ///
    /// The defect this locks out, owner-reported 2026-08-21 with a screenshot:
    /// a `ytop` started in a terminal was called `/home/user/proj` — its working
    /// directory, in full — while an agent CLI beside it was called
    /// `New Antigravity Session`, with no machine in it at all. Two families of
    /// row, two rules, and neither one said which of three machines it was on.
    ///
    /// Driven off `SessionKind::ALL`, so a kind added later cannot quietly opt
    /// out of the rule by not being in whichever list a reviewer remembered.
    #[test]
    fn every_kind_is_born_named_after_its_machine() {
        for kind in SessionKind::ALL {
            let title = new_session_birth_title(*kind, Some("box"));
            assert!(
                title.starts_with("New box "),
                "{kind:?} is born `{title}`, which does not name its machine",
            );
            assert!(
                !title.contains('/') && !title.contains('\\'),
                "{kind:?} is born `{title}`, which reads as a path rather than a name",
            );
            let noun = session_kind_birth_noun(*kind);
            assert!(
                !noun.trim().is_empty(),
                "{kind:?} has no noun for the birth rule",
            );
            assert_eq!(
                title,
                format!("New box {noun}"),
                "{kind:?}: the birth title must be the rule applied to its noun",
            );
        }
    }

    /// A CLI's birth noun is its DISPLAY NAME, from the registry — never its
    /// slug, its binary, or a product name invented at a call site.
    #[test]
    fn an_agent_clis_birth_noun_is_its_display_name() {
        for descriptor in AGENT_CLIS {
            assert_eq!(
                session_kind_birth_noun(descriptor.kind),
                descriptor.display_name,
                "{:?} is born under a name the registry does not know it by",
                descriptor.kind,
            );
        }
        // The two shells share one noun: the machine half already says which
        // host the row is on, so "SSH Terminal" would say it twice.
        assert_eq!(session_kind_birth_noun(SessionKind::Shell), "Terminal");
        assert_eq!(session_kind_birth_noun(SessionKind::SshShell), "Terminal");
    }

    /// ⛔ WHAT THE COMPOSER MAKES, THE RECOGNISER MUST SEE — for every kind, with
    /// and without a machine.
    ///
    /// This is the trap the birth-name change would otherwise have set. Every
    /// "may this row still be re-titled" gate runs through
    /// `looks_like_generated_fallback_title`, which matched a table of literal
    /// placeholder strings. Put a machine name in the middle of one and the
    /// literal stops matching — so a row born two seconds ago reads as already
    /// titled and keeps its placeholder for ever. Trading a name with no machine
    /// in it for a name that is never replaced is not a fix.
    #[test]
    fn a_birth_title_is_recognised_as_the_placeholder_it_is() {
        for kind in SessionKind::ALL {
            for machine in [None, Some("box"), Some("build-01")] {
                let title = new_session_birth_title(*kind, machine);
                assert!(
                    is_new_row_birth_title(&title),
                    "`{title}` is a birth name this build composes and must read \
                     as a placeholder",
                );
                assert!(
                    crate::looks_like_generated_fallback_title(&title),
                    "`{title}` must stay replaceable by the title chore",
                );
            }
        }
        // An app row's birth name too — the same composer, a label the app
        // registry supplies rather than a kind.
        assert!(is_new_row_birth_title(&new_row_birth_title(
            Some("box"),
            "Terminal"
        )));
    }

    /// ⚠ And it must not swallow a REAL title that happens to start with "New".
    /// The rule is `New ` + at most one machine token + a noun this build
    /// composes with; anything longer is somebody's sentence.
    #[test]
    fn a_real_title_starting_with_new_is_not_mistaken_for_a_birth_name() {
        for title in [
            "New retry budget for the fetch loop",
            "New terminal geometry is wrong after a resize",
            "Newton solver diverges",
            "Rewrite the new terminal path",
            // ⚠ The near miss the case rule exists for: same shape, but the
            // composer would never spell the noun in lower case.
            "New async terminal",
            "New two word Terminal",
            "New",
            "",
        ] {
            assert!(
                !is_new_row_birth_title(title),
                "`{title}` is a real title and must not be discarded as a placeholder",
            );
        }
    }

    /// A caller that cannot name the host degrades to `New {what}` — never an
    /// empty gap, never the word "unknown".
    #[test]
    fn a_nameless_machine_leaves_no_gap_in_the_birth_title() {
        for machine in [None, Some(""), Some("   ")] {
            assert_eq!(
                new_row_birth_title(machine, "Terminal"),
                "New Terminal",
                "a blank machine must not leave a hole in the row's name",
            );
        }
        assert_eq!(new_row_birth_title(Some(" box "), "Ytop"), "New box Ytop");
    }

    /// ⛔ A [`TitleAuthority::Store`] CLI has NO other way to be titled —
    /// yggterm refuses to generate copy for one on purpose — so one that also
    /// declares no `read_live_store_title` can never be named at all, and its
    /// rows wear their birth title for the life of the session.
    ///
    /// ⭐ **It was a REPORT and is now a PROHIBITION, because both holes it
    /// named have been closed** (2026-08-21) and they closed in opposite
    /// directions, which is the point:
    ///
    /// * one CLI's store WAS measured — two functions in this file already
    ///   decoded its layout — so `None` was claiming unmeasured about a store
    ///   the registry could read. It got a reader.
    /// * the other declared `Store` over a store the registry cannot locate at
    ///   all (`session_store_globs` empty). It was not owed a reader; it was
    ///   owed an honest authority, and now generates.
    ///
    /// ⇒ Leaving the hook `None` is still the honest answer for an unmeasured
    /// store — but then the CLI must not ALSO claim its store is authoritative,
    /// because the two together are what make a row unnameable.
    #[test]
    fn a_store_titled_cli_without_a_live_reader_can_never_be_titled() {
        let unreachable = AGENT_CLIS
            .iter()
            .filter(|descriptor| {
                descriptor.title_is_store_authoritative()
                    && descriptor.read_live_store_title.is_none()
            })
            .map(|descriptor| descriptor.slug)
            .collect::<Vec<_>>();
        assert!(
            unreachable.is_empty(),
            "a CLI is store-authoritative with no reader, so its rows can never \
             be titled at all: {unreachable:?} — either measure its store and \
             wire a reader, or stop claiming its store is authoritative",
        );
    }

    /// Every brand colour carries WHITE text, so every brand colour owes AA.
    ///
    /// ⛔ **The failure this locks out has already shipped once.** The start
    /// page's open button painted Claude Code `#d97706` under a white label —
    /// 3.19:1, which fails AA for normal text — and nothing in the codebase
    /// could notice, because the colour was a literal inside a `match` arm and
    /// contrast was nobody's field. A brand colour is a legibility decision
    /// before it is a decorative one; "nearest available" is licensed for the
    /// HUE, never for the contrast.
    #[test]
    fn the_brand_colours_clear_wcag_aa_against_white() {
        let white = relative_luminance("#ffffff");
        for descriptor in AGENT_CLIS {
            assert_eq!(
                descriptor.brand_color.len(),
                7,
                "{:?}: a brand colour is a `#rrggbb` literal",
                descriptor.kind,
            );
            let brand = relative_luminance(descriptor.brand_color);
            let ratio = (white + 0.05) / (brand + 0.05);
            assert!(
                ratio >= 4.5,
                "{:?}: {} contrasts {ratio:.2}:1 against white, below the 4.5:1 \
                 AA floor for normal text — darken it and keep the hue",
                descriptor.kind,
                descriptor.brand_color,
            );
        }
    }

    /// Two CLIs painting the same colour is the same defect as no colour at
    /// all: the mark stops carrying information the moment it is ambiguous.
    #[test]
    fn no_two_clis_share_a_brand_colour() {
        for (ix, descriptor) in AGENT_CLIS.iter().enumerate() {
            for other in AGENT_CLIS.iter().skip(ix + 1) {
                assert_ne!(
                    descriptor.brand_color, other.brand_color,
                    "{:?} and {:?} both paint {} — a shared colour identifies neither",
                    descriptor.kind, other.kind, descriptor.brand_color,
                );
            }
        }
    }

    /// The open verb NAMES the CLI, for every registered CLI, with no arm that
    /// falls back to a bare "Open" — that fallback is reserved for the rows
    /// that genuinely have no CLI.
    #[test]
    fn the_open_verb_names_every_registered_cli() {
        for descriptor in AGENT_CLIS {
            assert_eq!(
                agent_cli_open_session_label(Some(descriptor.kind)),
                format!("Open this {} Session", descriptor.display_name),
                "{:?}: the open verb must name the CLI",
                descriptor.kind,
            );
        }
        assert_eq!(agent_cli_open_session_label(None), "Open");
    }

    /// A launch refused for a missing binary shows this sentence, so every CLI
    /// owes one that actually points somewhere.
    ///
    /// ⛔ The failure it guards: the refusal is the ONLY thing the user sees when
    /// a CLI is absent (owner-reported 2026-08-08, Muse Code). A descriptor whose
    /// instruction did not name its package, its URL or "by hand" would refuse a
    /// launch and leave the user with nothing to do about it — which is the
    /// silent `/bin/bash` failure again, wearing an error message.
    #[test]
    fn every_cli_says_how_it_is_installed() {
        for descriptor in AGENT_CLIS {
            let instruction = descriptor.install_instruction();
            let names_its_source = match descriptor.install {
                CliInstall::Npm(package) | CliInstall::Uv(package) => {
                    instruction.contains(package)
                }
                CliInstall::VendorScript(url) => instruction.contains(url),
                CliInstall::Manual => instruction.contains(descriptor.binary_name),
            };
            assert!(
                names_its_source,
                "{}: install_instruction must name what it declares in CliInstall, got {instruction:?}",
                descriptor.display_name
            );
        }

        // Muse is the owner-gated case the report came from: closed source,
        // installed nowhere on the fleet, and the ONE thing yggterm knows is the
        // vendor installer — which, since the owner's 2026-08-08 ruling, it RUNS.
        //
        // ⛔ This assertion used to demand the words "never runs that unattended".
        // Inverted deliberately: the sentence a user reads is the only place the
        // superseded refusal could survive, so the test now FAILS if it comes back.
        let muse = agent_cli_descriptor(SessionKind::Muse).unwrap();
        let muse_instruction = muse.install_instruction();
        assert!(muse_instruction.contains("https://dev.meta.ai/install.sh"));
        assert!(
            !muse_instruction.contains("never runs that unattended"),
            "the vendor-script refusal is superseded and must not be re-stated: \
             {muse_instruction:?}"
        );

        // Every CLI yggterm provisions tells the user to WAIT rather than to
        // install by hand, because the attach that produced this refusal also
        // kicked the background provision. npm, uv and vendor-script alike —
        // that parity IS the ruling.
        for descriptor in AGENT_CLIS {
            let instruction = descriptor.install_instruction();
            if descriptor.install.provisions_unattended() {
                assert!(
                    instruction.contains("retry in a moment"),
                    "{}: a CLI yggterm provisions must tell the user to wait, got \
                     {instruction:?}",
                    descriptor.display_name
                );
            } else {
                assert!(
                    !instruction.contains("retry in a moment"),
                    "{}: a CLI yggterm does NOT provision must not promise an install \
                     is in flight, got {instruction:?}",
                    descriptor.display_name
                );
            }
        }
    }

    /// Arrival and staying-current are separate questions, and the registry must
    /// answer both for every CLI.
    ///
    /// ⛔ The failure this guards: the owner ruled that yggterm auto-installs AND
    /// auto-updates every CLI on every host. Before this, "can yggterm install
    /// it" was the only question asked, so Antigravity — which yggterm genuinely
    /// cannot fetch — was written off entirely, and the `agy update` its own
    /// `--help` advertises was never run. A CLI that cannot be installed can
    /// still be updated, and the registry has to be able to say so.
    #[test]
    fn a_cli_yggterm_cannot_install_can_still_be_updated() {
        let antigravity = agent_cli_descriptor(SessionKind::Antigravity).unwrap();
        assert!(
            !antigravity.install.provisions_unattended(),
            "agy is served behind a sign-in; yggterm cannot fetch it"
        );
        assert_eq!(
            antigravity.update,
            CliUpdate::SelfCommand(&["update"]),
            "agy updates itself — measured from its own --help"
        );

        // The converse: an npm CLI has no self-updater to prefer, so its refresh
        // is the install method run again.
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert!(codex.install.provisions_unattended());
        assert_eq!(codex.update, CliUpdate::Reinstall);

        // ⛔ No CLI may be BOTH unfetchable and unupdatable — that is a row in
        // the registry yggterm can do nothing for, and it must be noticed at
        // build time rather than by a user whose CLI silently rots.
        for descriptor in AGENT_CLIS {
            if !descriptor.install.provisions_unattended() {
                assert!(
                    matches!(descriptor.update, CliUpdate::SelfCommand(argv) if !argv.is_empty()),
                    "{}: yggterm can neither install nor update this CLI",
                    descriptor.display_name
                );
            }
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

    /// The fleet's transcript table names the same stores the registry does.
    ///
    /// ⛔ **WHY A TABLE EXISTS OUTSIDE THIS FILE AT ALL.** The orchestration verbs
    /// need one thing the registry does not publish: **where in a store path the
    /// session id sits.** They had been answering it by slicing a row's address,
    /// which is right only for a live `scheme://host/<id>` row — so of 281 rows
    /// measured on a live plane, 246 were called something that was not their id
    /// and five collapsed onto one shared name.
    ///
    /// ⚖ The registry stays the owner of WHERE A STORE IS. The table adds only the
    /// id position, and this test is the join: transcribing `session_store_globs`
    /// into it is safe precisely because a drift fails here rather than silently
    /// in a watchdog at three in the morning.
    ///
    /// ⛔ It also refuses a template that does not point INTO the store it claims,
    /// because a plausible-looking path is exactly what this area keeps shipping —
    /// a probe that reads the wrong file answers "nothing here", which is the same
    /// answer as the defect it was written to repair.
    #[test]
    /// ⛔⛔ A ROW SITTING AT A STARTUP GATE MUST NEVER CLASSIFY AS TYPEABLE.
    ///
    /// Measured live 2026-08-22, and the chain was watched end to end: codex
    /// declared NO gate phrases, so its trust prompt classified as `ready` with
    /// `may_type: true`; a delivery verb submitted into it; a picker consumes
    /// navigation keys and one option is `No, quit`; the CLI exited; the daemon
    /// relaunched it; the fresh process came up at the same gate reporting `idle`.
    /// Every brief aimed at that row was eaten, and nothing reported a failure.
    ///
    /// ⚠ The screens below are REAL captures with the directory replaced by an
    /// invented one — the gate's wording is what is under test, and the path is
    /// the one part of it that identifies nothing.
    #[test]
    fn a_startup_gate_is_recognised_for_every_cli_that_declares_one() {
        const CODEX_GATE: &str = "\
> You are in /home/example/work
  Do you trust the contents of this directory? Working with untrusted contents comes with higher risk of
  prompt injection. Trusting the directory allows project-local config, hooks, and exec policies to load.
› 1. Yes, continue
  2. No, quit
  Press enter to continue";
        const CLAUDE_GATE: &str = "\
 Accessing workspace:
 /home/example/work
 Quick safety check: Is this a project you created or one you trust?
 ❯ 1. Yes, I trust this folder
   2. No, exit
 Enter to confirm · Esc to cancel";
        // An ordinary working screen of each, which must NOT trip the gate.
        const CODEX_IDLE: &str = "\
› Run /review on my current changes
  gpt-5.6-terra medium · ~/work";
        const CLAUDE_IDLE: &str = "\
❯ Try \"write a test for <filepath>\"
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← 1 agent";

        for (kind, gate, idle) in [
            (SessionKind::Codex, CODEX_GATE, CODEX_IDLE),
            (SessionKind::ClaudeCode, CLAUDE_GATE, CLAUDE_IDLE),
        ] {
            let d = agent_cli_descriptor(kind).expect("registered");
            assert!(
                !d.startup_gate_screen_phrases.is_empty(),
                "{} declares no startup gate — EMPTY means UNMEASURED here, and an \
                 unrecognised gate classifies as typeable",
                d.slug
            );
            assert!(
                d.screen_shows_startup_gate(&gate.to_lowercase()),
                "{}'s own trust gate was not recognised from its real screen",
                d.slug
            );
            assert!(
                !d.screen_shows_startup_gate(&idle.to_lowercase()),
                "{}'s ordinary composer screen was mistaken for a startup gate — a \
                 gate that fires on an idle row makes every row untypeable",
                d.slug
            );
        }
    }

    /// ⛔ The composer marker is a MEASUREMENT, and seven of ten descriptors carry
    /// the same `❯`, which is how an assumed default hides among real ones. muse
    /// draws U+27E9 and was declared U+276F, so its rows reported
    /// `consuming_input: false` forever and delivery waited out every timeout —
    /// the same failure a hardcoded `›` caused for Claude Code on 2026-08-06.
    #[test]
    fn a_composer_marker_is_the_glyph_that_cli_actually_draws() {
        // Real composer lines, captured 2026-08-22. Paths invented.
        for (kind, line) in [
            (SessionKind::Muse, "\u{27e9}"),
            (SessionKind::Codex, "\u{203a} Run /review on my current changes"),
            (SessionKind::ClaudeCode, "\u{276f} Try \"write a test\""),
        ] {
            let d = agent_cli_descriptor(kind).expect("registered");
            assert!(
                line.starts_with(d.composer_marker),
                "{} declares composer_marker {:?} but its real composer line starts \
                 {:?} — the readiness probe finds no composer at all and the row \
                 reports never-ready forever",
                d.slug,
                d.composer_marker,
                line.chars().next().unwrap()
            );
        }
    }

    fn the_fleet_transcript_table_matches_the_registry() {
        let raw = include_str!(
            "../../../.agents/skills/yggterm-agent-fleet/cli-stores.json"
        );
        let table: serde_json::Value =
            serde_json::from_str(raw).expect("cli-stores.json is not valid JSON");
        let clis = table["clis"].as_object().expect("cli-stores.json has no `clis` map");

        for descriptor in AGENT_CLIS.iter() {
            let entry = clis.get(descriptor.slug).unwrap_or_else(|| {
                panic!(
                    "{} is a registered CLI with no row in cli-stores.json — the fleet \
                     verbs will find no transcript for it and read that as \"this row \
                     has never done anything\"",
                    descriptor.slug
                )
            });

            // ⛔⛔ THE SPELLING A ROW REPORTS, RATIFIED BY THE OWNER OF THAT
            //    QUESTION. `icon_kind` is not always the slug — a codex row wears
            //    the historical `session` — and the fleet's resolver matched a
            //    row's kind against the table's KEYS, so it never narrowed for
            //    codex: 410 of 742 live rows on 2026-08-22. It was right only
            //    because ids are unique across stores, and that luck runs out the
            //    moment a caller needs to know WHETHER IT LOOKED, which the reap
            //    does before destroying a row.
            let icon_kind = entry["icon_kind"].as_str().unwrap_or_else(|| {
                panic!(
                    "{} has no `icon_kind` in cli-stores.json - without it the fleet \
                     cannot recognise its own rows by the kind they report",
                    descriptor.slug
                )
            });
            // ⛔ Compared against the PRODUCER, not against mere resolvability.
            //    The first draft asked only whether `session_kind_for_row` could
            //    map the alias back, and `"codex"` passed that - the exact wrong
            //    guess whose absence caused the defect, because the slug arm
            //    resolves it too. A lock that accepts the wrong answer is not one.
            assert_eq!(
                Some(icon_kind),
                row_icon_kind(descriptor.kind),
                "cli-stores.json spells {}'s row kind {icon_kind:?}, but a row of \
                 that kind reports {:?} - the spelling a row reports has one owner \
                 and this is not a place to restate it",
                descriptor.slug,
                row_icon_kind(descriptor.kind)
            );

            let recorded: Vec<&str> = entry["store_globs"]
                .as_array()
                .expect("store_globs must be an array")
                .iter()
                .map(|value| value.as_str().expect("a glob must be a string"))
                .collect();
            assert_eq!(
                recorded, descriptor.session_store_globs,
                "cli-stores.json has drifted from the registry for {} — the registry \
                 owns where a store is, so fix the JSON, not this test",
                descriptor.slug
            );

            let template = entry["transcript"].as_str();
            if descriptor.session_store_globs.is_empty() {
                assert!(
                    template.is_none(),
                    "{} declares no store, so a transcript template for it is a guess \
                     wearing the costume of a measurement",
                    descriptor.slug
                );
                continue;
            }
            let template = template.unwrap_or_else(|| {
                panic!(
                    "{} declares a store but no transcript template, so the fleet \
                     cannot tell one of its sessions from another",
                    descriptor.slug
                )
            });
            assert!(
                template.contains("{id}"),
                "{}'s template names no session id, so it cannot address one session: \
                 {template}",
                descriptor.slug
            );
            // ⛔ Substituting a real-shaped id must land INSIDE this CLI's own store,
            //    judged by the registry's own predicate rather than by string eyeballing.
            let sample = template.replace("{id}", "00000000-1111-4222-8333-444444444444");
            let absolute = format!("/home/someone/{sample}");
            assert!(
                descriptor.store_path_is_under_root(&absolute),
                "{}'s transcript template points outside the store the registry \
                 declares for it: {absolute}",
                descriptor.slug
            );
        }
    }

    #[test]
    fn every_agent_cli_declares_a_store() {
        for descriptor in AGENT_CLIS {
            // A CLI may legitimately have no scannable store — opencode keeps
            // one SQLite DB, kimi buckets by MD5 of the cwd, Muse is not
            // installed anywhere yet. What is forbidden is SILENCE: the gap
            // must be declared, with the specific obstacle, so the next session
            // closes it instead of rediscovering it.
            // OpenCode and Kimi have empty globs but a dedicated scanner hook in
            // scan_all_durable_sessions (opencode DB, kimi MD5 buckets) — they are
            // scanned despite empty globs, so no gap is required. That exemption is
            // asked of `kind_has_dedicated_scanner`, the one owner of the question,
            // so this test and the `ls` warnings cannot drift apart from the
            // scanner dispatch the way they had by 2026-08-20.
            if descriptor.session_store_globs.is_empty()
                && !crate::startpage::kind_has_dedicated_scanner(descriptor.kind)
            {
                let gap = descriptor.store_scan_gap.unwrap_or_else(|| {
                    panic!(
                        "{:?} declares no store globs and no store_scan_gap — say \
                         which it is",
                        descriptor.kind
                    )
                });
                assert!(
                    gap.len() > 80,
                    "{:?}: a store_scan_gap must name the obstacle, not wave at it",
                    descriptor.kind
                );
                continue;
            }
            assert!(
                descriptor.store_scan_gap.is_none(),
                "{:?} declares BOTH store globs and a scan gap",
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

    // ⭐ THE SCREEN THIS ASSERTS IS MEASURED, not imagined: a real `claude` was
    // driven in a pty until it raised a one-question picker, and these are the
    // lines it painted. The footer is the discriminator — the options above it
    // are ordinary numbered text that any conversation could contain.
    #[test]
    fn a_claude_code_question_picker_is_recognised_from_its_footer() {
        let descriptor = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        let single_question = "\
 ☐ Next step  How would you like to proceed?\n\
 ❯ 1. Continue   Proceed with the current approach.\n\
   2. Stop here  Halt and wait for further instructions.\n\
   3. Type something.\n\
   4. Chat about this\n\
 Enter to select · ↑/↓ to navigate · Esc to cancel\n";
        assert!(descriptor.screen_shows_question_picker(single_question));

        // The multi-question spelling swaps the middle chord for a literal.
        let many_questions = " Enter to select · Tab/Arrow keys to navigate · Esc to cancel\n";
        assert!(descriptor.screen_shows_question_picker(many_questions));

        // The review step paints no navigate footer at all.
        assert!(descriptor.screen_shows_question_picker("  Ready to submit your answers?\n"));

        // The CLI's generic select list — menus and permission prompts — eats
        // typed text exactly the same way, so it is the same state.
        assert!(descriptor.screen_shows_question_picker("   (Use arrow keys)\n"));
        assert!(descriptor
            .screen_shows_question_picker("   (Use arrow keys to reveal more choices)\n"));
    }

    // The guard that keeps prose out: the needle alone is common English, so
    // the neighbours must be on the SAME line before the state arms.
    #[test]
    fn prose_that_merely_mentions_navigating_does_not_arm_the_picker() {
        let descriptor = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert!(!descriptor.screen_shows_question_picker(
            "  I will use the arrow keys to navigate the file tree next.\n"
        ));
        assert!(!descriptor.screen_shows_question_picker(
            "  esc to interrupt · 12s · 340 tokens\n"
        ));
        assert!(!descriptor.screen_shows_question_picker(""));
    }

    // ⛔ EMPTY MEANS UNMEASURED and must stay visibly empty: a CLI whose picker
    // nobody has looked at folds into the old two-state reading rather than
    // guessing, and the gap is where the next session can see it.
    #[test]
    fn an_unmeasured_cli_has_no_picker_phrases_and_never_arms() {
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert!(codex.question_picker_screen_phrases.is_empty());
        assert!(!codex.screen_shows_question_picker(
            " Enter to select · ↑/↓ to navigate · Esc to cancel\n"
        ));
    }

    // ⭐ MEASURED from the same pty run as the picker screens. The composer is
    // EMPTY on every line asserted here — that is the whole point.
    #[test]
    fn a_background_agent_hint_is_chrome_and_not_a_typed_draft() {
        let descriptor = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert!(descriptor.screen_shows_background_agent_hint("\u{276f}   \u{b7} \u{2190} 1 agent\n"));
        assert!(descriptor.screen_shows_background_agent_hint(
            " \u{23f5}\u{23f5} bypass permissions on (shift+tab to cycle) \u{b7} \u{2190} for agents\n"
        ));
        // Plural, because the count varies and the wording around it does not.
        assert!(descriptor.screen_shows_background_agent_hint("\u{276f}  \u{2190} 3 agents\n"));
    }

    // Either half alone is ordinary text; only the pair on ONE line is chrome.
    #[test]
    fn prose_about_agents_does_not_read_as_the_background_hint() {
        let descriptor = agent_cli_descriptor(SessionKind::ClaudeCode).unwrap();
        assert!(!descriptor.screen_shows_background_agent_hint(
            "  I will spawn an agent to sweep the remaining files.\n"
        ));
        assert!(!descriptor.screen_shows_background_agent_hint("  \u{2190} back to the menu\n"));
        assert!(!descriptor.screen_shows_background_agent_hint(""));
        // Unmeasured CLIs fold to false rather than guessing.
        let codex = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert!(codex.background_agent_hint_screen_phrases.is_empty());
        assert!(!codex.screen_shows_background_agent_hint("\u{276f}  \u{2190} 1 agent\n"));
    }

    // The state the misread turned on: a hint with NO working phrase is a
    // healthy idle row, not a stuck draft and not a busy one.
    #[test]
    fn a_hint_without_a_working_phrase_is_idle_with_a_background_agent() {
        let screen = "\u{276f}   \u{b7} \u{2190} 1 agent\n";
        assert!(crate::screen_text_shows_agent_background_hint(screen));
        assert!(
            !crate::screen_text_shows_agent_working(screen),
            "no interrupt footer means no turn is in flight"
        );
        // And while a turn IS running the row carries both, which is also true
        // and must not be reported as a draft either.
        let working = " esc to interrupt \u{b7} \u{2190} 1 agent\n";
        assert!(crate::screen_text_shows_agent_background_hint(working));
        assert!(crate::screen_text_shows_agent_working(working));
    }

    // The union is what callers with no kind in hand read, and it must agree
    // with the per-kind answer for the kind that HAS been measured.
    #[test]
    fn the_kind_agnostic_union_sees_the_measured_picker() {
        assert!(crate::screen_text_shows_agent_question_picker(
            " Enter to select · ↑/↓ to navigate · Esc to cancel\n"
        ));
        assert!(!crate::screen_text_shows_agent_question_picker(
            " nothing here is a picker\n"
        ));
    }

    #[test]
    fn opencode_index_reads_v2_first_and_never_answers_about_a_blind_table() {
        let home = std::env::temp_dir().join(format!("ygg-oc-idx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let oc_dir = home.join(".local/share/opencode");
        std::fs::create_dir_all(&oc_dir).unwrap();
        let conn = rusqlite::Connection::open(oc_dir.join("opencode.db")).unwrap();
        // The measured 2026-08-29 shape: the v1-era table holds STALE rows and
        // stops receiving writes once the service migrates to session_v2.
        conn.execute(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, title TEXT, \
             time_updated INTEGER, time_created INTEGER);",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, title, time_updated, time_created) \
             VALUES ('ses_v1only0000000000000000000x', '/home/user/proj', 'v1 session', 1, 1);",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE session_v2 (id TEXT PRIMARY KEY, project_id TEXT, parent_id TEXT, \
             directory TEXT, title TEXT, time_updated INTEGER, time_created INTEGER);",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2 (id, project_id, parent_id, directory, title, time_updated, \
             time_created) VALUES ('ses_realv2id0000000000000001', 'p1', NULL, \
             '/home/user/proj', 'v2 session', 1787984001574, 1787937064362);",
            [],
        )
        .unwrap();

        // A real v2 id is FOUND — the defect this locks: the v1-only reader
        // answered "absent" for ids the service actively served.
        assert_eq!(
            opencode_store_index_holds_session(&home, "ses_realv2id0000000000000001"),
            Some(true)
        );
        // A v1-era id is still found through the fallback table.
        assert_eq!(
            opencode_store_index_holds_session(&home, "ses_v1only0000000000000000000x"),
            Some(true)
        );
        // A genuinely absent id is a definite no, not unknown.
        assert_eq!(
            opencode_store_index_holds_session(&home, "ses_missing000000000000000001x"),
            Some(false)
        );
        // No DB at all → the store cannot answer (never absence).
        let empty = std::env::temp_dir().join(format!("ygg-oc-idx-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&empty);
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(opencode_store_index_holds_session(&empty, "ses_whatever00000000001"), None);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn muse_live_store_title_reads_index_and_jsonl() {
        let home = std::env::temp_dir().join(format!("ygg-muse-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let muse_dir = home.join(".local/share/muse");
        std::fs::create_dir_all(&muse_dir).unwrap();
        let db_path = muse_dir.join("session-index.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE sessions (session_id TEXT PRIMARY KEY, workspace_root TEXT, title TEXT, updated_at_us INTEGER);",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, workspace_root, title, updated_at_us) VALUES (?1, ?2, ?3, ?4);",
            rusqlite::params![
                "test-muse-uuid-1",
                "/home/user/proj",
                "Refactor Database Layer",
                1700000000000000i64,
            ],
        ).unwrap();

        let title = read_muse_live_store_title(&home, "test-muse-uuid-1");
        assert_eq!(title.as_deref(), Some("Refactor Database Layer"));

        // Fallback test: session not in DB but has session.jsonl
        let session_dir = muse_dir.join("sessions/2026/08/24/test-muse-uuid-2");
        std::fs::create_dir_all(&session_dir).unwrap();
        let jsonl_path = session_dir.join("session.jsonl");
        let jsonl_content = r#"{"payload_type":"runtime.user_intent.accepted","payload":{"model_messages":[{"content":[{"text":"Fix yggterm sidebar live titles"}]}]}}"#;
        std::fs::write(&jsonl_path, jsonl_content).unwrap();

        let title2 = read_muse_live_store_title(&home, "test-muse-uuid-2");
        assert_eq!(title2.as_deref(), Some("Fix Yggterm Sidebar Live Titles"));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Muse can emit hundreds of lifecycle records before the first accepted
    /// user intent.  The index has been observed to keep `prompt_count = 0`
    /// and `title = "New session"` even after that intent exists, so the
    /// transcript must remain capable of proving that this is a real session.
    #[test]
    fn muse_title_search_reaches_past_startup_lifecycle_records() {
        let root = dirs::home_dir()
            .unwrap()
            .join(".yggterm/scratchpad")
            .join(format!("muse-title-deep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let transcript = root.join("session.jsonl");
        let mut records = (0..80)
            .map(|sequence| {
                format!(
                    "{{\"sequence\":{sequence},\"payload_type\":\"runtime.lifecycle\"}}\n"
                )
            })
            .collect::<String>();
        records.push_str(
            r#"{"payload_type":"runtime.user_intent.accepted","payload":{"model_messages":[{"content":[{"text":"Repair durable Muse discovery"}]}]}}
"#,
        );
        std::fs::write(&transcript, records).unwrap();

        assert_eq!(
            muse_title_from_session_jsonl(&transcript).as_deref(),
            Some("Repair Durable Muse Discovery")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An accepted Muse envelope is not automatically a usable title. A
    /// launch-purpose envelope may precede the first real user task, so the
    /// reader must continue after the title classifier rejects one candidate.
    #[test]
    fn muse_title_search_continues_after_low_signal_intent() {
        let root = dirs::home_dir()
            .unwrap()
            .join(".yggterm/scratchpad")
            .join(format!("muse-title-candidates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let transcript = root.join("session.jsonl");
        std::fs::write(
            &transcript,
            concat!(
                r#"{"payload_type":"runtime.user_intent.accepted","payload":{"model_messages":[{"content":[{"text":"New Muse Code Session"}]}]}}"#,
                "\n",
                r#"{"payload_type":"runtime.user_intent.accepted","payload":{"model_messages":[{"content":[{"text":"Repair persistent CLI row identity"}]}]}}"#,
                "\n",
            ),
        )
        .unwrap();

        assert_eq!(
            muse_title_from_session_jsonl(&transcript).as_deref(),
            Some("Repair Persistent CLI Row Identity")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The ssh-side Muse reader used to stop after 65 records even though its
    /// local twin scans until the first accepted intent. Real Muse sessions
    /// have been measured with that intent hundreds of lifecycle records in,
    /// so the same durable session was titled locally and left as a raw cwd in
    /// a remote row. Exercise the actual Python probe, not a Rust paraphrase.
    #[test]
    fn remote_muse_title_search_reaches_past_startup_lifecycle_records() {
        let root = dirs::home_dir()
            .unwrap()
            .join(".yggterm/scratchpad")
            .join(format!("muse-remote-title-deep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let session_id = "9f1c2e3a-4b5d-46e7-8f90-1a2b3c4d5e6f";
        let session_dir = root
            .join(".local/share/muse/sessions/2026/08/25")
            .join(session_id);
        std::fs::create_dir_all(&session_dir).unwrap();
        let mut records = (0..80)
            .map(|sequence| {
                format!(
                    "{{\"sequence\":{sequence},\"payload_type\":\"runtime.lifecycle\"}}\n"
                )
            })
            .collect::<String>();
        records.push_str(
            r#"{"payload_type":"runtime.user_intent.accepted","payload":{"model_messages":[{"content":[{"text":"New Muse Code Session"}]}]}}
{"payload_type":"runtime.user_intent.accepted","payload":{"model_messages":[{"content":[{"text":"Repair remote Muse title parity"}]}]}}
"#,
        );
        std::fs::write(session_dir.join("session.jsonl"), records).unwrap();

        let descriptor = agent_cli_descriptor(SessionKind::Muse).expect("Muse is registered");
        let probe = descriptor
            .remote_live_store_title
            .expect("Muse declares a remote probe");
        let mut args = descriptor.remote_store_title_locators();
        args.push("--".to_string());
        args.push(session_id.to_string());
        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(probe.script)
            .args(args)
            .env("HOME", &root)
            .output()
            .expect("python3 is needed to exercise the remote Muse probe");
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            output.status.success(),
            "the probe script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let line = String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(str::to_string)
            .expect("the remote probe must answer for the deep Muse intent");
        let value: serde_json::Value = serde_json::from_str(&line).expect("JSON lines");
        let candidates: Vec<String> = value["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .filter_map(|candidate| candidate.as_str().map(ToOwned::to_owned))
            .collect();
        assert_eq!(
            (probe.choose)(&candidates),
            Some("Repair Muse Title Parity".to_string())
        );
    }

    /// Muse stores raw prompts. `please ...` is low-signal only as a finished
    /// title; it must first be condensed, exactly as the local durable reader
    /// does, or the remote half rejects useful title input as store absence.
    #[test]
    fn muse_remote_title_condenses_before_classifying_prompt_copy() {
        let candidates = vec![
            "Please inspect the widget importer and continue tracing its dropped rows."
                .to_string(),
        ];
        let chosen = super::first_muse_title_candidate(&candidates)
            .expect("the raw prompt should condense to a usable title");
        assert!(!chosen.to_ascii_lowercase().starts_with("please "));
        assert!(!crate::looks_like_generated_fallback_title(&chosen));
        assert!(!crate::looks_like_low_signal_generated_copy(&chosen));
    }

    /// Regression lock for the 2026-08-28 wrong-package fix: OpenCode's v2
    /// line ships as `@opencode-ai/cli` under the `beta` tag and installs the
    /// binary `opencode2`. The tag NAME alone is not the decision — the same
    /// word exists on the abandoned unscoped v1 package — so both halves of
    /// the pin are asserted here, and a future edit that restores either half
    /// of the frozen-August drift fails this instead of every opencode row.
    #[test]
    fn opencode_installs_the_v2_preview_package_not_the_abandoned_v1_beta() {
        let descriptor = agent_cli_descriptor(SessionKind::OpenCode)
            .expect("OpenCode is a registered agent CLI");
        assert_eq!(descriptor.binary_name, "opencode2");
        match descriptor.install {
            CliInstall::Npm(package) => assert_eq!(package, "@opencode-ai/cli"),
            other => panic!("OpenCode must install from npm, got {other:?}"),
        }
        assert_eq!(npm_dist_tag(SessionKind::OpenCode), Some("beta"));
    }
}
