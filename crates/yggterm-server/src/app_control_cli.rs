//! THE `server app …` CLI — one owner, both binaries.
//!
//! The top-level `server app` dispatch used to be a `match args[2]` COPIED
//! into `apps/yggterm/src/main.rs` AND `apps/yggterm/src/bin/yggterm-headless.rs`.
//! Not one function with two entry points — two copies, which had already
//! drifted by 6 top-level verbs: `audio` and `theme` existed only on the GUI
//! binary, and `chrome`, `row-set`, `row-expanded` and `split` only on the
//! headless one — **the copy every agent skill tells agents to drive**.
//!
//! ⛔ **The failure is silent and every instrument agrees with it.** A verb
//! added to one binary answers `unsupported app control command: <verb>` from
//! the other while `--build-commit` matches the deploy, the arm is visibly in
//! the source, and the running GUI's `/proc/<pid>/exe` md5 matches disk. There
//! is nothing to notice.
//!
//! This is the same collapse [`crate::app_control_web_cli`] already made for
//! the `web` sub-plane, for the same reason and with the same shape, and its
//! module note asked that no verb be inlined next to it again. Extending it to
//! the whole surface is the rest of that job.
//!
//! **What is genuinely per-binary is ONE cell, not the table** — how the app
//! gets LAUNCHED. The GUI binary spawns it in-process; the headless CLI must
//! ask a GUI companion. That fork is real, so it is named
//! ([`AppControlHost::launch_app`]) rather than duplicated, and everything else
//! has exactly one owner. ⇒ When a `match` arm is per-binary, ask whether the
//! TABLE forks or a single CELL does; only the cell did here.

use anyhow::Context;
use std::io::Read;

/// This module's own source, for the locks that assert the `server app` plane
/// has exactly one dispatcher and that it is this one.
pub(crate) const SOURCE_FOR_LOCKS: &str = include_str!("app_control_cli.rs");
use crate::{
    AppControlPreviewLayout,
    AppControlRightPanelMode,
    AppControlViewMode,
    ProbeTerminalViewportInputMode,
    ScreenshotPostProcess,
    run_app_control_background_window,
    run_app_control_close_window,
    run_app_control_close_window_preserving_sessions,
    run_app_control_create_split_group,
    run_app_control_create_terminal_with_tenancy,
    run_app_control_describe_rows,
    run_app_control_describe_state,
    run_app_control_reorder_sessions,
    run_app_control_desktop_identity,
    run_app_control_dom_eval,
    run_app_control_drag,
    run_app_control_dump_state,
    run_app_control_focus_split_pane,
    run_app_control_focus_window,
    run_app_control_grid,
    run_app_control_invoke_command,
    run_app_control_key,
    run_app_control_list_clients,
    run_app_control_list_commands,
    run_app_control_memory_profile,
    run_app_control_move_window_by,
    run_app_control_launch_app,
    run_app_control_open_path,
    run_app_control_paste_terminal_clipboard,
    run_app_control_paste_terminal_clipboard_image,
    run_app_control_pointer,
    run_app_control_probe_chrome_input,
    run_app_control_probe_terminal_context_menu,
    run_app_control_probe_terminal_primary_selection_paste,
    run_app_control_probe_terminal_viewport_input,
    run_app_control_probe_terminal_viewport_scroll,
    run_app_control_probe_terminal_viewport_select,
    run_app_control_reclaim_terminal_focus,
    run_app_control_read_terminal_buffer,
    run_app_control_reconcile_terminal_from_daemon,
    run_app_control_redraw_terminal,
    run_app_control_remove_session,
    run_app_control_rename_session,
    run_app_control_reset_theme_editor,
    run_app_control_resize_window,
    run_app_control_restart_pending_update,
    run_app_control_restart_session,
    run_app_control_scroll_preview,
    run_app_control_scroll_right_panel,
    run_app_control_scroll_terminal_viewport,
    run_app_control_send_terminal_input,
    run_app_control_set_clipboard_png_base64,
    run_app_control_set_clipboard_text,
    run_app_control_set_force_foreground,
    run_app_control_set_fullscreen,
    run_app_control_set_main_zoom,
    run_app_control_set_maximized,
    run_app_control_app_pane_action,
    run_app_control_set_right_panel_mode,
    run_app_control_arrange_row_set,
    run_app_control_set_row_expanded,
    run_app_control_set_search,
    run_app_control_set_session_keep_alive,
    run_app_control_set_launch_flags,
    run_app_control_set_split_group_ratio,
    run_app_control_set_theme_editor_open,
    run_app_control_set_theme_editor_values,
    run_app_control_set_preview_layout,
    run_app_control_set_tree_selection,
    run_app_control_set_ui_theme,
    run_app_control_set_window_chrome_hover,
    run_app_control_show_start_page,
    run_app_control_split_web_tab,
    run_app_control_start_action,
    run_app_control_check_terminal_input,
    run_app_control_submit_terminal_prompt,
    run_app_control_trigger_update_check,
    run_app_control_ungroup_split_group,
    run_screenrecord_capture,
    run_screenshot_capture,
    run_screenshot_capture_with_post_process,
};
use yggterm_core::{cli_flag_value, cli_positional_args};
use yggui_contract::UiTheme;


/// The one genuinely per-binary operation behind `server app`.
///
/// Everything else in this plane is identical for both callers, so it lives
/// here once. Launching is not: the GUI binary already owns a window and
/// spawns the app itself, while the headless CLI has no GUI and must ask a
/// companion to do it. Naming the fork keeps the other ~75 verbs shared.
pub trait AppControlHost {
    /// How this binary spells itself in `--help`, so one usage text serves both.
    fn binary_name(&self) -> &'static str;

    /// `server app launch …`.
    fn launch_app(
        &self,
        args: &[String],
        home_dir: &std::path::Path,
        timeout_ms: u64,
    ) -> anyhow::Result<()>;
}

fn app_control_close_preserve_flag(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--preserve-live-sessions" | "--preserve-sessions" | "--handoff" | "--restart-safe"
        )
    })
}
/// Read a verb's PAYLOAD argument (a script, a value, `-` for stdin).
///
/// Lived in the GUI binary as a private helper while the headless binary
/// inlined the same reads, which is the duplication in miniature. It is an
/// `anyhow` adapter over [`yggterm_core::cli_payload_arg`], the real owner.
pub fn app_control_payload_arg<'a>(
    args: &'a [String],
    start: usize,
    what: &str,
) -> anyhow::Result<&'a str> {
    yggterm_core::cli_payload_arg(args, start, what).map_err(|error| anyhow::anyhow!(error))
}

fn screenshot_backend_is_compositor(args: &[String]) -> bool {
    cli_flag_value(args, "--backend")
        .map(|value| value.eq_ignore_ascii_case("os"))
        .unwrap_or(false)
}
fn screenshot_post_process_from_args(args: &[String]) -> Option<ScreenshotPostProcess> {
    let region = cli_flag_value(args, "--region").map(str::to_string);
    let crop = cli_flag_value(args, "--crop").and_then(|raw| {
        let parts: Vec<u32> = raw
            .split(',')
            .filter_map(|piece| piece.trim().parse::<u32>().ok())
            .collect();
        if parts.len() == 4 {
            Some((parts[0], parts[1], parts[2], parts[3]))
        } else {
            None
        }
    });
    let scale = cli_flag_value(args, "--scale").and_then(|raw| raw.parse::<f32>().ok());
    // `--grid [COLSxROWS]` / `--grid-refine CELL`: the agent-only click grid,
    // composited into the RETURNED IMAGE only — the live page never sees it.
    let grid = crate::grid_overlay::screenshot_grid_from_args(args);
    if region.is_none() && crop.is_none() && scale.is_none() && grid.is_none() {
        return None;
    }
    Some(ScreenshotPostProcess {
        region,
        crop,
        scale: scale.unwrap_or(1.0),
        grid,
    })
}


