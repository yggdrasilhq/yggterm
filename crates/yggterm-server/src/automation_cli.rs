//! THE `automation …` verb plane, owned once for BOTH binaries.
//!
//! Same rule as `app_control_web_cli`: a flag must mean one thing whether it
//! was typed at `yggterm` or `yggterm-headless`, so neither binary carries a
//! copy of this parser. `yggterm-headless` is what the generated systemd unit
//! invokes; `yggterm` has it too because an agent driving the GUI binary should
//! not have to know that.
//!
//! # The executor calls the SAME verbs a human would
//!
//! [`execute_run`] opens its session through
//! [`crate::create_terminal_with_tenancy`] and injects through
//! [`crate::submit_terminal_prompt`] — the exact functions behind `server app
//! terminal new` and `server app terminal submit-prompt`. It does not have a
//! private spawn path. That is the single-source-of-truth rule applied where it
//! matters most: if an automation could create a row some other way, the two
//! ways would diverge and only one of them would arm the reaper.
//!
//! # ⚠ A known gap, stated rather than hidden
//!
//! `CreateTerminal` is an app-control command, and app-control routes to a GUI
//! worker. **An automation therefore cannot open a session while no GUI is
//! running.** On the live host the GUI is up essentially always, so the
//! motivating midnight job works — but a machine that reboots to a login screen
//! and stays there will record `spawn_failed` and raise a notice rather than
//! silently doing nothing. Closing this gap needs a daemon-side create, which
//! does not exist today (there is no `server terminal new`, only `server app
//! terminal new`). Recorded in docs/automations.md; not papered over here.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use serde_json::{Value, json};

use yggterm_core::cli_flag_value;

use crate::automation::{
    Automation, AutomationNotice, AutomationRun, CloseReason,
    DEFAULT_DEADLINE_SECS, DEFAULT_GRACE_SECS, DEFAULT_IDLE_TTL_SECS, GraceVerdict, NoticeKind,
    RunOutcome, grace_verdict, iso_week_number, load_store, parse_calendar, save_store,
    validate_id,
};
use crate::automation_units::{
    GeneratedUnit, UnitState, orphaned_units, reconcile, systemd_user_unit_dir, systemd_units,
};
use crate::session_tenancy::{CreateTerminalTenancy, CreatorStamp, EphemeralDeclaration};

/// How long to wait for a freshly spawned agent CLI to reach an idle prompt
/// before giving up on injecting. Generous: `claude` and `codex` both do
/// non-trivial startup work, and an automation has all night.
const PROMPT_READY_TIMEOUT_MS: u64 = 120_000;

/// The delegate-launch flags on `server app terminal new`, owned ONCE so both
/// binaries print the same text under their own name.
///
/// An agent that reads `--help` and does not see a verb concludes the build
/// lacks it — the misdiagnosis that kept `--kind claude-code` unused for weeks
/// while agents hand-rolled `--kind shell` workarounds instead.
pub fn delegate_launch_usage_block(binary: &str) -> String {
    format!(
        "delegate launch (server app terminal new, agent CLI kinds only):
  {binary} server app terminal new --kind claude-code --cwd <dir> --no-activate
      --purpose <what-for> --model <id> --permission-mode bypass --prompt <text>

  --model <id>              pins THIS launch's model instead of inheriting the
                            user's default (which is the expensive tier a
                            delegate exists to avoid). REFUSED on --kind shell.
  --permission-mode <mode>  default | plan | accept-edits | bypass. bypass ⇒
                            claude --dangerously-skip-permissions, codex
                            --dangerously-bypass-approvals-and-sandbox. codex
                            has no plan/accept-edits and says so. PER-LAUNCH:
                            it never reads or writes the global
                            claude_code_extra_args setting, and it wins over
                            whatever that setting holds.
  --prompt <text>           the opening prompt, delivered once the CLI
                            ECHO-CONFIRMS it is consuming input, as two writes
                            (text, then a discrete Enter). A single write with a
                            trailing newline is paste-buffered by the CC TUI and
                            never submits — that is why there is a verb for this.
  --prompt-stdin            read it from stdin instead (no quoting to get wrong)

  Agent CLI kinds are BORN keep-alive, so no --keep-alive flag is needed.
  The reply's `launch` block reports what the ROW was born with — model,
  permission_mode, applied, and the real launch_command — read back from the
  created row rather than echoed, so a create that landed on an older daemon
  reads applied:false instead of lying.\n"
    )
}

pub fn automation_usage_block(binary: &str) -> String {
    format!(
        "automations (scheduled agent-CLI sessions — see docs/automations.md):
  {binary} automation list [--json]
  {binary} automation show <id> [--json]
  {binary} automation create --id <slug> --kind <shell|codex|claude-code> --cwd <dir>
      --machine-key <host> (--prompt <text>|--prompt-stdin) --calendar <expr>
      [--every-n-weeks <n>] [--grace <secs>] [--idle-ttl <secs>] [--deadline <secs>]
      [--attach] [--title <t>] [--disabled]
  {binary} automation edit <id> [any create flag]
  {binary} automation enable <id> | disable <id> | delete <id>
  {binary} automation run <id> [--force] [--json]
  {binary} automation runs [<id>] [--json]
  {binary} automation notices [--json]
  {binary} automation dismiss <run-id>
  {binary} automation sync [--json] [--prune]

  `run` is what the GENERATED systemd timer invokes; --force ignores the grace
  and cadence guards, which is what \"Run now\" and a test both want.
  `sync` reconciles the generated unit files against the store — the store
  always wins, and a hand-edited unit is REPORTED, never silently overwritten.
  --prune removes unit files we generated that no automation claims any more."
    )
}

