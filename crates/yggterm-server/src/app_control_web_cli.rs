//! THE `server app web …` CLI — one owner, both binaries.
//!
//! This plane used to live inside `apps/yggterm/src/main.rs`, which meant it
//! existed on the GUI binary ONLY: `yggterm-headless server app web eval …`
//! answered `unsupported app control command: web` — on the binary the docs
//! and every agent skill tell agents to drive. The whole verb plane read as
//! nonexistent to the agent it was built for.
//!
//! The fix is NOT a copy of the arm in the second binary (415 lines of second
//! encoding that diverge on the first new verb — the same split-dispatch trap
//! the keytips arm already fell into once). Both binaries call
//! [`run_app_control_web_cli`], and both render [`web_usage_block`] into their
//! `server app --help`, so there is exactly one place that knows what
//! `server app web <verb>` MEANS, one place that documents it, and no way for
//! the two to disagree.
//!
//! The locks that keep it that way:
//!   * `every_web_action_appears_in_the_usage_string` (this file) — the
//!     dispatcher's own match arms are scanned and compared against
//!     [`WEB_ACTIONS`], so an implemented-and-undocumented verb fails the build.
//!   * `both_binaries_route_the_web_plane_to_its_one_owner`
//!     (`apps/yggterm/src/main.rs`) — derives the verb list from
//!     [`web_action_names`] and refuses either binary carrying a `web` verb
//!     dispatch of its own.

use anyhow::Context;
use std::io::Read;
use yggterm_core::{cli_flag_value, cli_positional_args};

use crate::{
    run_app_control_ensure_web_surface, run_app_control_web_surface_await,
    run_app_control_web_surface_batch, run_app_control_web_surface_capture_element,
    run_app_control_web_surface_close, run_app_control_web_surface_cookies,
    run_app_control_web_surface_devtools, run_app_control_web_surface_do,
    run_app_control_web_surface_eval, run_app_control_web_surface_fill,
    run_app_control_web_surface_fill_vault, run_app_control_web_surface_find,
    run_app_control_web_surface_frames, run_app_control_web_surface_lease,
    run_app_control_web_surface_read, run_app_control_web_surface_reload,
    run_app_control_web_surface_screenshot, run_app_control_web_surface_totp,
    run_app_control_web_surface_wait,
};

