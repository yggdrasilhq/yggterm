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
    /// Window and folding match the matcher this replaced exactly: the last ten
    /// non-empty lines, ASCII-lowercased, negations checked first.
    pub fn screen_shows_working(&self, sample: &str) -> bool {
        sample.lines().rev().take(10).any(|line| {
            let line = line.trim();
            if line.is_empty() {
                return false;
            }
            let lower = line.to_ascii_lowercase();
            if self
                .working_screen_negations
                .iter()
                .any(|deny| lower.contains(deny))
            {
                return false;
            }
            self.working_screen_phrases.iter().any(|phrase| {
                lower.contains(phrase.needle)
                    && (phrase.also_any.is_empty()
                        || phrase.also_any.iter().any(|also| lower.contains(also)))
            })
        })
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
        store_scan_gap: None,
        store_home_env_override: Some(crate::ENV_YGGTERM_CODEX_HOME),
        read_store_entry: read_codex_store_entry,
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
        store_scan_gap: None,
        // No override: only `resolve_codex_home` consults an env var, and it
        // relocates `.codex` alone. Preserving that exactly.
        store_home_env_override: None,
        read_store_entry: read_codex_store_entry,
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
        store_scan_gap: None,
        store_home_env_override: None,
        read_store_entry: read_claude_code_store_entry,
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
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_pi_store_entry,
    },
    AgentCliDescriptor {
        kind: SessionKind::OpenCode,
        display_name: "OpenCode",
        session_metadata_label: "OpenCode Session",
        slug: "opencode",
        binary_name: "opencode",
        // The npm package is `opencode-ai`; the binary it installs is
        // `opencode`. Naming the wrong one is how provisioning silently
        // installs nothing.
        install: CliInstall::Npm("opencode-ai"),
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
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_no_store_entry,
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
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_qwen_store_entry,
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
        title_authority: TitleAuthority::Store,
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
        session_store_globs: &[],
        store_excluded_name_fragments: &[],
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_no_store_entry,
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
        composer_marker: '\u{276f}',
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
        // Verified 2026-08-16 on openclaw: `session-index.db.sessions(workspace_root→cwd, title,
        // updated_at_us)` carries the cwd/title the cwd tree and startpage need, and
        // `route_facts.cwd` in the JSONL is the fallback when the DB is absent.
        session_store_globs: &[".local/share/muse/sessions/**/session.jsonl"],
        store_excluded_name_fragments: &["/subagent/", "/tool-outputs/"],
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_muse_store_entry,
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
        // Read off `agy --help`, v1.0.5 on guihost (2026-08-08): resume is
        // `--conversation <ID>`, and `-c`/`--continue` takes the most recent.
        resume_selector: ResumeSelector::Flag("--conversation"),
        resume_re_roots_with_cwd: false,
        model_flag: "--model",
        composer_marker: '\u{276f}',
        composer_footer_hints: &["esc", "ctrl", "enter", "tab"],
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
            ".gemini/antigravity-cli/brain/*/.system_generated/logs/transcript.jsonl",
            ".antigravitycli/*.json",
        ],
        store_excluded_name_fragments: &["-shm", "-wal"],
        // None of the 2026-08-08 intake relocates its home with an env var.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_antigravity_store_entry,
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
        // MEASURED: `grok update  Check for updates or install a specific
        // version`. A CLI that ships its own updater has it PREFERRED over
        // re-running the install method.
        update: CliUpdate::SelfCommand(&["update"]),
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
        // grok reads `GROK_SANDBOX` for the sandbox profile; nothing in its help
        // or its strings relocates the HOME, which stays `~/.grok`.
        store_home_env_override: None,
        store_scan_gap: None,
        read_store_entry: read_grok_build_store_entry,
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

fn muse_title_from_session_jsonl(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().flatten().take(64) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            // Muse's user prompt lives in payload.model_messages[0].content[0].text
            // under payload_type runtime.user_intent.accepted / materialized.
            let pt = value.get("payload_type").and_then(|v| v.as_str()).unwrap_or("");
            if pt == "runtime.user_intent.accepted" || pt == "runtime.user_intent.materialized" {
                if let Some(text) = value
                    .get("payload")
                    .and_then(|p| p.get("model_messages"))
                    .and_then(|m| m.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|msg| msg.get("content"))
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if let Some(title) = crate::best_effort_title_from_context(trimmed) {
                            return Some(title);
                        }
                        // Fallback: first line
                        let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();
                        if !first_line.is_empty() && first_line.len() <= 120 {
                            return Some(first_line.to_string());
                        }
                        return Some(trimmed.chars().take(80).collect());
                    }
                }
                // Fallback inside refill_blocks
                if let Some(text) = value
                    .get("payload")
                    .and_then(|p| p.get("refill_blocks"))
                    .and_then(|r| r.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|b| b.get("text"))
                    .and_then(|t| t.as_str())
                {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if let Some(title) = crate::best_effort_title_from_context(trimmed) {
                            return Some(title);
                        }
                    }
                }
            }
        }
    }
    None
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
    let title = db_title
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
    let title = db_title
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

fn clean_agy_prompt_first_line(raw: &str) -> Option<String> {
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
            || l.starts_with("{{ CHECKPOINT")
            || l.starts_with("<USER_REQUEST>")
        {
            continue;
        }
        if !crate::looks_like_generated_fallback_title(l) {
            return Some(l.to_string());
        }
    }
    None
}

fn read_antigravity_store_entry(path: &Path) -> Option<AgentStoreEntry> {
    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        // Layout: ~/.gemini/antigravity-cli/brain/<uuid>/.system_generated/logs/transcript.jsonl
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
    let home = dirs::home_dir()?;
    // Prefer the SQLite index for cwd/title/mtime — it is the same source
    // `muse resume` lists from, and it contains the workspace_root and
    // already-extracted title without scanning multi-MB JSONL.
    let (db_cwd, db_title, db_updated_ms) = 'db_block: {
        let db_path = home.join(".local/share/muse/session-index.db");
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
pub fn new_session_birth_title(kind: SessionKind) -> String {
    match agent_cli_descriptor(kind) {
        Some(descriptor) => descriptor.new_session_label(),
        None => match kind {
            SessionKind::Shell => "New Terminal".to_string(),
            SessionKind::SshShell => "New SSH Terminal".to_string(),
            SessionKind::Document => "New Document".to_string(),
            // Unreachable while every agent kind has a descriptor, which
            // `SessionKind::is_agent` derives from this very registry — so a
            // new kind reaching here is a missing registration, not a name to
            // invent. Stay generic rather than guess a product name.
            _ => "New Session".to_string(),
        },
    }
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
            // scanned despite empty globs, so no gap is required.
            if descriptor.session_store_globs.is_empty()
                && !matches!(descriptor.kind, SessionKind::OpenCode | SessionKind::Kimi)
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
}