/// The `server app` usage text, rendered for whichever binary is asking.
///
/// ⛔ ONE text for both binaries, for the same reason there is one dispatcher.
/// They each carried their own before, so the GUI's help documented `audio`
/// that the headless binary could not run, and the headless help documented
/// `row-set` that the GUI could not — the help drifted alongside the dispatch
/// because it was the same duplication a second time. The web and
/// delegate-launch planes were already rendered by their owners here; this
/// extends that to the surface around them.
pub fn server_app_usage_block(binary: &str) -> String {
    format!(
        "usage:
  {binary} server app audio play [--tone info|success|warning|error] [--repeat n]
                                [--gap-ms n] [--preroll on|off|auto] [--volume 0..1]
  {binary} server app audio tune --notes '[[startSec,freqHz,peak], …]'
    NATIVE audio (no webview, no GUI needed): WebKitGTK's autoplay gate streams
    silent samples without a user gesture, which an agent cannot produce.
    `server app audio --help` has the tone patterns and the tune's provenance
  {binary} server app clients
  {binary} server app desktop-identity
  {binary} server app state [--pid <pid>]
  {binary} server app rows [--pid <pid>]
  {binary} server app row-set <row-path> [--into <head>|--out|--dissolve|--reset]
    arranges a live row into or out of a ROW SET — the verb twin of dragging a
    row onto another and of right-click un-group. `--into` files it under that
    head, `--out` takes it to the top level, `--dissolve` breaks up the set this
    row heads and promotes its members to where the head sat. `--reset` forgets
    the hand's answer so the row's SEAT decides again — the way back, because
    `--into` and `--out` are both sticky by design. ⛔ It writes MEMBERSHIP,
    never a seat: grouping never renumbers a row, so un-numbered rows group like
    any other.
  {binary} server app drag <begin|hover|drop|clear> [row-path] [--placement before|into|after]
    the SYNTHETIC drag twin of a hand dragging a sidebar row. `begin <row-path>`
    arms the gesture, `hover <target> --placement <p>` moves the drop indicator
    to before/into/after that target, `drop` commits the membership + precise
    position, `clear`/`cancel` aborts. `before` lands before the target inside
    its set, `after` after it, `into` as first child (the three 12px/24px bands
    the pointer hit-tests in the sidebar). This is how an agent verifies a
    reorder without touching the screen — read `server app rows` before/after and
    `drag_paths`/`drag_hover_target` in `server app state`.
  {binary} server app pointer <move|down|up|click|drag|scroll> [--x <px> --y <px>] [--start-x <px> --start-y <px> --end-x <px> --end-y <px>] [--button left|right|middle] [--count n] [--steps n] [--step-delay-ms n]
    synthetic pointer gesture for Wayland/KWin where OS-level `xdotool`/`wtype`
    cannot be trusted (and where a real pointer would steal focus). Prefer `drag`
    for row-sets — this is the low-level fallback for pixel work and for probes
    that need an explicit coordinate.
  {binary} server app grid <show|hide|click|hover> [cell] [--cols n --rows n] [--region terminal|full] [--target main|surface|auto] [--ttl-secs n] [--button left|right] [--count n] [--refine] [--keep]
    labelled grid overlay composited into the RETURNED PNG only — the live page
    is never touched. `show` paints a `cols×rows` grid (default 12×8) and `click`
    resolves a cell label to a server-side coordinate hit-test. Safer than
    guessing pixels off a screenshot; composes with `--region`/`--crop`/`--scale`.
  {binary} server app row-expanded <row-path> <true|false>
    opens or shuts a container — a folder, a machine, or a ROW SET's head — the
    way clicking its disclosure control does. Row-set heads are ordinary session
    rows: `server app rows` marks one with a `child_count` above 1.
  {binary} server app sessions reorder <order.json>
    sets the order on the GUI — the process that RENDERS it — and answers with
    the resulting `rendered_order`. `server sessions reorder` writes to whichever
    daemon the CLI resolved, which is not always the one the GUI reads.
  {binary} server app sessions restore <session-path>... [--dry-run] [--include-closed]
    puts NAMED rows back through the same open a click takes, one at a time, and
    REFUSES any the user closed — the deny-list is consulted once for the batch
    and the reply carries `declined_closed_count` plus the paths. There is no
    restore-everything: a restore that guesses its own scope is how a deleted
    row comes back. `--include-closed` restores them anyway and reports them
    under `overridden_closed`, for the case where the close was a relay retiring
    a predecessor rather than the user deleting anything.
  {binary} server app sessions sort [--dry-run]
    re-derives the Live order from the rows' outline numbers and applies it.
    Segments compare as INTEGERS (1 · 1.1 · 2 · 10), unnumbered rows sort last
    and stably, and sorting a sorted list reports changed:false — which is the
    success case, not a no-op to chase.
  {binary} server app session outline <session-path> <prefix>
    numbers a row that already exists and RE-SEATS it (an empty prefix clears
    it). The number is stored apart from the title and composed at render time,
    so a CLI re-titling itself can no longer destroy a position.
  {binary} server app screenshot [output] [--pid <pid>] [--region terminal|full] [--crop x,y,w,h] [--scale n] [--backend os]
  {binary} server app open <session-path> [--view <terminal|preview>] [--pid <pid>]
  {binary} server app resize-window --width <px> --height <px> [--pid <pid>]
  {binary} server app maximize <on|off|toggle> [--pid <pid>]
  {binary} server app force-foreground <on|off> [--pid <pid>]
  {binary} server app session <remove|delete> <session-path> [--pid <pid>]
    answers verified:true only when the row left the live order AND every
    process the session owned is gone; otherwise verified:false with a named
    refusal and the surviving pids in live_processes
  {binary} server app start-page [--pid <pid>]
  {binary} server app notify <title> [message] [--tone info|success|warning|error]
      [--job <key>] [--progress 0..100] [--session <session-path>]
      [--persistent] [--silent] [--in <dur>|--at <clock>] [--pid <pid>]
    THE door for an agent, a cron job or a libyggterm app to reach the user.
    ⛔ It targets the ACTIVE GUI by default — an agent testing one MUST pass
    --pid/--client or its toast lands on the user's own screen.
    --job upserts one row (that is how a long job reports progress without
    burying everything else) and --progress draws the bar; --progress without
    --job is DROPPED. --session makes the card clickable through to that row.
    ⚠ $YGGTERM_SESSION_ID is NOT a row path — match its UUID against
    `server app rows` first, or the card is inert.
  {binary} server app launch-app <app> [verb] [--cwd <dir>] [--insert-after <session-path>]
    launch a libyggterm app's verb through the SAME owner the titlebar `+`,
    the row menu and the start page use. <app> is the registry key
    (`ychrome`, `yedit`); no verb given means the app's first, which is what
    a menu shows first. The reply's `shell.launch_app` block reports what was
    accepted and the real launch command the row was born with.
  {binary} server app pane <pane-id> <action> [value]
    press something in a contributed pane, exactly as a click does —
    `panel pane:<id>` opens one, this is what can act on it. value is
    what the widget would carry: a row id for a row_action, a tab id
    for tabs, absent for a plain button
  {binary} server app update <check|restart>
  {binary} server app chrome type <selector> --data <text> [--clear] [--enter]
      [--assert <selector>@<attribute>]
    types into the SHELL'S OWN chrome — the start page search box, a rename
    field — which no other verb can reach: probe-type targets a PTY, web/wpe
    drive a contributed app's page, and pointer/click reach a coordinate. Name
    the field by its stamp (`'[data-yggterm-start-page-search]'`).
    --assert reads an attribute BEFORE the keystroke and again after the render
    settles, in the SAME evaluation, and reports `assert_changed`. ⚠ Two reads
    taken at two different moments cannot prove the field drove a re-render —
    the difference between them may be time rather than the typing. The reply
    also carries value_before/value_after: `accepted:true` with the value
    unchanged is a field that REFUSED the input, not a success.
  {binary} server app terminal <new|send|input-check|focus|scroll|read-buffer|probe-type|probe-scroll|probe-select|probe-context-menu> ...
  {binary} server app terminal read-buffer <session> [--mode screen|full|cells]
    READ a row's screen without touching it — the only way to see what a row is
    showing before deciding whether to type into it. The watchdog's plan-limit
    guard is built on this: a row parked on a billing dialog must be READ, never
    guessed at. ⛔ It answers `terminal_host_missing` for a `--no-activate` row
    that nothing has ever mounted.
  {binary} server app terminal scroll <session> --to <top|bottom|±N>
  {binary} server app theme <light|dark>
  {binary} server app web-view layout <chat|graph|overview>
  {binary} server app terminal new [--machine-key <key>] [--cwd <dir>] [--kind <shell|codex|claude-code>] [--title <title>] [--purpose <what-for>] [--no-activate]
      [--outline <prefix> | --insert-after <session-path>]
    with no --title the row is named for the driving agent and its purpose.
    --outline seats the row at a stored number that survives restarts and
    re-titles; --insert-after places it below an anchor row this once. The seat
    is applied INSIDE the create, and the reply's `seat.honoured` is RE-READ
    from the rendered order rather than echoed from the request. Passing both,
    or a prefix that is not a dotted number, is refused BY NAME before the row
    is created.
  {binary} server app terminal input-check <session> [--check-timeout-ms <ms>]
    Is this row CONSUMING INPUT? Answers and submits NOTHING, so it is safe to
    point at a row the owner is using. A wedged agent row is ALIVE, its turn has
    ENDED and it draws its composer, so every other signal calls it healthy and
    `send` into it answers `error: null` while delivering nothing. `wedged:true`
    is a POSITIVE claim (composer displayed AND no echo), never merely quiet;
    a busy row mid-output answers `composer_shown:false` instead. Refuses by
    name when the composer holds an unsent draft — the probe clears the line.
  {binary} server app terminal send <session> (--data <data>|--stdin) [--allow-multiline]
    a payload with interior line breaks is REFUSED for an agent CLI row: its
    composer reads every \r as Enter, so line 1 submits alone and the rest
    become queued messages. Use `terminal submit` for a brief, or
    --allow-multiline to fire N separate submits deliberately. Shell rows are
    unaffected — there N lines are N commands.
  {binary} server app terminal new [--kind <shell|codex|claude-code>] [--cwd <dir>] [--title <t>]
      [--machine-key <k>] [--no-activate] [--purpose <text>]
      [--model <id>] [--permission-mode <default|plan|accept-edits|bypass>]
      [--prompt <text>|--prompt-stdin]
      [--ephemeral (--ephemeral-owner-pid <pid> | --ephemeral-idle-ttl-secs <n>)]
  {binary} server app terminal adopt --target-pid <pid> [--title <title>] [--machine-key <key>] [--cwd <dir>] [--no-activate]
    (also --outer-pid <pid> or legacy --pid <pid>; --pid alone is the outer PTY, not the GUI target — use --target-pid to avoid the GUI --pid collision)
    attach an existing outer PTY (for example a `muse --yolo` on /dev/pts/N) into a new daemon-owned yggterm row. The row is a shell that then `reptyr -T <pid>`s the target (Linux-only, needs ptrace; if the kernel refuses the verb reports `adopt_refused` with the `reptyr` stderr instead of leaving a blank shell). The new row is `local-shell` on the target machine and appears as `remote-*://` on other hosts — the same visibility path as any `terminal new`.
{delegate_usage}
  {binary} server app keytips <audit [--json]|show|hide>
  {binary} server app media answer <allow|deny-once|block-site> [--request <id>]
    answers the camera/microphone prompt `server app state` reports under
    pending_media_capture; non-zero exit + a named reason when it was NOT applied
{web_usage}
row tenancy (server app terminal new): every create from this CLI is stamped
  with the creating pid, this host, and --purpose if given; read it back with
  `server terminal tenants`. --ephemeral additionally OPTS IN to reaping, and it
  is REFUSED on its own: it needs --ephemeral-owner-pid <pid> naming a process
  you KNOW outlives the create (your own pid), or --ephemeral-idle-ttl-secs <n>
  for a TTL-only rule, or both. There is no default owner — under
  `bash -c \"<cli>\"` the parent is the wrapper bash and under `ssh host \"<cli>\"`
  it is sshd-session, both dead within milliseconds, so a defaulted owner would
  reap the row on the next tick. The row is then closed gracefully (tombstone +
  trace) once the owner pid leaves /proc, or after n seconds with no output.
  A TTL-only declaration names no owner and is never owner-reaped. Keep-alive
  does NOT shield a declared row: keep-alive governs whether a runtime survives
  the GUI WINDOW closing, and an explicit close — which is what a reap is —
  takes a keep-alive row like any other. Rows created any other way, and rows
  with no declaration, are never reaped. The check rides an existing daemon
  chore tick (12-60 s), so a rule fires within about a minute of becoming true,
  never instantly. Every flag takes --flag value or --flag=value.

targeting (any app verb): [--pid <pid>] or [--client <name>] picks which GUI
  worker handles the verb; --client names a client by its --client-id (a shadow
  view client, slice 4.3) — see `server app clients`. --pid wins if both given;
  with one GUI and no target it routes there automatically.
  {binary} server app theme <light|dark>",
        binary = binary,
        web_usage = crate::web_usage_block(binary),
        delegate_usage = crate::delegate_launch_usage_block(binary),
    )
}

pub fn print_server_app_help(binary: &str) {
    println!("{}", server_app_usage_block(binary));
}