/// EVERY `server app web` action, with its usage.
///
/// This exists because a hand-maintained usage string is exactly the thing that
/// drifts: `ensure`, `fill` and `totp` were implemented and undocumented, and a
/// stale usage block is what produced a "not deployed" misdiagnosis in the
/// field on 2026-07-22 (docs/agent-control-plane.md). An agent reading `--help`
/// and concluding a verb does not exist is a failure mode of the DOCS.
///
/// One list, rendered into the usage text, and a test that fails when the
/// dispatcher's own match arms disagree with it. An alias carries an empty
/// usage string — it is named in its primary's line.
///
/// `{bin}` is the calling binary's name, filled in by [`web_usage_block`]: the
/// verbs are identical on `yggterm` and `yggterm-headless` (one owner runs
/// both), but the line an agent COPIES has to name the binary they typed, so
/// the plane is data with one hole in it rather than two lists.
pub const WEB_ACTIONS: &[(&str, &str)] = &[
    ("eval", "  {bin} server app web eval (<script>|--script <js>|--stdin) [--frame <f>] [--session <path>]\n"),
    ("read", "  {bin} server app web read [--as snapshot|forms|tables|readable|links|text|html] [--frame <f>] [--session <path>]\n    read with NO --frame searches EVERY reachable frame and returns\n    frames:[ {{frame:{{path,url}},result}} ] — the top document is frame []\n"),
    ("await", "  {bin} server app web await (<script>|--script <file>|--stdin) [--await-timeout <ms>] [--session <path>]\n    the script is the BODY of an async function; `return` its value.\n    `eval` cannot return a Promise — this is the one verb that can.\n"),
    ("frames", "  {bin} server app web frames [--session <path>]\n    --frame <f> is an index (2), a path (0.2), or a url substring (billdesk)\n"),
    ("do", "  {bin} server app web do <click|move|scroll|type|fill|key> <target> [--text …|--key …|--mods …] [--generation <n>] [--new-batch] [--session <path>]\n    target (resolved in the page at click time, precedence in this order):\n      --selector <css> [--nth <n>] | --role <r> --label <s> [--nth <n>]\n      | --target-text <s> [--exact] [--tag <css>] [--nth <n>]   (on `click`, --text is an alias)\n      | --selector-set <css,css,…>   (segmented inputs: one box per character)\n      | --x <n> --y <n>              (blind coordinates; prefer an addressed target)\n    fill only: --mechanism <auto|real-keys|native-setter>  (auto: native setter on plain text inputs)\n               --redact                                    (secret: keep the value out of the response)\n    every addressed response carries `match` {matches,nth,hidden,ambiguous} — a\n    selector matching >1 node is reported, never silently resolved to the first.\n"),
    ("fill-vault", "  {bin} server app web fill-vault --item <name> [--field password|username|totp|notes] [--user <u>] <target> [--session <path>]\n"),
    ("fill-card", "  {bin} server app web fill-card --item <name> [--field number|code|holder|exp-month|exp-year|expiry] <target> [--session <path>]\n    reads the vault agent's card-secret op (NOT the CLI — no verb prints a PAN)\n    and types it with real keys. formats: exp-month MM, exp-year as stored\n    (usually YYYY), expiry MM/YY. needs the vault UNLOCKED and nothing else;\n    a locked one refuses `vault_locked` naming `ychrome-vault unlock`.\n    the answer is {item, field, chars, matched} — a name and a length, never\n    the value. every fill leaves one line in ~/.yggterm/vault/audit.log.\n"),
    ("fill", "  {bin} server app web fill [--entry <name>] [--user <u>] [--session <path>]\n    auto-match the page host against the vault and fill the login form.\n    For ONE named field into ONE addressed element, use fill-vault.\n"),
    ("totp", "  {bin} server app web totp [--entry <name>] [--user <u>] [--session <path>]  (alias: code)\n    put the entry's current TOTP code into the page's one-time-code field\n"),
    ("batch", "  {bin} server app web batch (--script <file>|--stdin) [--stop-on-error] [--generation <n>] [--session <path>]\n    one `do` invocation per line; # comments and blank lines skipped\n"),
    ("wait", "  {bin} server app web wait --until <cond> [--visible] [--wait-timeout <ms>] [--session <path>]\n    cond: load:committed | load:finished | idle:<ms> | settled:<ms>\n        | selector:<css> | js:<expr> | url:matches:<regex> | url:contains:<substring>\n    url:* and settled:* are read from the ENGINE, so they survive a navigation\n    that makes every page-side predicate unavailable\n"),
    ("ensure", "  {bin} server app web ensure --session <path> [--ttl <secs>]\n    LIVENESS-based: probes the page with a real round trip, rebuilds a corpse,\n    and reports generation_before/generation_after + healed so a caller can tell\n    a new page from the same one. Refusals name WHICH fact failed (no_declare,\n    declare_stale, declare_url_scheme_refused, daemon_declare_unavailable, ...).\n    session_closed is NOT retryable: the runtime is gone AND the user closed the\n    row, so a revived surface would have no row the user can see or click into.\n    Create your own session (`server app terminal new`) and drive that.\n"),
    ("reload", "  {bin} server app web reload --session <path>\n"),
    ("close", "  {bin} server app web close --session <path>\n"),
    (
        "lease",
        "  {bin} server app web lease --ttl <secs> [--session <path>]\n",
    ),
    (
        "screenshot",
        "  {bin} server app web screenshot [output.png] [--session <path>]\n",
    ),
    (
        "cookies",
        "  {bin} server app web cookies (--import <jar>|--export <jar>) [--session <path>]\n    Netscape format, both ways (what `curl -c`/`-b` writes and reads).\n    WARNING: the jar is per-PROFILE; an unqualified surface is `default`, the\n    user's own browsing jar. Use an `agent-<n>` profile surface. Export covers\n    every ROOT-PATH cookie per domain — path-scoped cookies are not visible to\n    the engine API and are reported as export_scope=root_path_per_domain.\n",
    ),
    (
        "capture-element",
        "  {bin} server app web capture-element <target> [out.png] [--split <n>] [--session <path>]\n    in-page canvas rasterize of one <img>/<canvas>/<video>; works on an UNMAPPED surface\n",
    ),
    (
        "devtools",
        "  {bin} server app web devtools [--close] [--session <path>]\n",
    ),
    (
        "find",
        "  {bin} server app web find --text <needle> [--next|--prev|--close] [--session <path>]\n    find-in-page through WebKit's own find controller — the same mechanism the\n    Ctrl+F bar drives, so what this reports is what the user sees.\n    answers {match_count, position, label} — the count is the ENGINE's and is\n    UNCAPPED (a capped count is reported as if it were the total), the position\n    is 1-based and wraps. case-insensitive. --close finishes the search, which\n    is what clears the highlights, and closes any bar the user had open.\n",
    ),
    ("code", ""),
    ("capture", ""),
];

/// Render the `server app web` usage block from [`WEB_ACTIONS`], with `binary`
/// (the invoked CLI's own name) filled into every usage line.
///
/// Both binaries interpolate this into their `server app --help`, so the
/// documented verb set cannot differ between them — only the binary name in
/// the copyable command does.
pub fn web_usage_block(binary: &str) -> String {
    WEB_ACTIONS
        .iter()
        .map(|(_, usage)| usage.replace("{bin}", binary))
        .collect()
}

/// Every action name this CLI accepts, aliases included — DERIVED from
/// [`WEB_ACTIONS`], which the drift lock below pins to the dispatcher's own
/// match arms. A parity lock that hand-lists verbs goes stale the first time
/// someone adds one; this one cannot.
pub fn web_action_names() -> Vec<&'static str> {
    WEB_ACTIONS.iter().map(|(name, _)| *name).collect()
}

