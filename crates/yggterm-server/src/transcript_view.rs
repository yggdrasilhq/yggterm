//! The RENDERED TRANSCRIPT view: yggterm's loopback server for the vendored
//! T3 Code timeline (`third_party/t3code-timeline`).
//!
//! A web surface cannot be pointed at `file://` — that is an http(s)-only trust
//! boundary in the surface layer — so the bundle and its data are served from
//! `127.0.0.1` instead, which is the same shape libyggterm apps already use for
//! their control endpoints (`TcpListener`, one tiny request shape, no
//! framework; the ychrome pattern yedit also follows).
//!
//! The split of responsibility is deliberate: **the page is dumb and Rust is
//! authoritative.** The browser fetches `/messages?session=…` and renders what
//! it is given; it never parses a transcript, never reaches the filesystem, and
//! never learns what a CLI is. That keeps one owner for "what did this session
//! say" and means a new agent CLI is picked up here for free once its reader
//! exists.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

/// Where the built bundle lives, in resolution order.
///
/// The assets are a build product of an npm package, not of cargo, so the
/// binary cannot embed them without making `cargo build` depend on `npm run
/// build`. Resolving at runtime keeps the two builds independent and gives a
/// developer checkout a working default.
pub fn resolve_asset_root() -> Option<PathBuf> {
    // 1. Explicit override — packaging and tests.
    if let Some(raw) = std::env::var_os("YGGTERM_TRANSCRIPT_ASSETS") {
        let path = PathBuf::from(raw);
        if path.join("transcript.js").is_file() {
            return Some(path);
        }
    }
    // 2. Beside the installed binary, where a package would place them.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join("transcript-assets");
        if candidate.join("transcript.js").is_file() {
            return Some(candidate);
        }
    }
    // 3. A developer checkout: the npm package's own dist.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("third_party/t3code-timeline/dist"));
    repo.filter(|path| path.join("transcript.js").is_file())
}

/// The page. Deliberately tiny: it fetches its own data, so the only thing the
/// server has to template in is the session it is showing.
///
/// `theme` is passed through rather than detected, because yggterm owns the
/// theme (DESIGN.md) and the vendored renderer must not consult the OS.
fn index_html(session_path: &str, theme: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Transcript</title>
<link rel="stylesheet" href="/t3code-timeline.css">
<style>html,body{{height:100%;margin:0}}#root{{height:100vh}}</style>
</head>
<body>
<div id="root"></div>
<script src="/transcript.js"></script>
<script>
(function () {{
  var session = {session};
  var root = document.getElementById("root");
  function load() {{
    fetch("/messages?session=" + encodeURIComponent(session))
      .then(function (response) {{ return response.json(); }})
      .then(function (payload) {{
        window.yggtermTranscript.mount(root, {{
          messages: payload.messages || [],
          theme: {theme},
          cwd: payload.cwd || undefined,
          working: !!payload.working,
        }});
      }})
      .catch(function (error) {{ console.error("transcript load failed", error); }});
  }}
  load();
  // A transcript grows while the user watches it. Re-mounting with fresh
  // messages is idempotent (see mount()), so this updates in place and keeps
  // scroll position rather than rebuilding the list.
  setInterval(load, 2000);
}})();
</script>
</body>
</html>
"#,
        session = serde_json::to_string(session_path).unwrap_or_else(|_| "\"\"".into()),
        theme = serde_json::to_string(theme).unwrap_or_else(|_| "\"dark\"".into()),
    )
}

/// Resolve a session path to the transcript file on THIS machine.
///
/// Returns `Ok(None)` for a session whose transcript is not local — a remote
/// agent's JSONL lives on its own host and reaching it is the remote-scan
/// pathway's job, not this server's. That is a "not yet", not an error, and the
/// caller renders an empty transcript rather than a failure.
pub fn local_transcript_path_for_session(session_path: &str) -> Result<Option<PathBuf>> {
    // A row built by the cwd-tree scanner IS the store file. The agent-CLI
    // registry says which CLI owns it, so this needs no per-CLI branch.
    if yggterm_core::agent_cli_for_store_session_file(session_path).is_some() {
        let path = PathBuf::from(session_path);
        return Ok(path.is_file().then_some(path));
    }
    // A live LOCAL Claude Code row is keyed by the CLI's own uuid.
    if let Some(uuid) = session_path.strip_prefix("local://") {
        return Ok(yggterm_core::local_cc_session_jsonl_path(uuid));
    }
    // …and so is a live RUNTIME KEY (`cc-runtime://<uuid>`), which is the shape
    // every agent row this daemon actually owns has: `local://` carries the
    // yggterm RUNTIME id, not the CLI's, so the branch above resolves nothing
    // for them. Measured on `dev` 2026-08-10 — every `local://` key its nine
    // daemons owned had no CC transcript, while the rows themselves were
    // `cc-runtime://<cc-session-id>`.
    //
    // ⛔ Dispatched through the scheme REGISTRY rather than by hand-writing
    // another prefix: `agent_scheme.rs` exists because a hand-listed set of
    // schemes is what rots, and its own docs name a sanitizer that was missing
    // `cc-runtime://` as the standing exhibit.
    if let Some(id) = id_addressable_claude_code_session(session_path) {
        return Ok(yggterm_core::local_cc_session_jsonl_path(id));
    }
    if let Some(id) = id_addressable_antigravity_session(session_path) {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let descriptor = yggterm_core::agent_cli_descriptor(yggterm_core::SessionKind::Antigravity);
            if let Some(desc) = descriptor {
                for root in desc.store_roots_absolute(&home) {
                    let path = root.join(format!("{id}.db"));
                    if path.is_file() {
                        return Ok(Some(path));
                    }
                }
            }
        }
    }
    Ok(None)
}