/// Dispatch one `server app <verb> …` invocation. `args` is the full argv tail
/// beginning at `server`.
pub fn run_app_control_cli(
    args: &[String],
    home_dir: &std::path::Path,
    host: &dyn AppControlHost,
) -> anyhow::Result<()> {
    // ONE owner for how a verb names its GUI target: an explicit
    // `--pid`/`--client` on this invocation wins (`--pid` beats `--client`
    // downstream in `choose_app_control_pid`), and with no flag the
    // exported YGGTERM_APP_CONTROL_PID/_CLIENT stands. The inline block
    // this replaces REMOVED the exported variable whenever the flag was
    // absent, which is why it worked for one verb and not another
    // (field report A5, 2026-07-28).
    crate::apply_app_control_target_overrides(&args);
    let timeout_ms = args
        .windows(2)
        .find_map(|window| {
            if window[0] == "--timeout-ms" {
                window[1].parse::<u64>().ok()
            } else {
                None
            }
        })
        .unwrap_or(15_000);
    match args[2].as_str() {
        "--help" | "-h" | "help" => {
            print_server_app_help(host.binary_name());
            Ok(())
        }
        "screenshot" => {
            let target = args
                .windows(2)
                .find_map(|window| {
                    if window[0] == "--target" {
                        Some(window[1].as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("app");
            let output_path = cli_positional_args(&args, 3)
                .into_iter()
                .find(|value| *value != target);
            let compositor = screenshot_backend_is_compositor(&args);
            match (screenshot_post_process_from_args(&args), compositor) {
                (None, false) => run_screenshot_capture(target, output_path, timeout_ms),
                (post, compositor) => run_screenshot_capture_with_post_process(
                    target,
                    output_path,
                    timeout_ms,
                    post.unwrap_or(ScreenshotPostProcess {
                        region: None,
                        crop: None,
                        scale: 1.0,
                        grid: None,
                    }),
                    compositor,
                ),
            }
        }
        "screenrecord" => {
            let duration_secs = args
                .windows(2)
                .find_map(|window| {
                    if window[0] == "--duration-sec" {
                        window[1].parse::<u64>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(10);
            let output_path = cli_positional_args(&args, 3).into_iter().next();
            run_screenrecord_capture("app", output_path, timeout_ms, duration_secs)
        }
        "launch" => {
            app_launch_duplicate_guard_check(&args, home_dir)?;
            host.launch_app(&args, home_dir, timeout_ms)
        }
        "clients" => run_app_control_list_clients(),
        "desktop-identity" => run_app_control_desktop_identity(),
        // A LOCAL /proc walk, not an app-control round trip: the profile is
        // most needed when the GUI is too loaded to answer a socket.
        "memory" | "mem" => {
            run_app_control_memory_profile(
                args.iter().any(|arg| arg == "--json"),
                args.iter().any(|arg| arg == "--sweep"),
            )
        }
        "state" => run_app_control_describe_state(timeout_ms),
        "dump" => {
            let output_path = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing output path for server app dump"))?;
            run_app_control_dump_state(output_path, timeout_ms)
        }
        "rows" => run_app_control_describe_rows(timeout_ms),
        // `server app row-expanded <row-path> <true|false>` — open or shut a
        // container from a verb, the way a click does it.
        //
        // ⛔ THE PROTOCOL COMMAND AND THE SHELL HANDLER BOTH EXISTED AND
        // NOTHING CALLED THEM. `run_app_control_set_row_expanded` had no
        // caller at all, so `server app rows` reported an `expanded` field
        // that no verb on the command line could change — the half of
        // "an agent arranges rows as easily as a hand does" that was
        // written and then never wired to a name a caller could type.
        // `server app row-set <row-path> [--into <head>|--out|--dissolve]`
        // — the verb twin of the inside-band drag and the right-click
        // un-group. DESIGN.md: both halves exist or neither is real.
        "row-set" => {
            let positional = cli_positional_args(&args, 3);
            let row_path = positional.first().ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: server app row-set <row-path> [--into <head-path>|--out|--dissolve]"
                )
            })?;
            let into = cli_flag_value(&args, "--into");
            let dissolve = args.iter().any(|arg| arg == "--dissolve");
            let out = args.iter().any(|arg| arg == "--out");
            let reset = args.iter().any(|arg| arg == "--reset");
            // Named rather than defaulted: guessing between "file this
            // under something" and "take it out" would silently do the
            // opposite of what the caller meant half the time.
            if into.is_none() && !dissolve && !out && !reset {
                anyhow::bail!(
                    "server app row-set needs one of --into <head-path>, --out, \
                     --dissolve or --reset"
                );
            }
            if into.is_some() && (dissolve || out || reset) {
                anyhow::bail!("--into cannot be combined with --out, --dissolve or --reset");
            }
            run_app_control_arrange_row_set(row_path, into, dissolve, reset, timeout_ms)
        }
        "row-expanded" => {
            let positional = cli_positional_args(&args, 3);
            let row_path = positional.first().ok_or_else(|| {
                anyhow::anyhow!("usage: server app row-expanded <row-path> <true|false>")
            })?;
            let expanded = match positional.get(1).copied() {
                Some("true") => true,
                Some("false") => false,
                // Named rather than defaulted: guessing here would silently
                // do the opposite of what the caller meant.
                other => anyhow::bail!(
                    "server app row-expanded needs `true` or `false`, got {}",
                    other.unwrap_or("nothing")
                ),
            };
            run_app_control_set_row_expanded(row_path, expanded, timeout_ms)
        }
        "sessions" if args.get(3).map(String::as_str) == Some("reorder") => {
            // `server app sessions reorder <order.json>` — the APP-path twin
            // of `server sessions reorder`. Same file format; the difference
            // is which process it reaches, and only this one reaches the
            // list the user is looking at.
            let order_path = args
                .get(4)
                .ok_or_else(|| anyhow::anyhow!(
                    "usage: server app sessions reorder <order.json>"
                ))?;
            let raw = std::fs::read_to_string(order_path)
                .with_context(|| format!("reading order file {order_path}"))?;
            let ordered_paths: Vec<String> = serde_json::from_str(&raw)
                .with_context(|| format!("{order_path} must be a JSON array of session paths"))?;
            if ordered_paths.is_empty() {
                anyhow::bail!("{order_path} is empty; refusing to clear the row order");
            }
            run_app_control_reorder_sessions(ordered_paths, timeout_ms)
        }
        // `server app sessions sort [--dry-run]` — the owner's shortcut:
        // re-derive the Live order from the rows' outline numbers and apply
        // it, through the same path a manual drag takes.
        // `server app sessions restore <session-path>... [--dry-run]` —
        // the recovery verb, with the user's deletions honoured. Every path
        // is checked against the tombstone plane BEFORE anything is opened,
        // and the reply names how many it declined; without that a restore
        // hands back the rows the user deliberately deleted.
        "sessions" if args.get(3).map(String::as_str) == Some("restore") => {
            let dry_run = args.iter().any(|arg| arg == "--dry-run");
            // ⚖ The override is NAMED, not implied. A relay legitimately
            // retires its predecessor's row with `session remove`, so a
            // deliberately-closed row is sometimes exactly the one someone
            // wants back — but the default must stay the deny-list, or the
            // verb is back to being the loop it replaced.
            let include_closed = args.iter().any(|arg| arg == "--include-closed");
            let session_paths = cli_positional_args(&args, 4)
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            crate::run_app_control_restore_sessions(
                session_paths,
                dry_run,
                include_closed,
                timeout_ms,
            )
        }
        "sessions" if args.get(3).map(String::as_str) == Some("sort") => {
            crate::run_app_control_sort_sessions(
                args.iter().any(|arg| arg == "--dry-run"),
                timeout_ms,
            )
        }
        "preview" | "web-view" | "webview" => {
            let action = args.get(3).map(String::as_str).unwrap_or("scroll");
            match action {
                "scroll" => {
                    let top_px = args.windows(2).find_map(|window| {
                        if window[0] == "--top" {
                            window[1].parse::<f64>().ok()
                        } else {
                            None
                        }
                    });
                    let ratio = args.windows(2).find_map(|window| {
                        if window[0] == "--ratio" {
                            window[1].parse::<f64>().ok()
                        } else {
                            None
                        }
                    });
                    run_app_control_scroll_preview(top_px, ratio, timeout_ms)
                }
                // Lost in the same consolidation as `terminal read-buffer`.
                // `AppControlCommand::SetPreviewLayout` survived it, so the
                // protocol could still carry the request and nothing could
                // send one — which is the shape the reachability lock below
                // now refuses.
                "layout" => {
                    let layout = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .unwrap_or("chat");
                    let layout = match layout {
                        "chat" => AppControlPreviewLayout::Chat,
                        "graph" | "overview" => AppControlPreviewLayout::Graph,
                        other => anyhow::bail!("unsupported app web view layout: {other}"),
                    };
                    run_app_control_set_preview_layout(layout, timeout_ms)
                }
                other => anyhow::bail!("unsupported app web view action: {other}"),
            }
        }
        "zoom" => {
            let value = args
                .windows(2)
                .find_map(|window| {
                    (window[0] == "--value").then(|| window[1].parse::<f32>().ok())
                })
                .flatten()
                .ok_or_else(|| anyhow::anyhow!("missing --value for server app zoom"))?;
            let view_mode = args.windows(2).find_map(|window| {
                if window[0] != "--view" {
                    return None;
                }
                match window[1].as_str() {
                    "preview" | "rendered" | "web-view" | "webview" => {
                        Some(AppControlViewMode::Preview)
                    }
                    "terminal" => Some(AppControlViewMode::Terminal),
                    _ => None,
                }
            });
            run_app_control_set_main_zoom(value, view_mode, timeout_ms)
        }
        "expand" | "collapse" => {
            let row_path = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .ok_or_else(|| {
                    anyhow::anyhow!("missing row path for server app expand/collapse")
                })?;
            run_app_control_set_row_expanded(row_path, args[2] == "expand", timeout_ms)
        }
        "focus" => run_app_control_focus_window(timeout_ms),
        "background" | "minimize" => run_app_control_background_window(timeout_ms),
        "move-window" | "move-by" | "nudge" => {
            let delta_x = args.windows(2).find_map(|window| {
                if window[0] == "--delta-x" || window[0] == "--dx" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let delta_y = args.windows(2).find_map(|window| {
                if window[0] == "--delta-y" || window[0] == "--dy" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            run_app_control_move_window_by(
                delta_x.context("missing --delta-x/--dx for server app move-window")?,
                delta_y.context("missing --delta-y/--dy for server app move-window")?,
                timeout_ms,
            )
        }
        "resize-window" | "set-window-size" | "size" => {
            let width = args.windows(2).find_map(|window| {
                if window[0] == "--width" || window[0] == "--w" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let height = args.windows(2).find_map(|window| {
                if window[0] == "--height" || window[0] == "--h" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            run_app_control_resize_window(
                width.context("missing --width/--w for server app resize-window")?,
                height.context("missing --height/--h for server app resize-window")?,
                timeout_ms,
            )
        }
        "close" | "quit" | "exit" => {
            if app_control_close_preserve_flag(&args) {
                run_app_control_close_window_preserving_sessions(
                    timeout_ms,
                    Some("manual-preserve-close".to_string()),
                    args.iter().any(|arg| arg == "--force"),
                )
            } else {
                run_app_control_close_window(timeout_ms)
            }
        }
        "chrome-hover" | "titlebar-hover" => {
            let active = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .map(|value| match value {
                    "on" | "true" | "1" | "hover" | "enter" => Some(true),
                    "off" | "false" | "0" | "leave" => Some(false),
                    _ => None,
                })
                .flatten()
                .context("missing or invalid hover state for server app chrome-hover")?;
            run_app_control_set_window_chrome_hover(active, timeout_ms)
        }
        "search" => {
            let action = args.get(3).map(String::as_str).unwrap_or("set");
            match action {
                "set" => {
                    let query = cli_flag_value(&args, "--query")
                        .or_else(|| cli_flag_value(&args, "--value"))
                        .or_else(|| cli_positional_args(&args, 4).into_iter().next())
                        .unwrap_or("");
                    let focused = args.windows(2).find_map(|window| {
                        if window[0] != "--focus" {
                            return None;
                        }
                        match window[1].as_str() {
                            "on" | "true" | "1" => Some(true),
                            "off" | "false" | "0" => Some(false),
                            _ => None,
                        }
                    });
                    run_app_control_set_search(query, focused, timeout_ms)
                }
                "clear" => run_app_control_set_search("", Some(false), timeout_ms),
                other => anyhow::bail!("unsupported app search action: {other}"),
            }
        }
        "clipboard" => {
            let action = args.get(3).map(String::as_str).unwrap_or("text");
            match action {
                "text" | "set" => {
                    let value = cli_flag_value(&args, "--value")
                        .or_else(|| cli_flag_value(&args, "--text"))
                        .or_else(|| {
                            args.iter()
                                .skip(4)
                                .find(|value| !value.starts_with("--"))
                                .map(String::as_str)
                        })
                        .unwrap_or("");
                    run_app_control_set_clipboard_text(value, timeout_ms)
                }
                "png" | "image" | "png-base64" => {
                    let value = cli_flag_value(&args, "--base64")
                        .or_else(|| cli_flag_value(&args, "--value"))
                        .or_else(|| {
                            args.iter()
                                .skip(4)
                                .find(|value| !value.starts_with("--"))
                                .map(String::as_str)
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing --base64/--value for server app clipboard image"
                            )
                        })?;
                    run_app_control_set_clipboard_png_base64(value, timeout_ms)
                }
                other => anyhow::bail!("unsupported app clipboard action: {other}"),
            }
        }
        "launch-app" | "app-launch" => {
            let positional = cli_positional_args(&args, 3);
            let mut positional = positional.into_iter();
            let app = positional.next().ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: server app launch-app <app> [verb] [--cwd <dir>] [--insert-after <session-path>]"
                )
            })?
            .to_string();
            let verb = positional.next().map(ToOwned::to_owned);
            let cwd = cli_flag_value(&args, "--cwd").map(ToOwned::to_owned);
            let insert_after = cli_flag_value(&args, "--insert-after").map(ToOwned::to_owned);
            run_app_control_launch_app(app, verb, cwd, insert_after, timeout_ms)
        }
        "panel" | "right-panel" => {
            let mode = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .unwrap_or("hidden");
            if mode == "scroll" {
                let top_px = args.windows(2).find_map(|window| {
                    if window[0] == "--top" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let ratio = args.windows(2).find_map(|window| {
                    if window[0] == "--ratio" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                return run_app_control_scroll_right_panel(top_px, ratio, timeout_ms);
            }
            let mode = match mode {
                "hidden" | "hide" | "close" | "none" => AppControlRightPanelMode::Hidden,
                "connect" => AppControlRightPanelMode::Connect,
                "notifications" | "notification" => AppControlRightPanelMode::Notifications,
                "settings" => AppControlRightPanelMode::Settings,
                "metadata" | "session-metadata" => AppControlRightPanelMode::Metadata,
                // `pane:<id>` opens a pane the ACTIVE APP contributed over
                // OSC 7717 (e.g. `pane:vault`). yggterm does not know the
                // ids; the app declares them.
                pane if pane.starts_with("pane:") => AppControlRightPanelMode::AppPane {
                    id: pane.trim_start_matches("pane:").to_string(),
                },
                other => anyhow::bail!(
                    "unsupported app right panel mode: {other} \
                     (try hidden|connect|notifications|settings|metadata|pane:<id>)"
                ),
            };
            run_app_control_set_right_panel_mode(mode, timeout_ms)
        }
        // Press something IN a contributed pane. `panel pane:<id>` opens
        // one; this is the verb that could then do nothing to it.
        "pane" => {
            let positional = cli_positional_args(&args, 3);
            let mut positional = positional.into_iter();
            let pane = positional
                .next()
                .ok_or_else(|| {
                    anyhow::anyhow!("usage: server app pane <pane-id> <action> [value]")
                })?
                .to_string();
            let action = positional
                .next()
                .ok_or_else(|| {
                    anyhow::anyhow!("usage: server app pane <pane-id> <action> [value]")
                })?
                .to_string();
            // The value a widget would have carried: a row's id for a
            // `row_action`, a tab's id for `tabs`, absent for a button.
            let value = positional.next().map(str::to_string);
            run_app_control_app_pane_action(pane, action, value, timeout_ms)
        }
        // NOTIFICATIONS. One verb for every plane: an agent, a cron job, a
        // /loop waking up, or a libyggterm app. `--in`/`--at` is the alarm clock.
        "notify" => {
            let positional = cli_positional_args(&args, 3);
            let title = positional
                .first()
                .map(|s| s.to_string())
                .or_else(|| cli_flag_value(&args, "--title").map(str::to_string))
                .context("missing title for server app notify")?;
            let message = positional
                .get(1)
                .map(|s| s.to_string())
                .or_else(|| cli_flag_value(&args, "--message").map(str::to_string))
                .unwrap_or_default();
            let delay_ms = match (cli_flag_value(&args, "--in"), cli_flag_value(&args, "--at")) {
                (Some(spec), _) => Some(crate::parse_duration_ms(spec)?),
                (None, Some(when)) => Some(crate::parse_clock_delay_ms(when)?),
                (None, None) => None,
            };
            let progress = cli_flag_value(&args, "--progress")
                .map(|v| v.parse::<f32>())
                .transpose()
                .map_err(|_| anyhow::anyhow!("--progress takes a number 0..100"))?;
            crate::run_app_control_notify(
                &title,
                &message,
                cli_flag_value(&args, "--tone").as_deref(),
                cli_flag_value(&args, "--job").as_deref(),
                progress,
                args.iter().any(|a| a == "--persistent"),
                args.iter().any(|a| a == "--silent"),
                delay_ms,
                cli_flag_value(&args, "--session").as_deref(),
                timeout_ms,
            )
        }
        "update" => {
            let action = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .unwrap_or("check");
            match action {
                "check" | "trigger" => run_app_control_trigger_update_check(timeout_ms),
                // Refuses while an agent holds a live web-surface lease
                // (`agent_lease_active`) — a deploy that lands mid-flow
                // kills the flow. `--force` says you mean it. Pre-flight
                // with `server app state | jq .agent_leases`.
                "restart" => run_app_control_restart_pending_update(
                    args.iter().any(|arg| arg == "--force"),
                    timeout_ms,
                ),
                other => anyhow::bail!("unsupported app update action: {other}"),
            }
        }
        // The keyboard analogue of the click grid: drive shell commands by
        // their registry id instead of pixel-hunting. `command list`
        // enumerates ids + KeyTips; `command invoke <id>` fires one.
        "command" | "commands" => {
            let positional = cli_positional_args(&args, 3);
            let action = positional.first().copied().unwrap_or("list");
            match action {
                "list" | "ls" => run_app_control_list_commands(timeout_ms),
                "invoke" | "run" => {
                    let id = positional.get(1).copied().ok_or_else(|| {
                        anyhow::anyhow!(
                            "missing <id> for server app command invoke \
                             (try `command list` to see ids)"
                        )
                    })?;
                    run_app_control_invoke_command(id.to_string(), timeout_ms)
                }
                other => anyhow::bail!(
                    "unsupported app command action: {other} (try list|invoke <id>)"
                ),
            }
        }
        // ✅ THIS ARM IS WHERE THE SPLIT DISPATCH WAS FOUND, AND IT IS NOW
        // CLOSED. The account, kept because it is the clearest statement of
        // the failure: `server app` used to be dispatched twice — once in the
        // GUI binary, once here — so a verb added to one was ABSENT from the
        // other while every instrument agreed the code had shipped. The binary
        // carried the arm, `--build-commit` matched the deploy, and
        // `server app launch-flags` still answered "unsupported app control
        // command". There was nothing to notice.
        // ⇒ The two dispatchers are one file now, this one, and
        // `neither_binary_dispatches_server_app_itself` fails the build if a
        // second one appears. A new verb goes here and is reachable from both
        // binaries by construction.
        // ⛔ AND `audio` AND `theme` ARE THE TWO THIS FILE'S OWN COMMIT NAMED
        // AND THEN DROPPED. The consolidation existed because they answered
        // only from the GUI binary; both came out of it answering from
        // NEITHER, and the comment above still described the fix. A verb named
        // in a commit message is not a verb that shipped.
        // ⚠ `audio` left the cleanest signature of the five: `pub mod
        // audio_cli;` still declared, the module still compiled, and not one
        // reference to it from any dispatcher.
        "audio" => crate::audio_cli::run_audio_command(&args),
        "theme" => {
            let theme = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .unwrap_or("light");
            let theme = match theme {
                "light" => UiTheme::ZedLight,
                "dark" => UiTheme::ZedDark,
                other => anyhow::bail!("unsupported app theme: {other}"),
            };
            run_app_control_set_ui_theme(theme, timeout_ms)
        }
        "launch-flags" => {
            let positional = cli_positional_args(&args, 3);
            let action = positional.first().copied().unwrap_or("open");
            let slug = cli_flag_value(&args, "--cli").map(str::to_string);
            let flags = cli_flag_value(&args, "--args").map(str::to_string);
            match action {
                "open" | "show" | "on" | "true" | "1" => {
                    run_app_control_set_launch_flags(Some(true), slug, flags, timeout_ms)
                }
                "close" | "hide" | "off" | "false" | "0" => {
                    run_app_control_set_launch_flags(Some(false), slug, flags, timeout_ms)
                }
                "set" => {
                    let slug = slug.context(
                        "server app launch-flags set needs --cli <slug> (and --args to \
                         store; omit --args to reset that CLI to its default)",
                    )?;
                    run_app_control_set_launch_flags(None, Some(slug), flags, timeout_ms)
                }
                "reset" => {
                    let slug = slug
                        .context("server app launch-flags reset needs --cli <slug>")?;
                    run_app_control_set_launch_flags(None, Some(slug), None, timeout_ms)
                }
                other => anyhow::bail!("unsupported app launch-flags action: {other}"),
            }
        }
        "theme-editor" => {
            let action = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .unwrap_or("open");
            match action {
                "open" | "show" | "on" | "true" | "1" => {
                    run_app_control_set_theme_editor_open(true, timeout_ms)
                }
                "close" | "hide" | "off" | "false" | "0" => {
                    run_app_control_set_theme_editor_open(false, timeout_ms)
                }
                "reset" | "defaults" => run_app_control_reset_theme_editor(timeout_ms),
                "set" | "values" => {
                    let brightness = cli_flag_value(&args, "--brightness")
                        .map(str::parse::<f32>)
                        .transpose()
                        .context("invalid --brightness for server app theme-editor set")?;
                    let alpha = cli_flag_value(&args, "--alpha")
                        .map(str::parse::<f32>)
                        .transpose()
                        .context("invalid --alpha for server app theme-editor set")?;
                    let grain = cli_flag_value(&args, "--grain")
                        .map(str::parse::<f32>)
                        .transpose()
                        .context("invalid --grain for server app theme-editor set")?;
                    run_app_control_set_theme_editor_values(
                        brightness, alpha, grain, timeout_ms,
                    )
                }
                other => anyhow::bail!("unsupported app theme-editor action: {other}"),
            }
        }
        "fullscreen" => {
            let action = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .unwrap_or("toggle");
            let current_state = crate::request_app_control(
                home_dir,
                crate::AppControlCommand::DescribeState,
                timeout_ms,
            )?;
            let currently_fullscreen = current_state
                .data
                .as_ref()
                .and_then(|data| data.get("shell"))
                .and_then(|shell| shell.get("fullscreen"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let enabled = match action {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                "toggle" => !currently_fullscreen,
                other => anyhow::bail!("unsupported fullscreen action: {other}"),
            };
            run_app_control_set_fullscreen(enabled, timeout_ms)
        }
        "maximize" | "maximized" => {
            let action = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .unwrap_or("toggle");
            let current_state = crate::request_app_control(
                home_dir,
                crate::AppControlCommand::DescribeState,
                timeout_ms,
            )?;
            let currently_maximized = current_state
                .data
                .as_ref()
                .and_then(|data| data.get("window"))
                .and_then(|window| window.get("maximized"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let enabled = match action {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                "toggle" => !currently_maximized,
                other => anyhow::bail!("unsupported maximize action: {other}"),
            };
            run_app_control_set_maximized(enabled, timeout_ms)
        }
        "force-foreground" | "force-fg" => {
            let action = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .unwrap_or("on");
            let enabled = match action {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                other => anyhow::bail!("unsupported force-foreground action: {other}"),
            };
            run_app_control_set_force_foreground(enabled, timeout_ms)
        }
        "split" => {
            // server app split create [--axis side-by-side|stacked] <path> <path> [...]
            // server app split web-tab [--axis ...] <session_path> <tab_id>
            // server app split ungroup <group_id>
            // server app split ratio <group_id> <0.0..1.0>
            // server app split focus <session_path>
            let action = args.get(3).map(String::as_str).ok_or_else(|| {
                anyhow::anyhow!(
                    "missing action for server app split (create|web-tab|ungroup|ratio|focus)"
                )
            })?;
            match action {
                "create" => {
                    let axis = args
                        .windows(2)
                        .find_map(|window| (window[0] == "--axis").then(|| window[1].clone()));
                    let members: Vec<String> = cli_positional_args(&args, 4)
                        .into_iter()
                        .filter(|arg| !arg.starts_with("--"))
                        .map(|arg| arg.to_string())
                        .collect();
                    if members.len() < 2 {
                        anyhow::bail!("server app split create needs at least 2 session paths");
                    }
                    run_app_control_create_split_group(members, axis, timeout_ms)
                }
                "web-tab" => {
                    let axis = args
                        .windows(2)
                        .find_map(|window| (window[0] == "--axis").then(|| window[1].clone()));
                    let mut positionals = cli_positional_args(&args, 4)
                        .into_iter()
                        .filter(|arg| !arg.starts_with("--"));
                    let session_path = positionals.next().ok_or_else(|| {
                        anyhow::anyhow!("missing session path for server app split web-tab")
                    })?;
                    let tab: u64 = positionals
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing tab id for server app split web-tab")
                        })?
                        .parse()
                        .map_err(|_| anyhow::anyhow!("tab id must be a number"))?;
                    run_app_control_split_web_tab(&session_path, tab, axis, timeout_ms)
                }
                "ungroup" | "dissolve" => {
                    let group_id = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing group id for server app split ungroup")
                        })?;
                    run_app_control_ungroup_split_group(&group_id, timeout_ms)
                }
                "ratio" => {
                    let mut positionals = cli_positional_args(&args, 4).into_iter();
                    let group_id = positionals.next().ok_or_else(|| {
                        anyhow::anyhow!("missing group id for server app split ratio")
                    })?;
                    let ratio: f32 = positionals
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing ratio for server app split ratio")
                        })?
                        .parse()
                        .map_err(|_| anyhow::anyhow!("ratio must be a number in 0.0..1.0"))?;
                    run_app_control_set_split_group_ratio(&group_id, ratio, timeout_ms)
                }
                "focus" => {
                    // server app split focus <session_path> [pane_index]
                    let mut positionals = cli_positional_args(&args, 4).into_iter();
                    let session_path = positionals.next().ok_or_else(|| {
                        anyhow::anyhow!("missing session path for server app split focus")
                    })?;
                    let pane: Option<usize> = match positionals.next() {
                        Some(raw) => Some(
                            raw.parse()
                                .map_err(|_| anyhow::anyhow!("pane index must be a number"))?,
                        ),
                        None => None,
                    };
                    run_app_control_focus_split_pane(&session_path, pane, timeout_ms)
                }
                other => anyhow::bail!(
                    "unknown server app split action {other:?} (create|web-tab|ungroup|ratio|focus)"
                ),
            }
        }
        "open" => {
            let session_path = cli_positional_args(&args, 3)
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing session path for server app open"))?;
            let view_mode = args.windows(2).find_map(|window| {
                if window[0] != "--view" {
                    return None;
                }
                match window[1].as_str() {
                    "preview" | "rendered" | "web-view" | "webview" => {
                        Some(AppControlViewMode::Preview)
                    }
                    "terminal" => Some(AppControlViewMode::Terminal),
                    _ => None,
                }
            });
            run_app_control_open_path(session_path, view_mode, timeout_ms)
        }
        "drag" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app drag"))?;
            let row_path = cli_positional_args(&args, 4).into_iter().next();
            let placement = args.windows(2).find_map(|window| {
                if window[0] == "--placement" {
                    Some(window[1].as_str())
                } else {
                    None
                }
            });
            run_app_control_drag(action, row_path, placement, timeout_ms)
        }
        "pointer" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app pointer"))?;
            let x = args.windows(2).find_map(|window| {
                if window[0] == "--x" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let y = args.windows(2).find_map(|window| {
                if window[0] == "--y" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let start_x = args.windows(2).find_map(|window| {
                if window[0] == "--start-x" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let start_y = args.windows(2).find_map(|window| {
                if window[0] == "--start-y" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let end_x = args.windows(2).find_map(|window| {
                if window[0] == "--end-x" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let end_y = args.windows(2).find_map(|window| {
                if window[0] == "--end-y" {
                    window[1].parse::<f64>().ok()
                } else {
                    None
                }
            });
            let button = args.windows(2).find_map(|window| {
                if window[0] == "--button" {
                    Some(window[1].as_str())
                } else {
                    None
                }
            });
            let count = args.windows(2).find_map(|window| {
                if window[0] == "--count" {
                    window[1].parse::<u8>().ok()
                } else {
                    None
                }
            });
            let steps = args.windows(2).find_map(|window| {
                if window[0] == "--steps" {
                    window[1].parse::<u16>().ok()
                } else {
                    None
                }
            });
            let step_delay_ms = args.windows(2).find_map(|window| {
                if window[0] == "--step-delay-ms" {
                    window[1].parse::<u64>().ok()
                } else {
                    None
                }
            });
            run_app_control_pointer(
                action,
                x,
                y,
                start_x,
                start_y,
                end_x,
                end_y,
                button,
                count,
                steps,
                step_delay_ms,
                timeout_ms,
            )
        }
        "grid" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .context("missing action for server app grid")?;
            let cell = cli_positional_args(&args, 4).into_iter().next();
            let cols = cli_flag_value(&args, "--cols").and_then(|v| v.parse::<u32>().ok());
            let rows = cli_flag_value(&args, "--rows").and_then(|v| v.parse::<u32>().ok());
            let region = cli_flag_value(&args, "--region");
            let target = cli_flag_value(&args, "--target");
            let ttl_secs =
                cli_flag_value(&args, "--ttl-secs").and_then(|v| v.parse::<u64>().ok());
            let button = cli_flag_value(&args, "--button");
            let count = cli_flag_value(&args, "--count").and_then(|v| v.parse::<u8>().ok());
            let refine = args.iter().any(|arg| arg == "--refine");
            let keep = args.iter().any(|arg| arg == "--keep");
            run_app_control_grid(
                action, cell, cols, rows, region, target, ttl_secs, button, count, refine,
                keep, timeout_ms,
            )
        }
        "dom-eval" => {
            // ⛔ NOT `args.get(3)`. `--client`/`--pid` are scanned out of the
            // WHOLE argv, so a fixed-index read disagrees with the very
            // flags it was typed beside: `dom-eval --client shadow '<js>'`
            // used to evaluate the STRING "--client" and report success.
            // One owner for the rule, in yggterm_core, because this binary
            // and `yggterm` both have this arm.
            let script = yggterm_core::cli_payload_arg(
                &args,
                3,
                "script for server app dom-eval",
            )
            .map_err(|error| anyhow::anyhow!(error))?;
            run_app_control_dom_eval(script, timeout_ms)
        }
        "start-action" | "start" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app start-action"))?;
            run_app_control_start_action(action, timeout_ms)
        }
        "start-page" | "show-start-page" | "home" => {
            run_app_control_show_start_page(timeout_ms)
        }
        "tree" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app tree"))?;
            match action {
                "select" | "selection" => {
                    let paths = cli_positional_args(&args, 4)
                        .into_iter()
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>();
                    let anchor_path = cli_flag_value(&args, "--anchor").map(ToOwned::to_owned);
                    run_app_control_set_tree_selection(paths, anchor_path, timeout_ms)
                }
                other => anyhow::bail!("unsupported app tree action: {other}"),
            }
        }
        "key" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app key"))?;
            let positional = cli_positional_args(&args, 4);
            let positional_owned = positional
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>();
            let text = args.windows(2).find_map(|window| {
                if window[0] == "--text" {
                    Some(window[1].as_str())
                } else {
                    None
                }
            });
            let keys = if action == "press" {
                positional_owned.clone()
            } else {
                Vec::new()
            };
            run_app_control_key(
                action,
                &keys,
                text.or_else(|| positional.first().copied()),
                timeout_ms,
            )
        }
        "terminal" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app terminal"))?;
            match action {
                "new" => {
                    let machine_key = args.windows(2).find_map(|window| {
                        if window[0] == "--machine-key" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let cwd = args.windows(2).find_map(|window| {
                        if window[0] == "--cwd" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let title_hint = args.windows(2).find_map(|window| {
                        if window[0] == "--title" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let purpose = args.windows(2).find_map(|window| {
                        if window[0] == "--purpose" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let kind = args.windows(2).find_map(|window| {
                        if window[0] == "--kind" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let activate = !args.iter().any(|arg| arg == "--no-activate");
                    // Per-launch model / permission mode + the initial
                    // prompt, all through the SHARED readers — a flag must
                    // mean the same thing typed at either binary.
                    let launch = yggterm_core::agent_launch_options_from_args(&args)
                        .map_err(|message| anyhow::anyhow!(message))?;
                    let prompt = crate::read_prompt(&args)?;
                    // WHERE the row lands, applied INSIDE the create. Same
                    // shared reader at both binaries.
                    let seat = crate::read_row_seat(&args);
                    run_app_control_create_terminal_with_tenancy(
                        machine_key,
                        cwd,
                        title_hint,
                        purpose,
                        kind,
                        activate,
                        // Provenance + opt-in ephemerality, parsed by the
                        // ONE shared reader both binaries call.
                        Some(crate::session_tenancy::agent_cli_create_terminal_tenancy(
                            &args,
                        )?),
                        &launch,
                        &seat,
                        prompt.as_deref(),
                        timeout_ms,
                    )
                }
                "adopt" => {
                    let pid = args
                        .windows(2)
                        .find_map(|window| {
                            if window[0] == "--target-pid"
                                || window[0] == "--outer-pid"
                                || window[0] == "--pid"
                            {
                                Some(window[1].as_str())
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| anyhow::anyhow!("missing --target-pid for server app terminal adopt (use --target-pid <pid> or legacy --pid <pid>)"))?;
                    let pid_num: u32 = pid
                        .parse()
                        .map_err(|_| anyhow::anyhow!("--pid must be an integer"))?;
                    // Validate pid exists and is a PTY leader; report adopt_refused early instead of creating a blank shell.
                    #[cfg(target_os = "linux")]
                    {
                        let stat = std::fs::read_to_string(format!("/proc/{pid_num}/stat"))
                            .map_err(|_| anyhow::anyhow!("adopt_refused: pid {} not found or unreadable (/proc/{}/stat)", pid_num, pid_num))?;
                        if !stat.contains(&format!(" {} ", pid_num)) && !stat.is_empty() {
                            // stat read succeeded, pid exists
                        }
                        // Check reptyr availability at create time so the verb refuses with a named reason instead of leaving a blank row.
                        if std::process::Command::new("which")
                            .arg("reptyr")
                            .output()
                            .map(|o| !o.status.success())
                            .unwrap_or(true)
                        {
                            anyhow::bail!("adopt_refused: reptyr not found in PATH (apt install reptyr)");
                        }
                    }
                    let title_hint = args.windows(2).find_map(|window| {
                        if window[0] == "--title" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let machine_key = args.windows(2).find_map(|window| {
                        if window[0] == "--machine-key" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let cwd = args.windows(2).find_map(|window| {
                        if window[0] == "--cwd" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    });
                    let activate = !args.iter().any(|arg| arg == "--no-activate");
                    let seat = crate::read_row_seat(&args);
                    // Create the host shell that will run reptyr -T <pid>. Title defaults to "adopted-pty-<pid>".
                    let adopt_title = title_hint
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| format!("adopted-pty-{}", pid_num));
                    let tenancy = crate::session_tenancy::agent_cli_create_terminal_tenancy(&args).ok();
                    let launch = yggterm_core::agent_launch_options_from_args(&args)
                        .map_err(|m| anyhow::anyhow!(m))
                        .unwrap_or_default();
                    // Deliver `reptyr -T <pid>` as the initial prompt so the brand-new shell steals the outer PTY
                    // in one verb (no manual second write needed). `run_app_control_create_terminal_with_tenancy`
                    // waits for the shell to reach an idle prompt before injecting, which is the correct sync point
                    // for reptyr hijack.
                    let reptyr_prompt = format!("reptyr -T {}", pid_num);
                    crate::run_app_control_create_terminal_with_tenancy(
                        machine_key,
                        cwd,
                        Some(&adopt_title),
                        Some(&format!("adopt outer PTY {} via reptyr -T", pid_num)),
                        Some("shell"),
                        activate,
                        tenancy,
                        &launch,
                        &seat,
                        Some(&reptyr_prompt),
                        timeout_ms,
                    )
                }
                "send" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing session path for server app terminal send")
                        })?;
                    let data = if args.iter().any(|arg| arg == "--stdin") {
                        let mut value = String::new();
                        std::io::stdin()
                            .read_to_string(&mut value)
                            .context("reading app terminal send stdin")?;
                        value
                    } else {
                        args.windows(2)
                            .find_map(|window| {
                                if window[0] == "--data" {
                                    Some(window[1].as_str())
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing --data or --stdin for server app terminal send"
                                )
                            })?
                            .to_string()
                    };
                    let allow_multiline =
                        args.iter().any(|arg| arg == "--allow-multiline");
                    run_app_control_send_terminal_input(
                        session_path,
                        &data,
                        allow_multiline,
                        timeout_ms,
                    )
                }
                "submit" => {
                    // Readiness-gated prompt insertion: waits for the session to
                    // reach an idle interactive prompt, then sends; refuses if it
                    // never becomes ready. `--ready-timeout-ms` bounds the wait.
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal submit"
                            )
                        })?;
                    let data = if args.iter().any(|arg| arg == "--stdin") {
                        let mut value = String::new();
                        std::io::stdin()
                            .read_to_string(&mut value)
                            .context("reading app terminal submit stdin")?;
                        value
                    } else {
                        args.windows(2)
                            .find_map(|window| {
                                if window[0] == "--data" {
                                    Some(window[1].as_str())
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing --data or --stdin for server app terminal submit"
                                )
                            })?
                            .to_string()
                    };
                    let ready_timeout_ms = args
                        .windows(2)
                        .find_map(|window| {
                            if window[0] == "--ready-timeout-ms" {
                                window[1].parse::<u64>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(30_000);
                    run_app_control_submit_terminal_prompt(
                        session_path,
                        &data,
                        ready_timeout_ms,
                        timeout_ms,
                    )
                }
                "input-check" => {
                    // The wedge question, asked without submitting anything:
                    // is this row consuming input? A wedged agent row is
                    // alive, idle-looking and deaf, and `send` into it
                    // reports success while delivering nothing.
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal input-check"
                            )
                        })?;
                    let check_timeout_ms = args
                        .windows(2)
                        .find_map(|window| {
                            if window[0] == "--check-timeout-ms" {
                                window[1].parse::<u64>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(6_000);
                    run_app_control_check_terminal_input(
                        session_path,
                        check_timeout_ms,
                        timeout_ms,
                    )
                }
                "focus" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal focus"
                            )
                        })?;
                    run_app_control_reclaim_terminal_focus(session_path, timeout_ms)
                }
                "redraw" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal redraw"
                            )
                        })?;
                    run_app_control_redraw_terminal(session_path, timeout_ms)
                }
                "reconcile" | "reconcile-from-daemon" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal reconcile"
                            )
                        })?;
                    run_app_control_reconcile_terminal_from_daemon(session_path, timeout_ms)
                }
                "paste" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal paste"
                            )
                        })?;
                    run_app_control_paste_terminal_clipboard(session_path, timeout_ms)
                }
                "paste-image" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal paste-image"
                            )
                        })?;
                    run_app_control_paste_terminal_clipboard_image(session_path, timeout_ms)
                }
                "keep" | "keep-alive" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing session path for server app terminal keep")
                        })?;
                    run_app_control_set_session_keep_alive(session_path, true, timeout_ms)
                }
                "unkeep" | "stop-keep-alive" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal unkeep"
                            )
                        })?;
                    run_app_control_set_session_keep_alive(session_path, false, timeout_ms)
                }
                "probe-type" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal probe-type"
                            )
                        })?;
                    let data = args
                        .windows(2)
                        .find_map(|window| {
                            if window[0] == "--data" {
                                Some(window[1].as_str())
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing --data for server app terminal probe-type")
                        })?;
                    let press_enter = args.iter().any(|arg| arg == "--enter");
                    let press_tab = args.iter().any(|arg| arg == "--tab");
                    let press_ctrl_c = args.iter().any(|arg| arg == "--ctrl-c");
                    let press_ctrl_e = args.iter().any(|arg| arg == "--ctrl-e");
                    let press_ctrl_u = args.iter().any(|arg| arg == "--ctrl-u");
                    let per_char = args.iter().any(|arg| arg == "--per-char");
                    let mode = args
                        .windows(2)
                        .find_map(|window| {
                            if window[0] != "--mode" {
                                return None;
                            }
                            match window[1].as_str() {
                                "auto" => Some(ProbeTerminalViewportInputMode::Auto),
                                "keyboard" => Some(ProbeTerminalViewportInputMode::Keyboard),
                                "xterm" => Some(ProbeTerminalViewportInputMode::Xterm),
                                _ => None,
                            }
                        })
                        .unwrap_or(ProbeTerminalViewportInputMode::Auto);
                    run_app_control_probe_terminal_viewport_input(
                        session_path,
                        data,
                        mode,
                        per_char,
                        press_enter,
                        press_tab,
                        press_ctrl_c,
                        press_ctrl_e,
                        press_ctrl_u,
                        timeout_ms,
                    )
                }
                "probe-scroll" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal probe-scroll"
                            )
                        })?;
                    let lines = args
                        .windows(2)
                        .find_map(|window| {
                            if window[0] == "--lines" {
                                window[1].parse::<i32>().ok()
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing --lines for server app terminal probe-scroll"
                            )
                        })?;
                    run_app_control_probe_terminal_viewport_scroll(
                        session_path,
                        lines,
                        timeout_ms,
                    )
                }
                // ⛔ READ-BUFFER AND SCROLL ARE THE BOOTER'S EYES AND HANDS.
                // Both were lost in the dispatcher consolidation and neither
                // failed at build or deploy time — the watchdog's
                // plan-limit guard reads a row's screen with `read-buffer`,
                // got "unsupported app terminal action", and correctly took
                // that as *could not look*, so it refused to wake anything on
                // the fleet. Restored together because the manual recovery
                // path used both.
                "read-buffer" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal read-buffer"
                            )
                        })?;
                    let mode = cli_flag_value(&args, "--mode").unwrap_or("screen");
                    run_app_control_read_terminal_buffer(session_path, mode, timeout_ms)
                }
                "scroll" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing session path for server app terminal scroll")
                        })?;
                    let to = cli_flag_value(&args, "--to").ok_or_else(|| {
                        anyhow::anyhow!(
                            "missing --to (top|bottom|±N lines) for server app terminal scroll"
                        )
                    })?;
                    run_app_control_scroll_terminal_viewport(session_path, to, timeout_ms)
                }
                "probe-select" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal probe-select"
                            )
                        })?;
                    run_app_control_probe_terminal_viewport_select(session_path, timeout_ms)
                }
                "probe-primary-paste" | "probe-primary-selection-paste" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal probe-primary-paste"
                            )
                        })?;
                    let data = args
                        .windows(2)
                        .find_map(|window| {
                            if window[0] == "--data" {
                                Some(window[1].as_str())
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing --data for server app terminal probe-primary-paste"
                            )
                        })?;
                    run_app_control_probe_terminal_primary_selection_paste(
                        session_path,
                        data,
                        timeout_ms,
                    )
                }
                "probe-context-menu" | "probe-right-click-menu" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app terminal probe-context-menu"
                            )
                        })?;
                    run_app_control_probe_terminal_context_menu(session_path, timeout_ms)
                }
                other => anyhow::bail!("unsupported app terminal action: {other}"),
            }
        }
        "chrome" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app chrome"))?;
            match action {
                "type" => {
                    let selector = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing selector for server app chrome type — name the \
                                 field by its stamp, e.g. '[data-yggterm-start-page-search]'"
                            )
                        })?;
                    let flag_value = |name: &str| {
                        args.windows(2).find_map(|window| {
                            (window[0] == name).then(|| window[1].clone())
                        })
                    };
                    let data = flag_value("--data").ok_or_else(|| {
                        anyhow::anyhow!("missing --data for server app chrome type")
                    })?;
                    let clear = args.iter().any(|arg| arg == "--clear");
                    let press_enter = args.iter().any(|arg| arg == "--enter");
                    // `--assert <selector>@<attribute>`: the pair that turns
                    // "the keystroke landed in the box" into "the keystroke
                    // changed what the surface renders".
                    let assert = flag_value("--assert");
                    let (assert_selector, assert_attribute) = match assert.as_deref() {
                        Some(spec) => match spec.rsplit_once('@') {
                            Some((selector, attribute)) => (
                                Some(selector.to_string()),
                                Some(attribute.to_string()),
                            ),
                            None => anyhow::bail!(
                                "--assert wants <selector>@<attribute>, got {spec:?}"
                            ),
                        },
                        None => (None, None),
                    };
                    run_app_control_probe_chrome_input(
                        &selector,
                        &data,
                        clear,
                        press_enter,
                        assert_selector.as_deref(),
                        assert_attribute.as_deref(),
                        timeout_ms,
                    )
                }
                other => anyhow::bail!("unsupported app chrome action: {other}"),
            }
        }
        "session" => {
            let action = args
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing action for server app session"))?;
            match action {
                "remove" | "delete" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app session remove"
                            )
                        })?;
                    run_app_control_remove_session(session_path, timeout_ms)
                }
                "rename" => {
                    let positionals = cli_positional_args(&args, 4);
                    let session_path = positionals.first().copied().ok_or_else(|| {
                        anyhow::anyhow!("missing session path for server app session rename")
                    })?;
                    let title = positionals.get(1).copied().ok_or_else(|| {
                        anyhow::anyhow!("missing title for server app session rename")
                    })?;
                    run_app_control_rename_session(session_path, title, timeout_ms)
                }
                "restart" => {
                    let session_path = cli_positional_args(&args, 4)
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "missing session path for server app session restart"
                            )
                        })?;
                    run_app_control_restart_session(session_path, timeout_ms)
                }
                // `server app session outline <path> <prefix>` — number a
                // row that already exists. The prefix is stored SEPARATELY
                // from the title and composed at render time, so a CLI
                // re-title can no longer destroy a position. An empty
                // prefix clears it.
                "outline" => {
                    let positionals = cli_positional_args(&args, 4);
                    let session_path = positionals.first().copied().ok_or_else(|| {
                        anyhow::anyhow!(
                            "usage: server app session outline <session-path> <prefix>  \
                             (an empty prefix clears it)"
                        )
                    })?;
                    let prefix = positionals.get(1).copied().unwrap_or("");
                    crate::run_app_control_set_session_outline(
                        session_path,
                        prefix,
                        timeout_ms,
                    )
                }
                other => anyhow::bail!("unsupported app session action: {other}"),
            }
        }
        "keytips" => {
            // The twin of main.rs's arm, verb-for-verb: the headless binary
            // is the one agents actually reach for, and a verb that exists
            // on only one binary is the split-dispatch trap the app-control
            // target work just closed (both binaries, one owner).
            let action = args.get(3).map(String::as_str).unwrap_or("audit");
            match action {
                "audit" => {
                    let json = args.iter().any(|arg| arg == "--json");
                    crate::run_app_control_keytips_audit(json, timeout_ms)
                }
                "show" => crate::run_app_control_keytips_overlay(true, timeout_ms),
                "hide" => crate::run_app_control_keytips_overlay(false, timeout_ms),
                other => anyhow::bail!("unsupported app keytips action: {other}"),
            }
        }
        // CAMERA / MICROPHONE. The twin of main.rs's arm — both binaries or
        // neither, per the split-dispatch trap the web plane already paid
        // for. Read the prompt from `server app state`
        // (`pending_media_capture`), answer it here.
        "media" => {
            let action = args.get(3).map(String::as_str).unwrap_or("answer");
            match action {
                "answer" => {
                    // ⛔ NOT `args.get(4)`. The two copies of this arm had
                    // DRIFTED: the GUI binary read the payload through the one
                    // guarded reader, which scans past flags and their values,
                    // while this one took position 4 and merely REFUSED
                    // anything flag-shaped. So the documented form
                    // `media answer --request 5 allow` worked from one binary
                    // and failed "missing answer" from the other — the one
                    // every agent skill says to drive. Found by collapsing the
                    // dispatchers, which is the argument for collapsing them.
                    let answer = app_control_payload_arg(
                        &args,
                        4,
                        "answer for server app media answer \
                         (allow | deny-once | block-site)",
                    )?
                    .to_string();
                    let request_id = cli_flag_value(&args, "--request")
                        .map(|value| value.parse::<u64>())
                        .transpose()
                        .map_err(|_| {
                            anyhow::anyhow!("--request takes the numeric request_id")
                        })?;
                    crate::run_app_control_media_answer(
                        answer, request_id, timeout_ms,
                    )
                }
                other => anyhow::bail!("unsupported app media action: {other}"),
            }
        }
        // THE web verb plane, on the binary agents actually drive. It used
        // to exist on the GUI binary only, so every verb in it —
        // eval/read/await/do/fill/wait/ensure/frames/… — answered
        // "unsupported app control command: web" here, which reads to an
        // agent as "not built". One owner
        // (crates/yggterm-server/src/app_control_web_cli.rs), both
        // binaries; do not inline a verb here.
        "web" => crate::run_app_control_web_cli(&args, timeout_ms),
        other => anyhow::bail!("unsupported app control command: {other}"),
    }
}