/// Parse the element-addressing flags shared by every `do` verb into ONE
/// [`WebElementRef`] (C4). One parser, so `click`, `type`, `key` and `fill`
/// can never disagree about how an element is named.
///
/// Precedence is fixed and documented rather than "most specific wins":
/// `--selector` (CSS) > `--role`+`--label` > text. A fixed order is what keeps
/// a script's meaning stable when someone adds a stray flag.
///
/// `text_addresses` is true only for verbs that carry NO text payload
/// (`click`), where `--text "Proceed to Pay"` is the natural spelling. For
/// `type`/`fill`, `--text` is the value being typed, so text addressing spells
/// itself `--target-text` — which is also accepted everywhere, so a script can
/// always use the unambiguous form.
fn parse_web_element_ref(
    args: &[String],
    text_addresses: bool,
) -> anyhow::Result<Option<crate::WebElementRef>> {
    use crate::WebElementRef;
    let nth = cli_flag_value(args, "--nth")
        .map(|raw| raw.parse::<usize>().context("--nth needs a number"))
        .transpose()?;
    if let Some(selector) = cli_flag_value(args, "--selector") {
        // `--nth` applies to a CSS selector too. A page that renders the same
        // ids in two form blocks (a services portal's complainant/opposite-party pair)
        // makes `#Name` ambiguous, and `querySelector` answers with the first
        // silently — so the index has to be expressible. `nth: 0` serializes
        // back to the bare string every existing payload uses.
        return Ok(Some(match nth {
            Some(0) | None => WebElementRef::Css(selector.to_string()),
            Some(nth) => WebElementRef::CssNth {
                css: selector.to_string(),
                nth,
            },
        }));
    }
    if let Some(role) = cli_flag_value(args, "--role") {
        let label = cli_flag_value(args, "--label")
            .context("--role needs --label (the element's accessible name)")?;
        return Ok(Some(WebElementRef::Role {
            role: role.to_string(),
            label: label.to_string(),
            nth,
        }));
    }
    let text = cli_flag_value(args, "--target-text").or_else(|| {
        if text_addresses {
            cli_flag_value(args, "--text")
        } else {
            None
        }
    });
    if let Some(text) = text {
        return Ok(Some(WebElementRef::Text {
            text: text.to_string(),
            exact: args.iter().any(|arg| arg == "--exact"),
            tag: cli_flag_value(args, "--tag").map(str::to_string),
            nth,
        }));
    }
    Ok(None)
}

/// Escape a literal so it matches itself as a regex.
///
/// `--until url:contains:<s>` is SUGAR, not a second predicate: it compiles to
/// the same `UrlMatches` the regex form does, so there is exactly one url rule
/// in the GUI. Escaping happens here, where the user's literal is, rather than
/// by teaching the matcher a second mode.
fn regex_escape_literal(literal: &str) -> String {
    let mut escaped = String::with_capacity(literal.len() * 2);
    for c in literal.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// Parse `--frame <index|path|url-substring>` into a [`WebFrameRef`].
///
/// Three spellings, one meaning: a bare number is `window.frames[n]`, a
/// dotted/comma path (`0.2`) is a descent — the form `web frames` reports, so
/// its output feeds straight back in — and anything else is a url substring.
fn parse_web_frame_ref(args: &[String]) -> anyhow::Result<Option<crate::WebFrameRef>> {
    use crate::WebFrameRef;
    let Some(raw) = cli_flag_value(args, "--frame") else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("--frame needs an index, a path like 0.2, or a url substring");
    }
    if let Ok(index) = raw.parse::<usize>() {
        return Ok(Some(WebFrameRef::Index(index)));
    }
    let separators: &[char] = &['.', ','];
    if raw.contains(separators)
        && raw
            .split(separators)
            .all(|part| part.trim().parse::<usize>().is_ok())
    {
        let path = raw
            .split(separators)
            .map(|part| part.trim().parse::<usize>().unwrap_or(0))
            .collect();
        return Ok(Some(WebFrameRef::Path(path)));
    }
    Ok(Some(WebFrameRef::UrlContains(raw.to_string())))
}

/// Split one batch-script line into argv tokens, honouring `'`/`"` quoting and
/// backslash escapes.
///
/// A batch line IS a `do` invocation, so it has to tokenize the way a shell
/// would or `--text "Proceed to Pay"` would arrive as three arguments. Kept
/// deliberately small: quotes and backslashes, no expansion, no globbing, no
/// variables — a batch script is a list of verbs, not a shell.
fn tokenize_argv_line(line: &str) -> anyhow::Result<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let escaped = chars.next().context("trailing backslash in batch line")?;
                current.push(escaped);
                has_token = true;
            }
            '\'' | '"' if quote.is_none() => {
                quote = Some(c);
                has_token = true;
            }
            c if Some(c) == quote => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if quote.is_some() {
        anyhow::bail!("unterminated quote in batch line");
    }
    if has_token {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Parse a batch script — one `do`-style invocation per line — into actions.
///
/// Blank lines and `#` comments are skipped. Every line goes through the SAME
/// `parse_web_surface_do_action` a CLI verb goes through, so a batched action
/// and a typed one can never mean different things; that shared parser is the
/// single source of truth for what a `do` verb IS.
fn parse_web_do_batch_script(
    script: &str,
) -> anyhow::Result<Vec<crate::WebSurfaceDoAction>> {
    let mut actions = Vec::new();
    for (index, line) in script.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let tokens =
            tokenize_argv_line(trimmed).with_context(|| format!("batch line {}", index + 1))?;
        // `parse_web_surface_do_action` reads the verb at args[4] — the shape a
        // real `server app web do …` invocation has — so the line is spliced
        // onto that prefix rather than parsed by a second, parallel reader.
        let mut argv: Vec<String> = ["server", "app", "web", "do"]
            .iter()
            .map(|part| (*part).to_string())
            .collect();
        argv.extend(tokens);
        actions.push(
            parse_web_surface_do_action(&argv)
                .with_context(|| format!("batch line {}: {trimmed}", index + 1))?,
        );
    }
    if actions.is_empty() {
        anyhow::bail!("batch script contained no actions");
    }
    Ok(actions)
}