/// The Claude Code session id inside a runtime key, when the path is one.
///
/// Pure and registry-driven, so the one judgement here — *"is this string a
/// LOCAL runtime key whose id names a CC transcript"* — is testable without a
/// home directory, and cannot drift from `agent_scheme.rs`.
///
/// ⛔ Only Claude Code is id-addressable: it stores a session as `<id>.jsonl`.
/// A codex rollout is named by TIMESTAMP under a dated directory, so finding one
/// by id means walking the store — different work, and not what this caller
/// needs (codex has no sub-agent plane, which is what the gate reads this for).
pub fn id_addressable_claude_code_session(session_path: &str) -> Option<&str> {
    let (prefix, id) = session_path.split_once("://")?;
    let descriptor = yggterm_core::agent_scheme::scheme_for_prefix(&format!("{prefix}://"))?;
    if descriptor.role != yggterm_core::agent_scheme::SchemeRole::RuntimeKey
        || descriptor.kind != Some(yggterm_core::SessionKind::ClaudeCode)
        // A remote row (`remote-cc://<machine>/<id>`) keeps its transcript on the
        // FAR host. The `/` is what separates the two shapes, and treating a
        // remote id as local would glob for a file that is not this machine's.
        || id.contains('/')
        || id.is_empty()
    {
        return None;
    }
    Some(id)
}

/// The Antigravity session id inside a runtime key (`agy-runtime://<id>`).
pub fn id_addressable_antigravity_session(session_path: &str) -> Option<&str> {
    let (prefix, id) = session_path.split_once("://")?;
    let descriptor = yggterm_core::agent_scheme::scheme_for_prefix(&format!("{prefix}://"))?;
    if descriptor.role != yggterm_core::agent_scheme::SchemeRole::RuntimeKey
        || descriptor.kind != Some(yggterm_core::SessionKind::Antigravity)
        || id.contains('/')
        || id.is_empty()
    {
        return None;
    }
    Some(id)
}

/// The messages payload for `session_path`, as the page consumes it.
pub fn messages_payload(session_path: &str) -> Result<serde_json::Value> {
    let Some(path) = local_transcript_path_for_session(session_path)? else {
        return Ok(serde_json::json!({ "messages": [], "cwd": null, "working": false }));
    };
    // ONE owner of "which CLI wrote this file" — the reader's own registry
    // dispatch. This used to re-answer it here, which is a second encoding of a
    // decision that has exactly one right answer.
    let messages = yggterm_core::read_agent_transcript_messages(&path).unwrap_or_default();
    let session_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("session");
    let cwd = yggterm_core::read_cc_session_identity_fields(&path)
        .ok()
        .flatten()
        .map(|(_, cwd)| cwd)
        .or_else(|| yggterm_core::local_antigravity_session_cwd(session_id));
    Ok(serde_json::json!({
        "messages": yggterm_core::transcript_view_messages(session_id, &messages),
        "cwd": cwd,
        "working": false,
    }))
}