/// What `server app launch` should do when a GUI is already live.
///
/// ⛔ **A launch used to ADD a GUI unconditionally**, with no existing-instance
/// check and no retirement of the incumbent. The old process kept its window,
/// kept painting and kept its rows, so the user's own remedy compounded the
/// fault: restarting to escape a broken window added a second one, and the
/// window they were looking at was still the OLD one. Measured cost of one such
/// orphan: 3.63 core-hours over 12.4 hours, 63% of all GUI CPU that day — at a
/// rate (29.2% of a core) that was entirely NORMAL. **The waste was duration,
/// not a runaway**, which is why this is a lifecycle guard and not an
/// optimisation.
///
/// ⚠ Shadows are not duplicates. An agent's read-only shadow client exists
/// precisely so probes stay off the user's seat, and several may be live at
/// once; only an **Active** client owns the window this guard protects.
#[derive(Debug, PartialEq, Eq)]
pub enum AppLaunchDuplicateDecision {
    /// No live Active GUI, or the caller accepted the consequence explicitly.
    Launch,
    /// An Active GUI already owns the display; naming it is the whole point,
    /// because "a GUI is already running" without a pid is unactionable.
    Refuse { incumbent_pids: Vec<u32> },
}

/// `--replace` and `--allow-duplicate` are the two ways to say "I meant it".
///
/// They differ in intent and the difference is deliberate: `--allow-duplicate`
/// is the sandbox's flag (a throwaway GUI with its own env is a legitimate
/// second instance), while `--replace` retires the incumbent first. Neither is
/// the default, because the default is what a confused user reaches for.
///
/// ⛔ **`--replace` must actually replace.** A flag that only suppresses the
/// refusal while leaving both GUIs alive would be a verb that lies — the exact
/// class this project keeps cataloguing — and it would recreate the bug the
/// guard exists to stop, behind a name that reads as the fix.
fn app_launch_duplicate_override(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--replace" | "--allow-duplicate"))
}