/// Local UTC offset in seconds, including DST.
///
/// Read through `libc::localtime_r` rather than the `time` crate's
/// `current_local_offset`, which refuses to answer in a multi-threaded process
/// (soundness, not capability). Getting this wrong is not a rounding error:
/// in IST a UTC fallback moves "midnight" to 05:30, which is precisely the
/// working hours the whole feature exists to stay out of. So a failure here is
/// an ERROR, never a silent zero.
#[cfg(unix)]
pub fn local_utc_offset_secs(now_ms: u64) -> anyhow::Result<i32> {
    // SAFETY: `localtime_r` writes into a caller-provided `tm` and takes a
    // pointer to a `time_t` we own; both live for the call.
    unsafe {
        let when = (now_ms / 1000) as libc::time_t;
        let mut out: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&when, &mut out).is_null() {
            return Err(anyhow!(
                "cannot read this machine's UTC offset, and guessing it would move a midnight \
                 job by whole hours"
            ));
        }
        i32::try_from(out.tm_gmtoff).context("this machine's UTC offset does not fit in i32")
    }
}

#[cfg(not(unix))]
pub fn local_utc_offset_secs(_now_ms: u64) -> anyhow::Result<i32> {
    Err(anyhow!(
        "automations need a local UTC offset and this platform has no reader yet — see \
         docs/automations.md, Windows is a 3.0.0 concern"
    ))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn home_dir() -> anyhow::Result<PathBuf> {
    crate::resolve_yggterm_home()
}

fn user_home() -> anyhow::Result<PathBuf> {
    dirs::home_dir().context("cannot resolve this user's home directory")
}

/// The absolute path to the binary a generated unit should invoke.
///
/// `current_exe()` and not a name on PATH: a systemd user unit inherits almost
/// no environment, and a unit that cannot find its own binary fails in the
/// middle of the night with nobody watching.
fn headless_exe() -> anyhow::Result<PathBuf> {
    let current = std::env::current_exe().context("cannot resolve this binary's own path")?;
    // Both binaries expose the verb, but the unit should name the headless one:
    // it is the one that does not want a display.
    if current.file_name().and_then(|name| name.to_str()) == Some("yggterm") {
        let sibling = current.with_file_name("yggterm-headless");
        if sibling.exists() {
            return Ok(sibling);
        }
    }
    Ok(current)
}

fn flag_u64(args: &[String], flag: &str) -> anyhow::Result<Option<u64>> {
    cli_flag_value(args, flag)
        .map(|raw| {
            raw.parse::<u64>()
                .with_context(|| format!("{flag} expects a whole number of seconds, got {raw:?}"))
        })
        .transpose()
}

/// Read `--prompt <text>` / `--prompt-stdin`.
///
/// Public because `server app terminal new --prompt` reads the SAME two flags
/// with the same stdin behaviour. Two readers would be two chances to disagree
/// about which flag wins, and the delegate-launch path is the one place where a
/// silently empty prompt means an agent session that starts and is asked
/// nothing.
pub fn read_prompt(args: &[String]) -> anyhow::Result<Option<String>> {
    if args.iter().any(|arg| arg == "--prompt-stdin") {
        let mut value = String::new();
        std::io::stdin()
            .read_to_string(&mut value)
            .context("reading automation prompt from stdin")?;
        // ⛔ An EMPTY read is refused BY NAME rather than passed on.
        //
        // It used to return `Some("")`, which the delivery path flattened to
        // `prompt: null` in the reply — so a delegate launched over ssh with
        // `--prompt-stdin` and no stdin came back looking successful
        // (`launch.applied: true`, the model and permission mode both right)
        // while the row sat idle with nothing to do. Live-hit 2026-08-06.
        // A launched-but-silent delegate is the most expensive way to fail
        // here, because nothing looks wrong until someone reads the row.
        if value.trim().is_empty() {
            anyhow::bail!(
                "--prompt-stdin was given but stdin was empty; pipe the prompt in \
                 (`… --prompt-stdin < brief.md`, or `cat brief.md | …`) or pass \
                 `--prompt <text>`. Over ssh, quote the remote command so the \
                 redirect is applied where you mean it."
            );
        }
        return Ok(Some(value));
    }
    Ok(cli_flag_value(args, "--prompt").map(str::to_string))
}

fn print(value: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// The executor
// ---------------------------------------------------------------------------

/// What one `automation run` did. Returned rather than printed so the daemon
/// chore and the CLI share it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub outcome: RunOutcome,
    pub due_at_ms: u64,
    pub session_path: Option<String>,
    pub detail: String,
}

/// Execute one run of `automation`, mutating it with the result.
///
/// `now_ms` and `utc_offset_secs` are arguments for the same reason every guard
/// in `automation.rs` takes them: this is testable against a fixed instant, and
/// a test can put the clock at 9 a.m. on a Sunday without waiting for one.
pub fn execute_run(
    automation: &mut Automation,
    force: bool,
    now_ms: u64,
    utc_offset_secs: i32,
    open_session: &dyn Fn(&Automation) -> anyhow::Result<String>,
    inject: &dyn Fn(&str, &str) -> anyhow::Result<()>,
) -> RunReport {
    let due_at_ms = automation
        .calendar
        .previous_occurrence_at_or_before(now_ms, utc_offset_secs)
        .unwrap_or(now_ms);
    let run_id = format!("{}-{}", automation.id, due_at_ms);

    let finish = |automation: &mut Automation, report: RunReport, error: Option<String>| {
        automation.record_run(AutomationRun {
            run_id: run_id.clone(),
            due_at_ms: report.due_at_ms,
            started_at_ms: now_ms,
            outcome: report.outcome,
            session_path: report.session_path.clone(),
            closed_at_ms: None,
            close_reason: CloseReason::Never,
            error,
        });
        report
    };

    if !force {
        // D3 — the grace guard.
        if let GraceVerdict::OutOfGrace { late_by_secs } =
            grace_verdict(due_at_ms, now_ms, automation.grace_secs)
        {
            return finish(
                automation,
                RunReport {
                    outcome: RunOutcome::SkippedOutOfGrace,
                    due_at_ms,
                    session_path: None,
                    detail: format!(
                        "{late_by_secs}s late, past a {}s grace window — skipped so it cannot \
                         start while the user is working",
                        automation.grace_secs
                    ),
                },
                None,
            );
        }
        // The every-N-weeks parity guard.
        if !automation.cadence_honours(due_at_ms, utc_offset_secs) {
            return finish(
                automation,
                RunReport {
                    outcome: RunOutcome::SkippedOffCadence,
                    due_at_ms,
                    session_path: None,
                    detail: format!(
                        "off week for an every-{}-weeks automation",
                        automation.every_n_weeks
                    ),
                },
                None,
            );
        }
    }

    // E1 — reuse before spawn. The previous run's session may still be live,
    // and a second row for the same job is exactly the leak this feature is
    // meant to prevent.
    if let Some(existing) = automation.open_run().and_then(|run| run.session_path.clone()) {
        return match inject(&existing, &automation.prompt) {
            Ok(()) => finish(
                automation,
                RunReport {
                    outcome: RunOutcome::ReusedLiveSession,
                    due_at_ms,
                    session_path: Some(existing.clone()),
                    detail: format!("re-prompted the live session {existing} instead of spawning a duplicate"),
                },
                None,
            ),
            Err(error) => finish(
                automation,
                RunReport {
                    outcome: RunOutcome::SpawnFailed,
                    due_at_ms,
                    session_path: Some(existing.clone()),
                    detail: format!("could not re-prompt the live session {existing}: {error}"),
                },
                Some(error.to_string()),
            ),
        };
    }

    let session_path = match open_session(automation) {
        Ok(path) => path,
        Err(error) => {
            return finish(
                automation,
                RunReport {
                    outcome: RunOutcome::SpawnFailed,
                    due_at_ms,
                    session_path: None,
                    detail: format!("could not open a session: {error}"),
                },
                Some(error.to_string()),
            );
        }
    };

    match inject(&session_path, &automation.prompt) {
        Ok(()) => finish(
            automation,
            RunReport {
                outcome: RunOutcome::Ran,
                due_at_ms,
                session_path: Some(session_path.clone()),
                detail: format!("opened {session_path} and delivered the prompt"),
            },
            None,
        ),
        Err(error) => finish(
            automation,
            RunReport {
                outcome: RunOutcome::SpawnFailed,
                due_at_ms,
                // The ROW EXISTS even though the prompt did not land, and the
                // record has to say so — otherwise the reaper's row is one
                // nothing in the store admits to owning.
                session_path: Some(session_path.clone()),
                detail: format!("opened {session_path} but the prompt did not land: {error}"),
            },
            Some(error.to_string()),
        ),
    }
}

/// The real spawn: the same verb a human types, with the tenancy declaration
/// that arms the EXISTING reaper.
fn open_session_for_real(automation: &Automation, timeout_ms: u64) -> anyhow::Result<String> {
    let tenancy = CreateTerminalTenancy {
        created_by: CreatorStamp::new(
            std::process::id(),
            &crate::session_tenancy::local_host_token(),
            Some(&format!("automation:{}", automation.id)),
        )
        // An automation usually runs from a timer with no row of its own, in
        // which case this is None and the row is correctly top-level. When it
        // IS run from inside a row, that row is its parent.
        .with_parent_session_path(
            crate::session_tenancy::parent_session_path_from_env().as_deref(),
        ),
        // TTL-only, and deliberately no owner pid. This process exits the
        // moment the run is recorded — naming it as the owner would have the
        // reaper close the session within a minute, which is the exact opposite
        // of what an all-night infra job wants.
        ephemeral: Some(EphemeralDeclaration::new(
            None,
            &crate::session_tenancy::local_host_token(),
            Some(automation.idle_ttl_secs),
        )),
    };
    let payload = crate::create_terminal_with_tenancy(
        Some(automation.machine_key.as_str()),
        automation.cwd.as_deref(),
        automation.title.as_deref(),
        Some(&format!("automation:{}", automation.id)),
        Some(session_kind_flag(automation)),
        automation.attach,
        Some(tenancy),
        timeout_ms,
    )?;
    payload
        .get("data")
        .and_then(|data| data.get("session_path"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "the create returned no session path. If no GUI is running this is the known gap \
                 in docs/automations.md: CreateTerminal is an app-control command and app-control \
                 routes to a GUI worker. Payload: {payload}"
            )
        })
}