/// A parsed request line — method and target only, which is all this server
/// routes on.
fn read_request_target(stream: &TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let _method = parts.next()?;
    Some(parts.next()?.to_string())
}

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// `?session=…` from a request target, percent-decoded.
fn session_query(target: &str) -> Option<String> {
    let (_, query) = target.split_once('?')?;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("session=") {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn serve_connection(mut stream: TcpStream, assets: &Path, theme: &str) {
    let Some(target) = read_request_target(&stream) else {
        return;
    };
    let path = target.split('?').next().unwrap_or("/");
    match path {
        "/" | "/index.html" => {
            let session = session_query(&target).unwrap_or_default();
            write_response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                index_html(&session, theme).as_bytes(),
            );
        }
        "/messages" => {
            let session = session_query(&target).unwrap_or_default();
            let payload = messages_payload(&session).unwrap_or_else(|error| {
                serde_json::json!({ "messages": [], "error": error.to_string() })
            });
            write_response(
                &mut stream,
                "200 OK",
                "application/json",
                payload.to_string().as_bytes(),
            );
        }
        // Only the two build products are servable. Everything else is 404 —
        // this server sits on loopback beside the user's files, so it must not
        // become a way to read arbitrary paths.
        "/transcript.js" | "/t3code-timeline.css" => {
            let name = path.trim_start_matches('/');
            let content_type = if name.ends_with(".css") {
                "text/css; charset=utf-8"
            } else {
                "text/javascript; charset=utf-8"
            };
            match std::fs::read(assets.join(name)) {
                Ok(body) => write_response(&mut stream, "200 OK", content_type, &body),
                Err(_) => write_response(&mut stream, "404 Not Found", "text/plain", b"missing"),
            }
        }
        _ => write_response(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Start the transcript server on an ephemeral loopback port; returns its base
/// URL. The thread is detached and lives as long as the process — the surface
/// pointing at it can be created and destroyed many times.
pub fn spawn(theme: &str) -> Result<String> {
    let assets = resolve_asset_root().ok_or_else(|| {
        anyhow!(
            "the transcript view bundle is not built — run `npm install && npm run build` in \
             third_party/t3code-timeline, or set YGGTERM_TRANSCRIPT_ASSETS"
        )
    })?;
    let listener =
        TcpListener::bind("127.0.0.1:0").context("binding the transcript view server")?;
    let port = listener.local_addr()?.port();
    let theme = theme.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let assets = assets.clone();
            let theme = theme.clone();
            std::thread::spawn(move || serve_connection(stream, &assets, &theme));
        }
    });
    Ok(format!("http://127.0.0.1:{port}"))
}

/// Drain a connection's body before closing, so a client that sent one does not
/// see a reset instead of the response.
#[allow(dead_code)]
fn drain(stream: &mut TcpStream) {
    let mut sink = Vec::new();
    let _ = stream.take(0).read_to_end(&mut sink);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_query_is_extracted_and_percent_decoded() {
        assert_eq!(
            session_query("/messages?session=remote-cc%3A%2F%2Fdev%2Fabc"),
            Some("remote-cc://dev/abc".to_string())
        );
        assert_eq!(session_query("/messages"), None);
        assert_eq!(
            session_query("/?session=local%3A%2F%2Fx&other=1"),
            Some("local://x".to_string())
        );
    }

    // The page is templated with JSON-encoded values, so a session path
    // containing a quote cannot break out into the script.
    #[test]
    fn the_page_json_encodes_what_it_templates_in() {
        let html = index_html("local://\"; alert(1); //", "dark");
        // The quote must arrive ESCAPED, so the JS string literal still closes
        // where the template intended. Asserting the whole emitted line is the
        // only check that cannot pass on a coincidence: the escaped form does
        // legitimately contain the substring `"; alert(1)`, so searching for
        // that proves nothing either way.
        assert!(
            html.contains(r#"var session = "local://\"; alert(1); //";"#),
            "session must be JSON-encoded into the script: {html}"
        );
    }

    #[test]
    fn a_session_with_no_local_transcript_yields_an_empty_payload_not_an_error() {
        let payload = messages_payload("remote-cc://dev/not-here").unwrap();
        assert_eq!(payload["messages"].as_array().unwrap().len(), 0);
        assert!(payload.get("error").is_none());
    }

    // The shape every agent row this daemon OWNS actually has. Measured on `dev`
    // 2026-08-10: the terminal map is keyed `local://<yggterm-runtime-id>` and
    // the row is `cc-runtime://<cc-session-id>` — so a resolver that knew only
    // the first two branches returned `None` for every live CC session on the
    // machine while looking entirely correct.
    #[test]
    fn a_live_runtime_key_names_the_claude_code_session_behind_it() {
        assert_eq!(
            id_addressable_claude_code_session(
                "cc-runtime://2765f233-53ae-49f6-bb19-a156ea81d677"
            ),
            Some("2765f233-53ae-49f6-bb19-a156ea81d677")
        );
    }

    // Each of these would send the reader looking for a file that is not on this
    // machine, or not a transcript at all.
    #[test]
    fn nothing_else_is_id_addressable_on_this_machine() {
        for path in [
            // A remote row's transcript lives on the far host.
            "remote-cc://dev/2765f233-53ae-49f6-bb19-a156ea81d677",
            // Codex names its rollout by timestamp, not by id.
            "codex-runtime://2765f233-53ae-49f6-bb19-a156ea81d677",
            // A row identity, not a runtime key.
            "session://2765f233-53ae-49f6-bb19-a156ea81d677",
            // Not a scheme this project registers.
            "postgres://2765f233-53ae-49f6-bb19-a156ea81d677",
            "cc-runtime://",
            "no-scheme-at-all",
        ] {
            assert_eq!(
                id_addressable_claude_code_session(path),
                None,
                "{path} must not be treated as a local CC session id"
            );
        }
    }

    // Only the two build products are servable; this server sits beside the
    // user's files on loopback.
    #[test]
    fn asset_routing_refuses_anything_but_the_two_build_products() {
        for target in ["/../../etc/passwd", "/index.js", "/messages/../transcript.js"] {
            let path = target.split('?').next().unwrap();
            assert!(
                !matches!(path, "/transcript.js" | "/t3code-timeline.css"),
                "{target} must not resolve to an asset"
            );
        }
    }
}