fn app_launch_replace_requested(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--replace")
}

/// Retire the incumbent GUI before the replacement starts.
///
/// ⚠ **This leaves a few seconds with no window**, and that is deliberate
/// rather than hidden: it is exactly the gap the existing manual restart ritual
/// has (kill, then launch), and the daemon owns every PTY, so no session, no
/// scrollback and no agent's work is lost with the GUI. Retiring AFTER the new
/// one paints would be better, but this call path spawns the companion and
/// waits on it, so there is no moment between "started" and "painted" to act
/// in. Naming the limitation beats shipping a flag that quietly does neither.
fn app_launch_retire_incumbents(pids: &[u32]) {
    for pid in pids {
        // SIGTERM, never SIGKILL: the GUI's own shutdown path deregisters it
        // from the client registry, and a killed process leaves the record for
        // the liveness sweep to reap instead.
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        eprintln!("server app launch: retired incumbent GUI pid {pid} (--replace)");
    }
}

/// The pids that own a window. `client_role` of `None` reads as **Active** —
/// that is the legacy record shape, and treating it as a shadow would silently
/// disarm the guard on exactly the records that predate the role field.
fn active_gui_pids(records: &[crate::ClientInstanceRecord]) -> Vec<u32> {
    records
        .iter()
        .filter(|record| {
            record
                .client_role
                .as_deref()
                .map(|role| role.eq_ignore_ascii_case("active"))
                .unwrap_or(true)
        })
        .map(|record| record.pid)
        .collect()
}