/// Parse a `server app web do <verb> …` invocation into a typed
/// `WebSurfaceDoAction` (agent control plane `do` verb, slice 2b). Coordinates
/// are document-space CSS pixels; the GUI resolves selectors + maps to widget
/// px. `args[4]` is the verb (`click`/`move`/`scroll`/`type`/`key`).
fn parse_web_surface_do_action(
    args: &[String],
) -> anyhow::Result<crate::WebSurfaceDoAction> {
    use crate::{AppControlPointerButton, WebSurfaceDoAction};
    let verb = args
        .get(4)
        .map(String::as_str)
        .context("missing verb for server app web do (click|move|scroll|type|fill|key)")?;
    let button = match cli_flag_value(args, "--button") {
        Some("middle" | "auxiliary" | "2") => AppControlPointerButton::Middle,
        Some("secondary" | "right" | "3") => AppControlPointerButton::Secondary,
        _ => AppControlPointerButton::Primary,
    };
    let req_f64 = |flag: &str| -> anyhow::Result<f64> {
        cli_flag_value(args, flag)
            .with_context(|| format!("missing {flag} for server app web do {verb}"))?
            .parse::<f64>()
            .with_context(|| format!("invalid number for {flag}"))
    };
    let opt_f64 = |flag: &str| cli_flag_value(args, flag).and_then(|v| v.parse::<f64>().ok());
    let action = match verb {
        "click" | "tap" => {
            // Addressed click (CSS / text / role+label) when any addressing flag
            // is present; blind coordinates only when none is. The addressed
            // path resolves in the page immediately before injection, which is
            // both safer (it hit-tests) and stale-proof.
            if let Some(target) = parse_web_element_ref(args, true)? {
                WebSurfaceDoAction::ClickSelector {
                    selector: target,
                    button,
                }
            } else {
                WebSurfaceDoAction::Click {
                    x: req_f64("--x")?,
                    y: req_f64("--y")?,
                    button,
                }
            }
        }
        "move" | "hover" => WebSurfaceDoAction::Move {
            x: req_f64("--x")?,
            y: req_f64("--y")?,
        },
        "scroll" => {
            let dx = opt_f64("--dx").unwrap_or(0.0);
            let dy = opt_f64("--dy").unwrap_or(0.0);
            if dx == 0.0 && dy == 0.0 {
                anyhow::bail!("server app web do scroll needs --dx and/or --dy");
            }
            WebSurfaceDoAction::Scroll {
                x: opt_f64("--x"),
                y: opt_f64("--y"),
                dx,
                dy,
            }
        }
        "type" => WebSurfaceDoAction::Type {
            text: cli_flag_value(args, "--text")
                .context("missing --text for server app web do type")?
                .to_string(),
            selector: parse_web_element_ref(args, false)?,
        },
        // `fill` REPLACES a field's contents; `type` appends. Use it whenever the
        // field may already hold something — see the merge failure documented on
        // `WebSurfaceDoAction::Fill`. `--selector-set` is the comma-separated
        // box list of a segmented input (a 6-box OTP), which must be cleared
        // box-by-box or the old characters survive inside the widget.
        "fill" | "set" => WebSurfaceDoAction::Fill {
            text: cli_flag_value(args, "--text")
                .context("missing --text for server app web do fill")?
                .to_string(),
            selector: parse_web_element_ref(args, false)?,
            selectors: cli_flag_value(args, "--selector-set")
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|css| crate::WebElementRef::Css(css.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            // Default `auto`: a plain text/textarea input gets the native
            // setter (React controlled inputs drop injected keystrokes), and
            // anything else keeps real keys.
            mechanism: match cli_flag_value(args, "--mechanism") {
                Some("real-keys" | "real_keys" | "keys") => {
                    crate::WebFillMechanism::RealKeys
                }
                Some("native-setter" | "native_setter" | "native") => {
                    crate::WebFillMechanism::NativeSetter
                }
                Some("auto") | None => crate::WebFillMechanism::Auto,
                Some(other) => {
                    anyhow::bail!("unsupported --mechanism: {other} (auto|real-keys|native-setter)")
                }
            },
            redact: args.iter().any(|arg| arg == "--redact"),
        },
        "key" | "press" => WebSurfaceDoAction::Key {
            key: cli_flag_value(args, "--key")
                .context("missing --key for server app web do key")?
                .to_string(),
            mods: cli_flag_value(args, "--mods")
                .map(|raw| raw.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
            selector: parse_web_element_ref(args, false)?,
        },
        other => {
            anyhow::bail!("unsupported web do verb: {other} (click|move|scroll|type|fill|key)")
        }
    };
    Ok(action)
}
/// Run one `server app web <verb> …` invocation.
///
/// `args` is the whole argv tail (`["server", "app", "web", <verb>, …]`) so the
/// verb sits at `args[3]`, exactly as both binaries' `server app` dispatchers
/// hold it. THE single owner: `apps/yggterm/src/main.rs` and
/// `apps/yggterm/src/bin/yggterm-headless.rs` each route their `"web"` arm
/// straight here and carry no verb knowledge of their own.
pub fn run_app_control_web_cli(args: &[String], timeout_ms: u64) -> anyhow::Result<()> {
    // Web-surface (ychrome) automation: the agent is a first-class
    // user of pages. `--session <path>` targets a session's active
    // surface tab; omitted = the active session.
    let action = args
        .get(3)
        .map(String::as_str)
        .context("missing action for server app web")?;
    let session_path = cli_flag_value(args, "--session");
    match action {
        "eval" => {
            let script = if args.iter().any(|arg| arg == "--stdin") {
                let mut value = String::new();
                std::io::stdin()
                    .read_to_string(&mut value)
                    .context("reading app web eval stdin")?;
                value
            } else {
                cli_flag_value(args, "--script")
                    .map(str::to_string)
                    .or_else(|| {
                        cli_positional_args(args, 4)
                            .into_iter()
                            .next()
                            .map(str::to_string)
                    })
                    .context("missing script (positional, --script or --stdin) for server app web eval")?
            };
            run_app_control_web_surface_eval(
                session_path,
                &script,
                parse_web_frame_ref(args)?,
                timeout_ms,
            )
        }
        "screenshot" => {
            let output = cli_positional_args(args, 4)
                .into_iter()
                .next()
                .unwrap_or("web-surface.png");
            run_app_control_web_surface_screenshot(session_path, output, timeout_ms)
        }
        "await" => {
            // The ONE async bridge:
            //   web await (--script <file>|--stdin) [--await-timeout <ms>]
            // The script is the BODY of an async function; `return`
            // its value. `eval` cannot return a Promise, and this
            // is the verb that means nobody has to hand-roll a
            // stash-and-poll around that fact again.
            let script = if args.iter().any(|arg| arg == "--stdin") {
                let mut value = String::new();
                std::io::stdin()
                    .read_to_string(&mut value)
                    .context("reading app web await stdin")?;
                value
            } else if let Some(path) = cli_flag_value(args, "--script") {
                // A FILE by default, unlike `eval`'s `--script`,
                // because an async body is rarely a one-liner. A
                // path that does not exist is read as the script
                // itself rather than failing silently.
                std::fs::read_to_string(path).unwrap_or_else(|_| path.to_string())
            } else {
                cli_positional_args(args, 4)
                    .into_iter()
                    .next()
                    .map(str::to_string)
                    .context("missing script (positional, --script <file> or --stdin) for server app web await")?
            };
            let await_timeout_ms = cli_flag_value(args, "--await-timeout")
                .map(|raw| raw.parse::<u64>().context("--await-timeout needs ms"))
                .transpose()?
                .unwrap_or(15_000);
            run_app_control_web_surface_await(session_path, &script, await_timeout_ms)
        }
        "frames" => {
            // What frames this page has, and how much is IN each:
            //   web frames [--session <path>]
            // A top-document `read` returning [] next to a frame
            // reporting 107 elements is a legible answer; the []
            // alone was not.
            run_app_control_web_surface_frames(session_path, timeout_ms)
        }
        "cookies" => {
            // Move the surface's cookie jar to or from a Netscape
            // file — the format `curl -c`/`-b` speaks:
            //   web cookies --import <jar> | --export <jar>
            // This is what makes a flow SPLITTABLE: script the
            // mechanical parts on curl, hand the session to a
            // surface for the one interactive step, hand it back.
            //
            // ⚠ The jar is per-PROFILE and an unqualified surface
            // is `default` — the user's own browsing jar. Drive
            // agent work on a `--profile agent-<n>` surface before
            // importing. The response reports which profile was
            // written; check it.
            use crate::WebCookieDirection;
            let (direction, jar) = match (
                cli_flag_value(args, "--import"),
                cli_flag_value(args, "--export"),
            ) {
                (Some(jar), None) => (WebCookieDirection::Import, jar),
                (None, Some(jar)) => (WebCookieDirection::Export, jar),
                (Some(_), Some(_)) => anyhow::bail!(
                    "web cookies takes --import <jar> OR --export <jar>, not both"
                ),
                (None, None) => anyhow::bail!(
                    "web cookies needs --import <jar> or --export <jar> (Netscape format)"
                ),
            };
            run_app_control_web_surface_cookies(
                session_path,
                direction,
                jar,
                timeout_ms,
            )
        }
        "capture-element" | "capture" => {
            // Rasterize ONE addressed element to a PNG, in the page:
            //   web capture-element <target> [out.png] [--split <n>]
            // Compositor-independent, so it works on an unmapped
            // surface — <img>/<canvas>/<video> only, and every other
            // element gets a named refusal rather than a blank file.
            let target = parse_web_element_ref(args, true)?.context(
                "capture-element needs a target: --selector <css>, --role <r> --label <s>, \
                 or --text <s>",
            )?;
            let output = cli_positional_args(args, 4)
                .into_iter()
                .next()
                .unwrap_or("web-element.png");
            let split = cli_flag_value(args, "--split")
                .map(|raw| raw.parse::<usize>().context("--split needs a number"))
                .transpose()?;
            run_app_control_web_surface_capture_element(
                session_path,
                target,
                output,
                split,
                timeout_ms,
            )
        }
        "devtools" => {
            let open = !args.iter().any(|arg| arg == "--close");
            run_app_control_web_surface_devtools(session_path, open, timeout_ms)
        }
        "find" => {
            // Find-in-page through WebKit's own find controller —
            // the agent's door onto the Ctrl+F bar's mechanism:
            //   web find --text <needle> [--next|--prev|--close]
            // Answers with `match_count` (the ENGINE's number for
            // the page, uncapped) and `position` (1-based, wrapping)
            // so `3/17` is readable without a screenshot.
            //
            // A step flag is exclusive: asking for two at once is a
            // typo, and guessing which one was meant is how a test
            // surface starts lying.
            let steps: Vec<&str> = [("--next", "next"), ("--prev", "prev"), ("--previous", "prev"), ("--close", "close")]
                .into_iter()
                .filter(|(flag, _)| args.iter().any(|arg| arg == flag))
                .map(|(_, step)| step)
                .collect();
            let step = match steps.as_slice() {
                [] => "search",
                [only] => only,
                _ => anyhow::bail!(
                    "web find takes at most one of --next / --prev / --close"
                ),
            };
            let text = cli_flag_value(args, "--text").or_else(|| {
                cli_positional_args(args, 4).into_iter().next()
            });
            if text.is_none() && step != "close" {
                anyhow::bail!(
                    "web find needs --text <needle> (or a positional needle); \
                     only --close may omit it"
                );
            }
            run_app_control_web_surface_find(session_path, text, step, timeout_ms)
        }
        "fill" => {
            let entry = cli_flag_value(args, "--entry");
            let user = cli_flag_value(args, "--user");
            run_app_control_web_surface_fill(session_path, entry, user, timeout_ms)
        }
        "fill-vault" | "fill-card" => {
            // Type ONE named vault field into ONE addressed
            // element, with real keys:
            //   web fill-vault --item <name> --field password
            //                  (--selector <css>|--role …|--target-text …)
            //   web fill-card  --item <name> --field number …
            // The secret never reaches this process: the CLI names
            // the item and the field, the GUI reads and types it,
            // and the answer is a length plus a boolean.
            let source = if action == "fill-card" {
                crate::VaultFieldSource::Card
            } else {
                crate::VaultFieldSource::Login
            };
            let target = parse_web_element_ref(args, false)?.context(
                "fill-vault needs a target: --selector <css>, --role <r> --label <s>, \
                 or --target-text <s>",
            )?;
            let item = cli_flag_value(args, "--item")
                .context("missing --item (the vault entry NAME) for web fill-vault")?;
            let field =
                cli_flag_value(args, "--field").unwrap_or(if action == "fill-card" {
                    "number"
                } else {
                    "password"
                });
            let user = cli_flag_value(args, "--user");
            let generation = cli_flag_value(args, "--generation")
                .map(|raw| raw.parse::<u64>().context("--generation needs a number"))
                .transpose()?;
            run_app_control_web_surface_fill_vault(
                session_path,
                target,
                item,
                field,
                user,
                source,
                generation,
                timeout_ms,
            )
        }
        "totp" | "code" => {
            let entry = cli_flag_value(args, "--entry");
            let user = cli_flag_value(args, "--user");
            run_app_control_web_surface_totp(session_path, entry, user, timeout_ms)
        }
        "do" => {
            // Trusted action injection (agent control plane slice 2b):
            //   web do click   --selector <css> | --x <n> --y <n> [--button …]
            //   web do move    --x <n> --y <n>
            //   web do scroll  [--x --y] --dx <n> --dy <n>
            //   web do type    --text "…" [--selector <css>]
            //   web do key     --key Enter [--mods ctrl,shift]
            // `--generation <n>` pins the surface incarnation the
            // verb was issued against: if the webview has been
            // destroyed and rebuilt since (reload, profile/proxy
            // change, hold expiry), the verb fails closed with
            // `stale_handle` rather than acting on a page the
            // caller never observed (F3). Every response reports
            // the current `generation` to pin the next call with.
            let action = parse_web_surface_do_action(args)?;
            let generation = cli_flag_value(args, "--generation")
                .map(|raw| raw.parse::<u64>().context("--generation needs a number"))
                .transpose()?;
            // `--new-batch` is the documented recovery from a
            // `preempted` refusal: the agent asserts it re-observed
            // the page, and the surface's batch lane is reopened.
            let new_batch = args.iter().any(|arg| arg == "--new-batch");
            run_app_control_web_surface_do(
                session_path,
                action,
                generation,
                new_batch,
                timeout_ms,
            )
        }
        "batch" => {
            // One explicitly-opened agent batch, N verbs, one gate:
            //   web batch --script <file> [--stop-on-error]
            //             [--generation <n>] [--session <path>]
            // Each line is a `do` invocation; the human still wins
            // mid-batch (the GUI re-reads seat input between
            // actions and aborts the remainder).
            let script = if args.iter().any(|arg| arg == "--stdin") {
                let mut value = String::new();
                std::io::stdin()
                    .read_to_string(&mut value)
                    .context("reading app web batch stdin")?;
                value
            } else {
                let path = cli_flag_value(args, "--script").context(
                    "missing --script <file> (or --stdin) for server app web batch",
                )?;
                std::fs::read_to_string(path)
                    .with_context(|| format!("reading batch script {path}"))?
            };
            let actions = parse_web_do_batch_script(&script)?;
            let generation = cli_flag_value(args, "--generation")
                .map(|raw| raw.parse::<u64>().context("--generation needs a number"))
                .transpose()?;
            let stop_on_error = args.iter().any(|arg| arg == "--stop-on-error");
            run_app_control_web_surface_batch(
                session_path,
                actions,
                generation,
                stop_on_error,
            )
        }
        "ensure" => {
            // Headless surface-create: materialize a BACKGROUNDED
            // session's declared web surfaces into the soft stash
            // (never revealed) so agent verbs can drive them:
            //   web ensure --session <path> [--ttl <secs>]
            let ttl_secs = cli_flag_value(args, "--ttl")
                .map(|raw| raw.parse::<u64>().context("--ttl needs a number"))
                .transpose()?;
            let session = session_path
                .context("web ensure needs --session <path> (a backgrounded surface has no active default)")?;
            run_app_control_ensure_web_surface(session, ttl_secs, timeout_ms)
        }
        "reload" | "close" => {
            // Recover a surface without destroying its session:
            //   web reload --session <path>   (new incarnation)
            //   web close  --session <path>
            // Both report generation_before; compare it against a
            // following `web ensure`'s generation_after to tell a
            // HEALED surface from the same corpse.
            let session =
                session_path.context("web reload/close needs --session <path>")?;
            if action == "reload" {
                run_app_control_web_surface_reload(session, timeout_ms)
            } else {
                run_app_control_web_surface_close(session, timeout_ms)
            }
        }
        "lease" => {
            // Claim the surface so the background reaper leaves it
            // alone while unattended work runs:
            //   web lease --ttl <secs>   (0 releases the claim)
            // The lease only ever EXTENDS the background hold — it
            // cannot cut one short, so leasing can never destroy a
            // surface the user is about to return to.
            let ttl_secs = cli_flag_value(args, "--ttl")
                .context("missing --ttl for server app web lease")?
                .parse::<u64>()
                .context("--ttl needs a number of seconds")?;
            run_app_control_web_surface_lease(session_path, ttl_secs, timeout_ms)
        }
        "read" => {
            // Structured read-only observation (agent control plane
            // slice 2b, rung 1):
            //   web read [--as snapshot|forms|tables|readable|links|text|html]
            let mode = match cli_flag_value(args, "--as").unwrap_or("snapshot") {
                "snapshot" | "interactable" | "tree" => {
                    crate::WebSurfaceReadAs::Snapshot
                }
                "forms" | "form" => crate::WebSurfaceReadAs::Forms,
                "tables" | "table" => crate::WebSurfaceReadAs::Tables,
                "readable" | "article" => crate::WebSurfaceReadAs::Readable,
                "links" | "link" => crate::WebSurfaceReadAs::Links,
                "text" => crate::WebSurfaceReadAs::Text,
                "html" => crate::WebSurfaceReadAs::Html,
                other => anyhow::bail!(
                    "unknown --as for web read: {other} (snapshot|forms|tables|readable|links|text|html)"
                ),
            };
            run_app_control_web_surface_read(
                session_path,
                mode,
                parse_web_frame_ref(args)?,
                timeout_ms,
            )
        }
        "wait" => {
            // Event-driven synchronization (agent control plane slice
            // 2b, rung 2) — no more screenshot-poll loops:
            //   web wait --until load:finished|load:committed|idle:<ms>
            //                    |selector:<css> [--visible] |js:<expr>
            //            [--wait-timeout <ms>]
            use crate::WebSurfaceWaitUntil;
            let raw = cli_flag_value(args, "--until")
                .context("missing --until for server app web wait")?;
            let visible = args.iter().any(|arg| arg == "--visible");
            let until = match raw.split_once(':') {
                Some(("load", "committed")) => WebSurfaceWaitUntil::LoadCommitted,
                Some(("load", "finished")) => WebSurfaceWaitUntil::LoadFinished,
                Some(("idle", ms)) => WebSurfaceWaitUntil::Idle {
                    ms: ms.parse().context("--until idle:<ms> needs a number")?,
                },
                Some(("selector", css)) => WebSurfaceWaitUntil::Selector {
                    css: css.to_string(),
                    visible,
                },
                Some(("js", expr)) => WebSurfaceWaitUntil::Js {
                    expr: expr.to_string(),
                },
                // `url:matches:<re>` and its sugar `url:contains:<s>`
                // compile to ONE predicate: the sugar is escaped
                // into a regex here rather than becoming a second
                // matching rule in the GUI.
                Some(("url", rest)) => match rest.split_once(':') {
                    Some(("matches", pattern)) => WebSurfaceWaitUntil::UrlMatches {
                        pattern: pattern.to_string(),
                    },
                    Some(("contains", needle)) => WebSurfaceWaitUntil::UrlMatches {
                        pattern: regex_escape_literal(needle),
                    },
                    _ => anyhow::bail!(
                        "bad --until url:… ({rest}) — use url:matches:<regex> or url:contains:<substring>"
                    ),
                },
                Some(("settled", ms)) => WebSurfaceWaitUntil::Settled {
                    ms: ms.parse().context("--until settled:<ms> needs a number")?,
                },
                _ => match raw {
                    "committed" => WebSurfaceWaitUntil::LoadCommitted,
                    "finished" | "load" | "loaded" => WebSurfaceWaitUntil::LoadFinished,
                    other => anyhow::bail!(
                        "bad --until: {other} (load:committed|load:finished|idle:<ms>|settled:<ms>|selector:<css>|js:<expr>|url:matches:<re>|url:contains:<s>)"
                    ),
                },
            };
            let wait_timeout_ms = cli_flag_value(args, "--wait-timeout")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10_000);
            run_app_control_web_surface_wait(session_path, until, wait_timeout_ms)
        }
        other => anyhow::bail!("unsupported app web action: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action name the `server app web` dispatcher accepts, read from the
    /// dispatcher's own match arms.
    ///
    /// This is a SCANNER, and a scanner that silently matches nothing passes
    /// green while proving nothing — the exact failure a brace-counting lock in
    /// this repo shipped with. So it returns the arms AND the test asserts a
    /// coverage floor below.
    pub(crate) fn dispatcher_web_actions() -> Vec<String> {
        let source = include_str!("app_control_web_cli.rs");
        let start = source
            .find("pub fn run_app_control_web_cli(")
            .expect("the web dispatcher block moved; fix this scanner, do not delete it");
        let end = source[start..]
            .find("other => anyhow::bail!(\"unsupported app web action")
            .expect("the web dispatcher's catch-all moved")
            + start;
        let mut actions = Vec::new();
        // The OUTER match's arms sit at exactly this indentation. Matching on
        // indentation is what keeps the INNER matches out of the answer — the
        // `--as` modes and `--until` forms are options of a verb, not verbs,
        // and counting them would make this lock unsatisfiable and then
        // deleted. (Starting the scan at the dispatcher's own signature is what
        // keeps `parse_web_surface_do_action`'s same-indentation arms out too:
        // `click`/`move`/`scroll` are `do` sub-verbs, not `web` verbs.)
        const ARM_INDENT: &str = "        \"";
        for line in source[start..end].lines() {
            if !line.starts_with(ARM_INDENT) {
                continue;
            }
            let trimmed = line.trim();
            // An arm looks like:  "eval" => {   or   "totp" | "code" => {
            let Some(head) = trimmed.split("=>").next() else {
                continue;
            };
            if !trimmed.contains("=>") || !head.trim_start().starts_with('"') {
                continue;
            }
            for part in head.split('|') {
                let part = part.trim();
                if let Some(name) = part
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                {
                    actions.push(name.to_string());
                }
            }
        }
        actions
    }

    /// THE DRIFT LOCK. `ensure`, `fill` and `totp` were implemented and
    /// undocumented, and a stale usage block is what produced a "not deployed"
    /// misdiagnosis in the field — an agent read `--help`, did not see the
    /// verb, and concluded the build lacked it.
    ///
    /// Fails today against the pre-D1 usage string, which is the point.
    #[test]
    fn every_web_action_appears_in_the_usage_string() {
        let dispatcher = dispatcher_web_actions();
        // COVERAGE FLOOR: a scanner that finds nothing must fail, not pass.
        assert!(
            dispatcher.len() >= 15,
            "the arm scanner found only {} actions — it went blind; fix it rather than \
             lowering this floor",
            dispatcher.len()
        );
        assert!(
            dispatcher.contains(&"eval".to_string()),
            "sanity: {dispatcher:?}"
        );

        let documented: std::collections::BTreeSet<&str> =
            WEB_ACTIONS.iter().map(|(name, _)| *name).collect();
        let implemented: std::collections::BTreeSet<String> = dispatcher.iter().cloned().collect();

        let undocumented: Vec<&String> = implemented
            .iter()
            .filter(|name| !documented.contains(name.as_str()))
            .collect();
        assert!(
            undocumented.is_empty(),
            "these web actions are implemented and undocumented: {undocumented:?} — add them to \
             WEB_ACTIONS. An agent that reads --help and does not see a verb concludes the build \
             lacks it."
        );

        let phantom: Vec<&&str> = documented
            .iter()
            .filter(|name| !implemented.contains(&(**name).to_string()))
            .collect();
        assert!(
            phantom.is_empty(),
            "these web actions are documented and NOT implemented: {phantom:?} — worse than an \
             omission, because it sends a caller after a verb that will never answer."
        );
    }

    /// The rendered block must actually contain a line per non-alias action —
    /// a `WEB_ACTIONS` entry with an empty usage string would satisfy the set
    /// comparison above while printing nothing.
    #[test]
    fn the_rendered_usage_names_every_non_alias_action() {
        let rendered = web_usage_block("yggterm-headless");
        // The `{bin}` hole is FILLED, on whichever binary asks: a usage line
        // that still reads `{bin} server app web eval` is not a command anyone
        // can copy.
        assert!(
            !rendered.contains("{bin}"),
            "an unfilled {{bin}} survived into the rendered usage block"
        );
        assert!(
            rendered.contains("  yggterm-headless server app web eval "),
            "the rendered block must name the binary that asked for it"
        );
        for (name, usage) in WEB_ACTIONS {
            if usage.is_empty() {
                // An alias: it must still be findable in its primary's line.
                assert!(
                    rendered.contains(name),
                    "alias {name} is documented nowhere"
                );
                continue;
            }
            assert!(
                rendered.contains(&format!("server app web {name} ")),
                "{name} has a usage entry that does not name it"
            );
        }
        // And the whole block is non-trivial.
        assert!(rendered.lines().count() > 25, "the usage block collapsed");
    }

    /// A verb this CLI does not have must be refused BY NAME, and refused
    /// before any app-control round trip — the catch-all arm answers straight
    /// from argv, so this is safe to run with no daemon and no GUI.
    ///
    /// It is also the behavioural half of the parity lock: both binaries route
    /// here, so this is the message a typo gets on EITHER of them. The message
    /// it must not be is `unsupported app control command: web`, which is what
    /// yggterm-headless answered for every verb of this plane while it lived in
    /// the GUI binary — an agent reads that as "the build lacks the feature",
    /// not "you typed the verb wrong".
    #[test]
    fn an_unknown_web_verb_is_refused_by_name_without_a_round_trip() {
        let argv: Vec<String> = ["server", "app", "web", "definitely-not-a-verb"]
            .iter()
            .map(|part| (*part).to_string())
            .collect();
        let error = run_app_control_web_cli(&argv, 10)
            .expect_err("an unknown web verb must fail, not silently succeed")
            .to_string();
        assert_eq!(
            error, "unsupported app web action: definitely-not-a-verb",
            "the refusal must name the verb and the plane"
        );

        // And a missing verb is its own message, not a panic.
        let bare: Vec<String> = ["server", "app", "web"]
            .iter()
            .map(|part| (*part).to_string())
            .collect();
        let error = run_app_control_web_cli(&bare, 10)
            .expect_err("`server app web` with no verb must fail")
            .to_string();
        assert_eq!(error, "missing action for server app web");
    }
}