fn session_kind_flag(automation: &Automation) -> &'static str {
    use yggterm_core::SessionKind;
    match automation.agent_kind {
        SessionKind::ClaudeCode => "claude-code",
        SessionKind::Codex | SessionKind::CodexLiteLlm => "codex",
        _ => "shell",
    }
}

// ---------------------------------------------------------------------------
// Unit rendering on disk
// ---------------------------------------------------------------------------

fn write_units(units: &[GeneratedUnit]) -> anyhow::Result<()> {
    for unit in units {
        if let Some(parent) = unit.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&unit.path, &unit.contents)
            .with_context(|| format!("writing {}", unit.path.display()))?;
    }
    Ok(())
}

fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Regenerate an automation's units, refusing to clobber a hand-edited one.
fn sync_one(automation: &Automation, exe: &Path, unit_dir: &Path) -> anyhow::Result<Value> {
    let reports = reconcile(automation, exe, unit_dir, &read_file);
    let hand_edited: Vec<String> = reports
        .iter()
        .filter(|report| report.state == UnitState::HandEdited)
        .map(|report| report.path.display().to_string())
        .collect();
    let expected = systemd_units(automation, exe, unit_dir);
    // A DISABLED automation keeps its record and loses its timer. Leaving the
    // timer behind would keep opening agent sessions for a job the user
    // believes they turned off.
    if !automation.enabled {
        let mut removed = Vec::new();
        for unit in &expected {
            if unit.path.exists() && crate::automation_units::is_generated(&read_file(&unit.path).unwrap_or_default()) {
                std::fs::remove_file(&unit.path).ok();
                removed.push(unit.path.display().to_string());
            }
        }
        return Ok(json!({
            "id": automation.id,
            "enabled": false,
            "removed": removed,
        }));
    }
    if !hand_edited.is_empty() {
        return Ok(json!({
            "id": automation.id,
            "written": false,
            "hand_edited": hand_edited,
            "detail": "left alone: these files are not ours or were edited after we wrote them. \
                       Delete them and re-run sync to regenerate.",
        }));
    }
    write_units(&expected)?;
    Ok(json!({
        "id": automation.id,
        "written": expected.iter().map(|unit| unit.path.display().to_string()).collect::<Vec<_>>(),
        "states_before": reports
            .iter()
            .map(|report| format!("{:?}", report.state))
            .collect::<Vec<_>>(),
    }))
}