/// Pure half, so the decision is testable without a daemon, a display or a
/// filesystem.
pub fn app_launch_duplicate_decision(
    records: &[crate::ClientInstanceRecord],
    args: &[String],
) -> AppLaunchDuplicateDecision {
    if app_launch_duplicate_override(args) {
        return AppLaunchDuplicateDecision::Launch;
    }
    let incumbent_pids = active_gui_pids(records);
    if incumbent_pids.is_empty() {
        AppLaunchDuplicateDecision::Launch
    } else {
        AppLaunchDuplicateDecision::Refuse { incumbent_pids }
    }
}

/// Effectful half: ask who is live, then apply the decision.
///
/// ⛔ **An unreadable answer must never read as "nobody is running".** That is
/// the same seam `active_client_instance_records_from_dir` documents, from the
/// other side: there, mistaking unreadable for empty retires a daemon that
/// still has clients; here it would launch a second GUI over a live one. So a
/// query error is NOT swallowed into a permissive default — but it is also not
/// fatal, because refusing to launch on a host that cannot answer would leave a
/// user with no window at all. It warns and proceeds, which is the direction
/// that cannot strand them.
fn app_launch_duplicate_guard_check(
    args: &[String],
    home_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let endpoint = crate::default_endpoint(home_dir);
    let records = match crate::active_client_instance_records(home_dir, &endpoint) {
        Ok(records) => records,
        Err(error) => {
            eprintln!(
                "server app launch: could not read the client registry ({error}); \
                 launching anyway rather than leaving you with no window. \
                 If a GUI was already running, there are now two."
            );
            return Ok(());
        }
    };
    // `--replace` still has to find out WHO it is replacing, so the incumbent
    // scan happens before the decision short-circuits on the override.
    if app_launch_replace_requested(args) {
        let incumbents = active_gui_pids(&records);
        app_launch_retire_incumbents(&incumbents);
        return Ok(());
    }
    match app_launch_duplicate_decision(&records, args) {
        AppLaunchDuplicateDecision::Launch => Ok(()),
        AppLaunchDuplicateDecision::Refuse { incumbent_pids } => {
            let pids = incumbent_pids
                .iter()
                .map(|pid| pid.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "a yggterm GUI is already running (pid {pids}) and launching would ADD a second \
                 one rather than replace it — the window you are looking at would still be the \
                 old one. Re-run with --replace to retire the incumbent, or --allow-duplicate if \
                 you genuinely want two (a sandbox GUI does)."
            )
        }
    }
}

#[cfg(test)]
mod dispatch_reachability_lock {
    /// Handlers that exist on purpose and are dispatched by nobody. Each entry
    /// is a promise that its absence from the CLI is intended; the list should
    /// only ever shrink.
    const UNDISPATCHED_ON_PURPOSE: &[(&str, &str)] = &[(
        "run_app_control_create_terminal",
        "a thin legacy wrapper that only forwards to \
         run_app_control_create_terminal_with_tenancy, which is what the CLI calls",
    )];

    /// ⛔ EVERY APP-CONTROL HANDLER MUST BE REACHABLE FROM A DISPATCHER.
    ///
    /// The existing lock (`neither_binary_dispatches_server_app_itself`) bans a
    /// SECOND dispatcher appearing. It cannot see the failure that actually
    /// happened: collapsing the two dispatchers into one **silently dropped
    /// four verbs on the way**, and nothing anywhere failed.
    ///
    /// `terminal read-buffer`, `terminal scroll`, `web-view layout` and `theme`
    /// all stopped existing while their handlers stayed compiled, still `pub`,
    /// and still IMPORTED by the GUI binary. The protocol kept carrying
    /// `SetPreviewLayout` and `SetUiTheme` that nothing could ever send.
    /// ⚠ `theme` is the sharpest instance: the consolidation commit named it in
    /// its own message as a verb that answered from only one binary, and
    /// shipped it answering from neither, with the comment above still
    /// describing the fix.
    ///
    /// ⭐ The cost was not cosmetic. The fleet watchdog reads a parked row's
    /// screen with `terminal read-buffer` to check whether it is sitting on a
    /// billing dialog before typing into it. The verb answered "unsupported",
    /// the guard read that as *could not look* — correctly, and fail-safe — and
    /// refused to wake anything at all. **A lost verb took out the whole wake
    /// plane, and the direction it failed in is the only reason it was merely
    /// a stall.**
    ///
    /// ⇒ A handler with no caller is a verb that has stopped existing. This
    /// lock is cheap, it is structural, and it does not care how the verb was
    /// lost — refactor, bad merge, or a deletion nobody re-read.
    #[test]
    fn every_app_control_handler_is_dispatched() {
        const LIB: &str = include_str!("lib.rs");
        // The general surface, the `web` plane that was collapsed onto its own
        // file first, AND BOTH BINARIES.
        //
        // ⚠ The binaries were missing here for the same reason they were missing
        // from the twin below, and leaving them out is a LATENT RED rather than a
        // present one: no handler is dispatched only from a binary *today*, so
        // this list looked complete. The twin proved it is not — it fired on
        // `main` the moment a module both binaries dispatch met the lock — and a
        // lock whose blindness is waiting for the next commit is worse than one
        // that fails now, because it will fail in someone else's lane.
        //
        // ⇒ Both halves of this lock now name the same four sources. A scanner
        // that cannot see one of the dispatchers is the exact defect this file
        // exists to prevent, and it had it.
        let dispatchers = [
            include_str!("app_control_cli.rs"),
            include_str!("app_control_web_cli.rs"),
            include_str!("../../../apps/yggterm/src/main.rs"),
            include_str!("../../../apps/yggterm/src/bin/yggterm-headless.rs"),
        ];
        let mut handlers: Vec<&str> = Vec::new();
        for (index, _) in LIB.match_indices("pub fn run_app_control_") {
            let rest = &LIB[index + "pub fn ".len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            handlers.push(&rest[..end]);
        }
        handlers.sort_unstable();
        handlers.dedup();
        assert!(
            handlers.len() > 50,
            "the handler scan found only {} — it has gone blind and this lock \
             would pass on an empty CLI",
            handlers.len()
        );
        let orphans: Vec<&str> = handlers
            .into_iter()
            .filter(|name| {
                !UNDISPATCHED_ON_PURPOSE
                    .iter()
                    .any(|(allowed, _)| allowed == name)
            })
            .filter(|name| {
                // A CALL, not a `use` line: the four lost verbs were imported
                // by the GUI binary the whole time they were unreachable, so
                // "the name appears" is exactly the check that would have
                // passed while they were gone.
                let call = format!("{name}(");
                !dispatchers.iter().any(|source| {
                    source.match_indices(&call).any(|(at, _)| {
                        let line_start =
                            source[..at].rfind('\n').map(|nl| nl + 1).unwrap_or(0);
                        !source[line_start..at].trim_start().starts_with("use ")
                    })
                })
            })
            .collect();
        assert!(
            orphans.is_empty(),
            "these app-control handlers are compiled, exported and reachable from \
             no CLI verb — the command exists and nothing can send it: {orphans:?}"
        );
    }

    /// ⛔ AND THE TWIN, BECAUSE THE HANDLER SCAN ABOVE CANNOT SEE IT.
    ///
    /// `audio` was lost the same way as the rest and left no orphaned
    /// `run_app_control_*` behind, because its whole surface lives in its own
    /// module and the dispatcher reached it with one line
    /// (`audio_cli::run_audio_command(&args)`). Delete that line and you get a
    /// module that still compiles, still exports, still has its help text — and
    /// answers nothing. **An entire CLI module can go dark without a single
    /// symbol becoming unused.**
    ///
    /// ⚠ This is why "the help still documents it" is not an available oracle
    /// either: the help was rewritten in the same commit, so help and
    /// dispatcher lost the verb TOGETHER and any consistency check between
    /// them stays green. Reachability is the only property that noticed.
    #[test]
    fn every_cli_module_is_reached_by_a_dispatcher() {
        const LIB: &str = include_str!("lib.rs");
        // ⚠ THE BINARIES ARE DISPATCHERS TOO, and leaving them out made this lock
        // fire on a module that is perfectly reachable.
        //
        // It went red the moment two lanes met on `main`: one added this lock,
        // the other added `server_cli` — a module BOTH BINARIES dispatch, not
        // one app-control routes to. Neither lane was wrong on its own and
        // neither could have seen it, because the collision exists only in the
        // merge. (Both lanes then fixed it independently and identically, and
        // this text is their merge.)
        //
        // ⛔ The alternative was an exclusion list naming the module, which is
        // exactly the hand-list this test's own doc argues against: a module
        // added tomorrow would be excused by nobody's decision. Naming the
        // dispatchers is a fact about the code; naming the exceptions is a guess
        // that rots. It would also blind the lock to that module going dark,
        // which is a live risk the day nine verbs moved into it.
        //
        // ⛔⛔ SCANNED PRODUCT-HALF ONLY, WITH COMMENTS STRIPPED, and both halves
        // of that were paid for. A whole-file scan matches the binaries' own TEST
        // assertions, which are written against these very strings. Stripping
        // tests alone is still not enough: the paragraph you are reading once
        // NAMED the needle it scans for, so the lock matched the prose describing
        // the defect it was suffering from. Reworded prose fixes that once; a
        // stripped scan fixes it for every future comment.
        //
        // ⭐ Neither blindness was visible from a green suite. Both were caught
        // by a decoy control — mutate both binaries to dispatch a module that
        // does not exist, and require this test to go RED. If it stays green,
        // the lock is reading something other than the code.
        let scannable = |source: &'static str| -> String {
            source
                .split("mod tests {")
                .next()
                .expect("a source has a product half above its tests")
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let dispatchers = [
            scannable(include_str!("app_control_cli.rs")),
            scannable(include_str!("app_control_web_cli.rs")),
            scannable(include_str!("../../../apps/yggterm/src/main.rs")),
            scannable(include_str!(
                "../../../apps/yggterm/src/bin/yggterm-headless.rs"
            )),
        ];
        let mut modules: Vec<&str> = Vec::new();
        for line in LIB.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("pub mod ")
                && let Some(name) = rest.strip_suffix(';')
                && name.ends_with("_cli")
                // The owner cannot be asked to reference itself.
                && name != "app_control_cli"
            {
                modules.push(name);
            }
        }
        assert!(
            !modules.is_empty(),
            "the CLI-module scan found none — it has gone blind"
        );
        let dark: Vec<&str> = modules
            .into_iter()
            .filter(|name| {
                let path = format!("{name}::");
                !dispatchers.iter().any(|source| source.contains(&path))
            })
            .collect();
        assert!(
            dark.is_empty(),
            "these CLI modules compile and export a surface that no dispatcher \
             can reach — every verb they define answers \"unsupported app control \
             command\": {dark:?}"
        );
    }
}