fn automation_to_json(automation: &Automation, utc_offset_secs: i32, now: u64) -> Value {
    json!({
        "id": automation.id,
        "enabled": automation.enabled,
        "kind": session_kind_flag(automation),
        "machine_key": automation.machine_key,
        "cwd": automation.cwd,
        "calendar": automation.calendar.to_on_calendar(),
        "every_n_weeks": automation.every_n_weeks,
        "grace_secs": automation.grace_secs,
        "idle_ttl_secs": automation.idle_ttl_secs,
        "deadline_secs": automation.deadline_secs,
        "attach": automation.attach,
        "prompt": automation.prompt,
        "last_run_at_ms": automation.last_run_at_ms,
        "next_honoured_run_at_ms": automation.next_honoured_run_after(now, utc_offset_secs),
        "open_session": automation.open_run().and_then(|run| run.session_path.clone()),
    })
}

// ---------------------------------------------------------------------------
// The CLI
// ---------------------------------------------------------------------------

/// `args` is the full argv tail, e.g. `["automation", "run", "infra-upgrade"]`.
pub fn run_automation_cli(args: &[String], timeout_ms: u64) -> anyhow::Result<()> {
    let action = args
        .get(1)
        .map(String::as_str)
        .context("missing automation action — try `automation list`")?;
    let json_out = args.iter().any(|arg| arg == "--json");
    let now = now_ms();
    let offset = local_utc_offset_secs(now)?;
    let home = home_dir()?;
    let mut store = load_store(&home).with_context(|| {
        format!(
            "reading {} — a corrupt store is not silently reset, because that would lose your \
             schedule",
            crate::automation::automations_path(&home).display()
        )
    })?;
    let unit_dir = systemd_user_unit_dir(&user_home()?);
    let exe = headless_exe()?;

    match action {
        "list" => {
            let listed: Vec<Value> = store
                .automations
                .iter()
                .map(|automation| automation_to_json(automation, offset, now))
                .collect();
            if json_out {
                print(&json!({ "automations": listed }))?;
            } else if listed.is_empty() {
                println!("no automations. `automation create --help` to make one.");
            } else {
                for automation in &store.automations {
                    println!(
                        "{:<24} {:<9} {:<22} every {} week(s)  {}",
                        automation.id,
                        if automation.enabled { "enabled" } else { "disabled" },
                        automation.calendar.to_on_calendar(),
                        automation.every_n_weeks,
                        session_kind_flag(automation),
                    );
                }
            }
            Ok(())
        }
        "show" => {
            let id = args.get(2).context("missing automation id")?;
            let automation = store
                .get(id)
                .with_context(|| format!("no automation {id:?}"))?;
            let mut value = automation_to_json(automation, offset, now);
            value["runs"] = serde_json::to_value(&automation.runs)?;
            print(&value)
        }
        "create" | "edit" => {
            let editing = action == "edit";
            let id = if editing {
                args.get(2).context("missing automation id")?.to_string()
            } else {
                cli_flag_value(args, "--id")
                    .context("missing --id for automation create")?
                    .to_string()
            };
            validate_id(&id).map_err(|message| anyhow!(message))?;

            let mut automation = if editing {
                store
                    .get(&id)
                    .cloned()
                    .with_context(|| format!("no automation {id:?} to edit"))?
            } else {
                if store.get(&id).is_some() {
                    return Err(anyhow!(
                        "automation {id:?} already exists — `automation edit {id}` to change it"
                    ));
                }
                Automation {
                    id: id.clone(),
                    enabled: true,
                    agent_kind: yggterm_core::SessionKind::ClaudeCode,
                    machine_key: String::new(),
                    cwd: None,
                    prompt: String::new(),
                    calendar: parse_calendar("Sun *-*-* 00:00:00")
                        .map_err(|message| anyhow!(message))?,
                    every_n_weeks: 1,
                    anchor_week: 0,
                    grace_secs: DEFAULT_GRACE_SECS,
                    idle_ttl_secs: DEFAULT_IDLE_TTL_SECS,
                    deadline_secs: DEFAULT_DEADLINE_SECS,
                    attach: false,
                    title: None,
                    created_at_ms: now,
                    last_run_at_ms: None,
                    runs: Vec::new(),
                }
            };

            if let Some(kind) = cli_flag_value(args, "--kind") {
                automation.agent_kind = match kind {
                    "claude-code" | "claude" | "cc" => yggterm_core::SessionKind::ClaudeCode,
                    "codex" => yggterm_core::SessionKind::Codex,
                    "shell" => yggterm_core::SessionKind::Shell,
                    other => {
                        return Err(anyhow!(
                            "--kind {other:?} is not an agent CLI this scheduler can open. \
                             Try shell, codex or claude-code."
                        ));
                    }
                };
            }
            if let Some(machine) = cli_flag_value(args, "--machine-key") {
                automation.machine_key = machine.to_string();
            }
            if let Some(cwd) = cli_flag_value(args, "--cwd") {
                automation.cwd = Some(cwd.to_string());
            }
            if let Some(title) = cli_flag_value(args, "--title") {
                automation.title = Some(title.to_string());
            }
            if let Some(prompt) = read_prompt(args)? {
                automation.prompt = prompt;
            }
            if let Some(calendar) = cli_flag_value(args, "--calendar") {
                automation.calendar =
                    parse_calendar(calendar).map_err(|message| anyhow!(message))?;
            }
            if let Some(every) = cli_flag_value(args, "--every-n-weeks") {
                automation.every_n_weeks = every
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value >= 1)
                    .context("--every-n-weeks expects a whole number of weeks, 1 or more")?;
            }
            if let Some(value) = flag_u64(args, "--grace")? {
                automation.grace_secs = value;
            }
            if let Some(value) = flag_u64(args, "--idle-ttl")? {
                automation.idle_ttl_secs = value;
            }
            if let Some(value) = flag_u64(args, "--deadline")? {
                automation.deadline_secs = value;
            }
            if args.iter().any(|arg| arg == "--attach") {
                automation.attach = true;
            }
            if args.iter().any(|arg| arg == "--detach") {
                automation.attach = false;
            }
            if args.iter().any(|arg| arg == "--disabled") {
                automation.enabled = false;
            }

            if automation.machine_key.is_empty() {
                return Err(anyhow!("--machine-key names the machine the session opens on"));
            }
            if automation.prompt.trim().is_empty() {
                return Err(anyhow!(
                    "--prompt (or --prompt-stdin) is what the automation says to the agent; an \
                     empty one would open a session and ask it nothing"
                ));
            }
            if !editing {
                // Anchor the fortnight to the first honoured run rather than to
                // "now", so `every-n-weeks` counts from a Sunday the user can
                // point at instead of from whenever they happened to type this.
                let first = automation
                    .calendar
                    .next_occurrence_after(now, offset)
                    .unwrap_or(now);
                automation.anchor_week = iso_week_number(first, offset).unwrap_or(0);
            }

            store.upsert(automation.clone());
            save_store(&home, &store)?;
            let unit = sync_one(&automation, &exe, &unit_dir)?;
            print(&json!({
                "automation": automation_to_json(&automation, offset, now),
                "units": unit,
                "next_step": format!(
                    "systemctl --user daemon-reload && systemctl --user enable --now {}",
                    crate::automation_units::timer_unit_name(&automation.id)
                ),
            }))
        }
        "enable" | "disable" => {
            let id = args.get(2).context("missing automation id")?;
            let automation = store
                .get_mut(id)
                .with_context(|| format!("no automation {id:?}"))?;
            automation.enabled = action == "enable";
            let snapshot = automation.clone();
            save_store(&home, &store)?;
            let unit = sync_one(&snapshot, &exe, &unit_dir)?;
            print(&json!({ "id": id, "enabled": snapshot.enabled, "units": unit }))
        }
        "delete" => {
            let id = args.get(2).context("missing automation id")?;
            let removed = store
                .remove(id)
                .with_context(|| format!("no automation {id:?}"))?;
            save_store(&home, &store)?;
            // Its timer goes with it, or the deleted job keeps firing.
            let mut disabled = removed.clone();
            disabled.enabled = false;
            let unit = sync_one(&disabled, &exe, &unit_dir)?;
            print(&json!({ "deleted": id, "units": unit }))
        }
        "run" => {
            let id = args.get(2).context("missing automation id")?;
            let force = args.iter().any(|arg| arg == "--force");
            let mut automation = store
                .get(id)
                .cloned()
                .with_context(|| format!("no automation {id:?}"))?;
            if !automation.enabled && !force {
                println!("automation {id} is disabled; nothing to do");
                return Ok(());
            }
            let report = execute_run(
                &mut automation,
                force,
                now,
                offset,
                &|automation| open_session_for_real(automation, timeout_ms),
                &|session_path, prompt| {
                    crate::submit_terminal_prompt(
                        session_path,
                        prompt,
                        PROMPT_READY_TIMEOUT_MS,
                        timeout_ms,
                    )
                    .map(|_| ())
                },
            );
            if report.outcome == RunOutcome::SpawnFailed {
                let run_id = automation
                    .latest_run()
                    .map(|run| run.run_id.clone())
                    .unwrap_or_else(|| id.to_string());
                store.raise_notice(AutomationNotice {
                    run_id,
                    automation_id: id.to_string(),
                    kind: NoticeKind::SpawnFailed,
                    raised_at_ms: now,
                    message: report.detail.clone(),
                    session_path: report.session_path.clone(),
                });
            }
            store.upsert(automation);
            save_store(&home, &store)?;
            print(&json!({
                "id": id,
                "outcome": report.outcome.as_str(),
                "due_at_ms": report.due_at_ms,
                "session_path": report.session_path,
                "detail": report.detail,
            }))?;
            // A SKIP IS A SUCCESS — see RunOutcome::is_success. A non-zero exit
            // here would leave `systemctl --user list-timers` reporting a
            // permanently failed timer for a job working exactly as designed.
            if report.outcome.is_success() {
                Ok(())
            } else {
                Err(anyhow!("{}", report.detail))
            }
        }
        "runs" => {
            let listed: Vec<Value> = store
                .automations
                .iter()
                .filter(|automation| {
                    args.get(2)
                        .filter(|id| !id.starts_with("--"))
                        .is_none_or(|id| &automation.id == id)
                })
                .map(|automation| {
                    json!({ "id": automation.id, "runs": automation.runs })
                })
                .collect();
            print(&json!({ "automations": listed }))
        }
        "notices" => print(&json!({ "notices": store.notices })),
        "dismiss" => {
            let run_id = args.get(2).context("missing run id to dismiss")?;
            let cleared = store.dismiss_notice(run_id);
            save_store(&home, &store)?;
            print(&json!({ "dismissed": cleared, "run_id": run_id }))
        }
        "sync" => {
            let mut results = Vec::new();
            for automation in &store.automations {
                results.push(sync_one(automation, &exe, &unit_dir)?);
            }
            let live_ids: Vec<String> = store
                .automations
                .iter()
                .filter(|automation| automation.enabled)
                .map(|automation| automation.id.clone())
                .collect();
            let present: Vec<(PathBuf, String)> = std::fs::read_dir(&unit_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    let name = path.file_name()?.to_str()?.to_string();
                    if !name.starts_with("yggterm-automation-") {
                        return None;
                    }
                    Some((path.clone(), read_file(&path)?))
                })
                .collect();
            let orphans = orphaned_units(&live_ids, &present);
            let pruned = if args.iter().any(|arg| arg == "--prune") {
                orphans
                    .iter()
                    .filter(|path| std::fs::remove_file(path).is_ok())
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            print(&json!({
                "synced": results,
                "orphans": orphans.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
                "pruned": pruned,
                "unit_dir": unit_dir.display().to_string(),
                "hint": "run `systemctl --user daemon-reload` after any change here",
            }))
        }
        other => Err(anyhow!(
            "unknown automation action {other:?}\n\n{}",
            automation_usage_block("yggterm-headless")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::DEFAULT_GRACE_SECS;
    use std::cell::RefCell;
    use time::{Date, Month, PrimitiveDateTime, Time, UtcOffset};

    const IST: i32 = 5 * 3600 + 1800;

    /// `--prompt` still reads normally; only the stdin arm gained a refusal.
    /// (The empty-stdin arm itself is not unit-tested: it reads the process's
    /// real stdin, which a test harness does not own — the refusal is asserted
    /// on the message text a caller would see instead.)
    #[test]
    fn a_prompt_flag_is_read_without_touching_stdin() {
        let args = vec!["--prompt".to_string(), "do the thing".to_string()];
        assert_eq!(
            read_prompt(&args).expect("reads"),
            Some("do the thing".to_string())
        );
        assert_eq!(read_prompt(&[]).expect("reads"), None);
    }

    #[test]
    fn the_empty_stdin_refusal_names_the_flag_and_both_ways_to_fix_it() {
        let source = include_str!("automation_cli.rs");
        assert!(source.contains("--prompt-stdin was given but stdin was empty"));
        // A refusal that does not say what to do instead is a dead end.
        assert!(source.contains("--prompt <text>"));
    }

    fn at(year: i32, month: Month, day: u8, hour: u8, minute: u8) -> u64 {
        let date = Date::from_calendar_date(year, month, day).unwrap();
        let clock = Time::from_hms(hour, minute, 0).unwrap();
        (PrimitiveDateTime::new(date, clock)
            .assume_offset(UtcOffset::from_whole_seconds(IST).unwrap())
            .unix_timestamp() as u64)
            * 1000
    }

    fn job() -> Automation {
        Automation {
            id: "infra-upgrade".to_string(),
            enabled: true,
            agent_kind: yggterm_core::SessionKind::ClaudeCode,
            machine_key: "jojo".to_string(),
            cwd: Some("/home/user/gh/yggterm".to_string()),
            prompt: "some time has passed, can you upgrade again".to_string(),
            calendar: parse_calendar("Sun *-*-* 00:00:00").unwrap(),
            every_n_weeks: 1,
            anchor_week: 0,
            grace_secs: DEFAULT_GRACE_SECS,
            idle_ttl_secs: DEFAULT_IDLE_TTL_SECS,
            deadline_secs: DEFAULT_DEADLINE_SECS,
            attach: false,
            title: None,
            created_at_ms: 0,
            last_run_at_ms: None,
            runs: Vec::new(),
        }
    }

    /// A recording spawner, so every test asserts on what the executor DID
    /// rather than on what a helper returned.
    struct Spy {
        opened: RefCell<Vec<String>>,
        injected: RefCell<Vec<(String, String)>>,
        open_fails: bool,
        inject_fails: bool,
    }

    impl Spy {
        fn new() -> Self {
            Self {
                opened: RefCell::new(Vec::new()),
                injected: RefCell::new(Vec::new()),
                open_fails: false,
                inject_fails: false,
            }
        }
        fn run(&self, automation: &mut Automation, force: bool, now: u64) -> RunReport {
            execute_run(
                automation,
                force,
                now,
                IST,
                &|a| {
                    if self.open_fails {
                        return Err(anyhow!("no GUI worker answered"));
                    }
                    self.opened.borrow_mut().push(a.id.clone());
                    Ok(format!("live/{}/0", a.machine_key))
                },
                &|session, prompt| {
                    if self.inject_fails {
                        return Err(anyhow!("prompt never landed"));
                    }
                    self.injected
                        .borrow_mut()
                        .push((session.to_string(), prompt.to_string()));
                    Ok(())
                },
            )
        }
    }

    #[test]
    fn a_run_on_time_opens_a_session_and_delivers_the_prompt() {
        let spy = Spy::new();
        let mut automation = job();
        let report = spy.run(&mut automation, false, at(2026, Month::August, 2, 0, 1));
        assert_eq!(report.outcome, RunOutcome::Ran);
        assert_eq!(spy.opened.borrow().len(), 1);
        assert_eq!(
            spy.injected.borrow()[0].1,
            "some time has passed, can you upgrade again"
        );
        assert_eq!(automation.runs.len(), 1);
        assert!(automation.open_run().is_some());
    }

    #[test]
    fn a_late_fire_outside_grace_opens_nothing_at_all() {
        let spy = Spy::new();
        let mut automation = job();
        // Wednesday afternoon: the last due Sunday was days ago.
        let report = spy.run(&mut automation, false, at(2026, Month::August, 5, 14, 0));
        assert_eq!(report.outcome, RunOutcome::SkippedOutOfGrace);
        assert!(
            spy.opened.borrow().is_empty(),
            "a skipped run must not open a session — that is the whole point of the guard"
        );
    }

    #[test]
    fn force_ignores_the_guards_because_run_now_and_a_test_both_want_that() {
        let spy = Spy::new();
        let mut automation = job();
        let report = spy.run(&mut automation, true, at(2026, Month::August, 5, 14, 0));
        assert_eq!(report.outcome, RunOutcome::Ran);
        assert_eq!(spy.opened.borrow().len(), 1);
    }

    #[test]
    fn an_off_week_opens_nothing_and_is_recorded_as_such() {
        let spy = Spy::new();
        let mut automation = job();
        automation.every_n_weeks = 2;
        let first = at(2026, Month::August, 2, 0, 1);
        automation.anchor_week = iso_week_number(first, IST).unwrap() + 1;
        let report = spy.run(&mut automation, false, first);
        assert_eq!(report.outcome, RunOutcome::SkippedOffCadence);
        assert!(spy.opened.borrow().is_empty());
    }

    #[test]
    fn a_still_live_session_is_re_prompted_never_duplicated() {
        // E1. A second row for the same job is exactly the leak this feature
        // exists to prevent.
        let spy = Spy::new();
        let mut automation = job();
        spy.run(&mut automation, false, at(2026, Month::August, 2, 0, 1));
        let report = spy.run(&mut automation, false, at(2026, Month::August, 9, 0, 1));
        assert_eq!(report.outcome, RunOutcome::ReusedLiveSession);
        assert_eq!(
            spy.opened.borrow().len(),
            1,
            "the second run must not have opened a second row"
        );
        assert_eq!(spy.injected.borrow().len(), 2);
    }

    #[test]
    fn a_closed_session_is_not_reused_and_the_next_run_spawns_fresh() {
        let spy = Spy::new();
        let mut automation = job();
        spy.run(&mut automation, false, at(2026, Month::August, 2, 0, 1));
        automation.runs[0].closed_at_ms = Some(at(2026, Month::August, 2, 1, 0));
        automation.runs[0].close_reason = CloseReason::EphemeralIdleTtl;
        let report = spy.run(&mut automation, false, at(2026, Month::August, 9, 0, 1));
        assert_eq!(report.outcome, RunOutcome::Ran);
        assert_eq!(spy.opened.borrow().len(), 2);
    }

    #[test]
    fn a_failed_spawn_is_recorded_with_its_reason_and_exits_non_zero() {
        let mut spy = Spy::new();
        spy.open_fails = true;
        let mut automation = job();
        let report = spy.run(&mut automation, false, at(2026, Month::August, 2, 0, 1));
        assert_eq!(report.outcome, RunOutcome::SpawnFailed);
        assert!(!report.outcome.is_success());
        assert!(
            automation.runs[0].error.as_deref().unwrap().contains("no GUI worker"),
            "the run must carry WHY, so `automation runs` can answer it later"
        );
    }

    #[test]
    fn a_row_that_opened_but_never_got_its_prompt_is_still_recorded_as_a_row() {
        // Otherwise the reaper owns a row nothing in the store admits to, and
        // the user has an orphan with no explanation.
        let mut spy = Spy::new();
        spy.inject_fails = true;
        let mut automation = job();
        let report = spy.run(&mut automation, false, at(2026, Month::August, 2, 0, 1));
        assert_eq!(report.outcome, RunOutcome::SpawnFailed);
        assert_eq!(report.session_path.as_deref(), Some("live/jojo/0"));
        assert!(automation.open_run().is_some());
    }

    #[test]
    fn a_skipped_run_still_lands_in_the_history_so_silence_is_never_the_answer() {
        let spy = Spy::new();
        let mut automation = job();
        spy.run(&mut automation, false, at(2026, Month::August, 5, 14, 0));
        assert_eq!(automation.runs.len(), 1);
        assert_eq!(automation.runs[0].outcome, RunOutcome::SkippedOutOfGrace);
        assert!(automation.runs[0].session_path.is_none());
    }

    #[test]
    fn the_run_id_is_stable_for_one_due_instant_so_a_retry_is_not_a_new_run() {
        let spy = Spy::new();
        let mut first = job();
        let mut second = job();
        let now = at(2026, Month::August, 2, 0, 1);
        spy.run(&mut first, false, now);
        spy.run(&mut second, false, now + 60_000);
        assert_eq!(first.runs[0].run_id, second.runs[0].run_id);
    }

    #[test]
    fn the_spawn_declares_a_ttl_only_tenancy_and_never_an_owner_pid() {
        // Naming this process as the owner would have the reaper close the
        // session within a minute — this process exits as soon as the run is
        // recorded, which is the opposite of what an all-night job wants.
        let automation = job();
        let declaration = EphemeralDeclaration::new(
            None,
            "test-host",
            Some(automation.idle_ttl_secs),
        );
        assert!(declaration.owner_pid.is_none());
        assert!(declaration.declares_a_rule());
        assert_eq!(declaration.idle_ttl_secs, Some(DEFAULT_IDLE_TTL_SECS));
    }

    #[test]
    fn the_kind_flag_matches_what_terminal_new_accepts() {
        // These strings are handed straight to `--kind`; a mismatch here is a
        // create that fails at midnight.
        let mut automation = job();
        assert_eq!(session_kind_flag(&automation), "claude-code");
        automation.agent_kind = yggterm_core::SessionKind::Codex;
        assert_eq!(session_kind_flag(&automation), "codex");
        automation.agent_kind = yggterm_core::SessionKind::Shell;
        assert_eq!(session_kind_flag(&automation), "shell");
    }

    #[test]
    fn the_usage_block_names_the_binary_it_was_asked_about() {
        assert!(automation_usage_block("yggterm-headless").contains("yggterm-headless automation run"));
        assert!(automation_usage_block("yggterm").contains("yggterm automation run"));
    }
}