#[cfg(test)]
mod app_launch_duplicate_guard_tests {
    use super::*;
    use crate::ClientInstanceRecord;

    fn record(pid: u32, role: Option<&str>) -> ClientInstanceRecord {
        ClientInstanceRecord {
            pid,
            started_at_ms: 0,
            client_id: None,
            client_role: role.map(str::to_string),
            linux_desktop_app_id: None,
            process_start_ticks: None,
            executable_path: None,
            build_commit: None,
            display: None,
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: None,
            webkit_gl_environment: Default::default(),
        }
    }

    fn args(extra: &[&str]) -> Vec<String> {
        std::iter::once("launch")
            .chain(extra.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn an_empty_registry_launches() {
        assert_eq!(
            app_launch_duplicate_decision(&[], &args(&[])),
            AppLaunchDuplicateDecision::Launch
        );
    }

    #[test]
    fn a_live_active_gui_refuses_and_names_the_incumbent() {
        // The whole point of naming it: "a GUI is already running" with no pid
        // leaves the caller nothing to act on.
        assert_eq!(
            app_launch_duplicate_decision(&[record(4004668, Some("active"))], &args(&[])),
            AppLaunchDuplicateDecision::Refuse {
                incumbent_pids: vec![4004668]
            }
        );
    }

    #[test]
    fn a_legacy_record_with_no_role_counts_as_active() {
        // ⛔ Reading `None` as a shadow would disarm the guard on exactly the
        // records that predate the role field.
        assert_eq!(
            app_launch_duplicate_decision(&[record(1234, None)], &args(&[])),
            AppLaunchDuplicateDecision::Refuse {
                incumbent_pids: vec![1234]
            }
        );
    }

    #[test]
    fn shadows_are_not_duplicates() {
        // Agent shadows exist so probes stay off the user's seat; several may
        // be live and none of them owns the window.
        assert_eq!(
            app_launch_duplicate_decision(
                &[record(11, Some("shadow")), record(12, Some("Shadow"))],
                &args(&[])
            ),
            AppLaunchDuplicateDecision::Launch
        );
    }

    #[test]
    fn both_overrides_launch_past_a_live_gui() {
        for flag in ["--replace", "--allow-duplicate"] {
            assert_eq!(
                app_launch_duplicate_decision(&[record(99, Some("active"))], &args(&[flag])),
                AppLaunchDuplicateDecision::Launch,
                "{flag} should launch past a live GUI"
            );
        }
    }

    #[test]
    fn replace_targets_exactly_the_window_owners_and_never_a_shadow() {
        // `--replace` SIGTERMs whatever this returns. An agent's read-only
        // shadow must never be in that list: killing one would take out
        // another lane's probe surface as a side effect of a routine restart.
        let records = [
            record(10, Some("active")),
            record(11, Some("shadow")),
            record(12, None),
            record(13, Some("SHADOW")),
        ];
        assert_eq!(active_gui_pids(&records), vec![10, 12]);
    }

    #[test]
    fn replace_on_a_quiet_host_retires_nothing() {
        // The kill-then-launch ritual reaches here with an empty registry, and
        // it must not turn into an error or a stray signal.
        assert!(active_gui_pids(&[]).is_empty());
        assert!(app_launch_replace_requested(&args(&["--replace"])));
        assert!(!app_launch_replace_requested(&args(&[])));
    }

    #[test]
    fn every_active_incumbent_is_named_not_just_the_first() {
        // The failure this guard exists for produced TWO live GUIs; a message
        // naming one of them would have sent the reader after the wrong pid.
        assert_eq!(
            app_launch_duplicate_decision(
                &[
                    record(1, Some("active")),
                    record(2, Some("shadow")),
                    record(3, None),
                ],
                &args(&[])
            ),
            AppLaunchDuplicateDecision::Refuse {
                incumbent_pids: vec![1, 3]
            }
        );
    }
}
