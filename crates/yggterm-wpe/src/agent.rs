//! The agent verb plane — one JSON object per line, DOM-addressed.
//!
//! Split from the binary on purpose: the dispatch is a pure function of
//! (session table, request) plus the engine, so the whole verb surface is
//! testable without a socket, and the socket server stays thin enough to read
//! in one screen.
//!
//! ## Protocol
//!
//! Request and response are ONE JSON object per line, the daemon idiom.
//! Every response echoes `id` and carries `ok`; a failure carries `error` and
//! never a partial result.
//!
//! ```text
//! -> {"id":"1","verb":"ensure","session":"a","width":800,"height":600}
//! <- {"id":"1","ok":true,"view":0,"created":true}
//! -> {"id":"2","verb":"navigate","session":"a","url":"http://…/page"}
//! <- {"id":"2","ok":true,"title":"…","uri":"http://…/page"}
//! -> {"id":"3","verb":"click","session":"a","selector":"#go"}
//! <- {"id":"3","ok":true,"x":42,"y":17}
//! -> {"id":"4","verb":"type","session":"a","text":"x"}
//! <- {"id":"4","ok":true,"typed":1}
//! -> {"id":"5","verb":"read-back","session":"a","selector":"#out"}
//! <- {"id":"5","ok":true,"text":"done","value":null}
//! -> {"id":"6","verb":"status"}
//! <- {"id":"6","ok":true,"views":[…],"web_processes":[…]}
//! ```
//!
//! ## Two rules the verbs enforce
//!
//! - **Ambiguity is refused, with a count.** A selector matching 0 or 2+ nodes
//!   is an error naming how many it matched, never a silent first-match. This
//!   is the web-do idiom and it exists because "it clicked something" is the
//!   worst possible outcome for an agent.
//! - **Recovery is surfaced, never automatic.** `status` names a terminated
//!   view; only an explicit `restart` brings it back. A verb plane that quietly
//!   re-spawns a crashing view turns a visible fault into an invisible loop.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::json::{Json, obj, parse, s};
use crate::{Engine, Error, Supervisor, ViewId};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const EVAL_TIMEOUT: Duration = Duration::from_secs(15);

/// The agent's session table: a stable session key → the view serving it.
pub struct AgentState<'engine> {
    supervisor: Supervisor<'engine>,
    sessions: BTreeMap<String, ViewId>,
}

impl<'engine> AgentState<'engine> {
    pub fn new(engine: &'engine Engine) -> Self {
        AgentState {
            supervisor: Supervisor::new(engine),
            sessions: BTreeMap::new(),
        }
    }

    pub fn supervisor(&self) -> &Supervisor<'engine> {
        &self.supervisor
    }

    /// Handle one request line, returning one response line.
    ///
    /// Never returns `Err`: a malformed line still deserves a well-formed
    /// answer, or the caller is left waiting on a socket for a reply that will
    /// not come.
    pub fn handle_line(&mut self, line: &str) -> String {
        let request = match parse(line) {
            Ok(value) => value,
            Err(err) => {
                return err_response("", &format!("malformed request line: {err}"));
            }
        };
        let id = request.get("id").and_then(Json::as_str).unwrap_or("").to_string();
        let Some(verb) = request.get("verb").and_then(Json::as_str) else {
            return err_response(&id, "request has no \"verb\"");
        };

        match self.dispatch(verb, &request) {
            Ok(mut fields) => {
                fields.insert("id".to_string(), s(&id));
                fields.insert("ok".to_string(), Json::Bool(true));
                Json::Object(fields).to_string()
            }
            Err(message) => err_response(&id, &message),
        }
    }

    fn dispatch(
        &mut self,
        verb: &str,
        request: &Json,
    ) -> std::result::Result<BTreeMap<String, Json>, String> {
        match verb {
            "ensure" => self.verb_ensure(request),
            "navigate" => self.verb_navigate(request),
            "eval" => self.verb_eval(request),
            "click" => self.verb_click(request),
            "type" => self.verb_type(request),
            "read-back" => self.verb_read_back(request),
            "capture-view" => self.verb_capture(request, false),
            "capture-element" => self.verb_capture(request, true),
            "restart" => self.verb_restart(request),
            "status" => self.verb_status(),
            other => Err(format!(
                "unknown verb {other:?} (expected ensure, navigate, eval, click, read-back, \
                 type, capture-view, capture-element, restart, status)"
            )),
        }
    }

    // ---- helpers ---------------------------------------------------------

    fn session_key(request: &Json) -> std::result::Result<String, String> {
        request
            .get("session")
            .and_then(Json::as_str)
            .map(str::to_string)
            .ok_or_else(|| "this verb needs a \"session\"".to_string())
    }

    fn view_for(&self, request: &Json) -> std::result::Result<ViewId, String> {
        let key = Self::session_key(request)?;
        self.sessions
            .get(&key)
            .copied()
            .ok_or_else(|| format!("no view for session {key:?}; call `ensure` first"))
    }

    fn selector(request: &Json) -> std::result::Result<String, String> {
        request
            .get("selector")
            .and_then(Json::as_str)
            .filter(|sel| !sel.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| "this verb needs a non-empty \"selector\"".to_string())
    }

    fn timeout(request: &Json) -> Duration {
        request
            .get("timeout_ms")
            .and_then(Json::as_u32)
            .map(|ms| Duration::from_millis(u64::from(ms)))
            .unwrap_or(DEFAULT_TIMEOUT)
    }

    /// Resolve a selector to exactly ONE element's viewport rect.
    ///
    /// Refusing 0 and 2+ with the count is the whole point: an agent told "it
    /// clicked" when the selector matched three things has been actively
    /// misled. The script returns a tagged object so the count survives the
    /// round trip instead of collapsing into a null.
    fn resolve_rect(
        &mut self,
        id: ViewId,
        selector: &str,
    ) -> std::result::Result<(f64, f64, f64, f64), String> {
        let script = format!(
            r#"(() => {{
                 const nodes = document.querySelectorAll({sel});
                 if (nodes.length !== 1) return {{ count: nodes.length }};
                 const r = nodes[0].getBoundingClientRect();
                 return {{ count: 1, x: r.x, y: r.y, w: r.width, h: r.height }};
               }})()"#,
            sel = json_string(selector),
        );
        let raw = self
            .supervisor
            .eval(id, &script, EVAL_TIMEOUT)
            .map_err(|e| e.to_string())?;
        let value = parse(&raw).map_err(|e| format!("engine returned unparseable JSON: {e}"))?;
        let count = value
            .get("count")
            .and_then(Json::as_f64)
            .ok_or("the resolver returned no count")? as i64;
        if count != 1 {
            return Err(format!(
                "selector {selector:?} matched {count} elements; refusing to guess which one \
                 (pass a selector that matches exactly one)"
            ));
        }
        let get = |k: &str| value.get(k).and_then(Json::as_f64).unwrap_or(0.0);
        Ok((get("x"), get("y"), get("w"), get("h")))
    }

    // ---- verbs -----------------------------------------------------------

    fn verb_ensure(
        &mut self,
        request: &Json,
    ) -> std::result::Result<BTreeMap<String, Json>, String> {
        let key = Self::session_key(request)?;
        if let Some(existing) = self.sessions.get(&key) {
            return Ok(fields(vec![
                ("view", Json::Number(existing.index() as f64)),
                ("created", Json::Bool(false)),
            ]));
        }
        let width = request.get("width").and_then(Json::as_u32).unwrap_or(1280);
        let height = request.get("height").and_then(Json::as_u32).unwrap_or(720);
        let url = request
            .get("url")
            .and_then(Json::as_str)
            .unwrap_or("about:blank");
        let id = self
            .supervisor
            .open(url, width, height, Self::timeout(request))
            .map_err(|e| e.to_string())?;
        self.sessions.insert(key, id);
        Ok(fields(vec![
            ("view", Json::Number(id.index() as f64)),
            ("created", Json::Bool(true)),
        ]))
    }

    fn verb_navigate(
        &mut self,
        request: &Json,
    ) -> std::result::Result<BTreeMap<String, Json>, String> {
        let id = self.view_for(request)?;
        let url = request
            .get("url")
            .and_then(Json::as_str)
            .ok_or("navigate needs a \"url\"")?
            .to_string();
        self.supervisor
            .view_mut(id)
            .map_err(|e| e.to_string())?
            .load_uri(&url)
            .map_err(|e| e.to_string())?;
        let settled = self.supervisor.pump_until(Self::timeout(request), |sup| {
            sup.view(id).is_ok_and(|v| v.painted_current_document())
        });
        if !settled {
            return Err(format!("{url} did not finish painting before the timeout"));
        }
        let view = self.supervisor.view(id).map_err(|e| e.to_string())?;
        Ok(fields(vec![
            ("title", s(view.title())),
            ("uri", s(view.uri())),
        ]))
    }

    fn verb_eval(&mut self, request: &Json) -> std::result::Result<BTreeMap<String, Json>, String> {
        let id = self.view_for(request)?;
        let script = request
            .get("script")
            .and_then(Json::as_str)
            .ok_or("eval needs a \"script\"")?
            .to_string();
        let raw = self
            .supervisor
            .eval(id, &script, Self::timeout(request))
            .map_err(|e| e.to_string())?;
        // The engine already produced JSON, so hand back the VALUE rather than
        // a string containing JSON — a number stays a number across the wire.
        let value = parse(&raw).unwrap_or_else(|_| s(raw));
        Ok(fields(vec![("result", value)]))
    }

    fn verb_click(&mut self, request: &Json) -> std::result::Result<BTreeMap<String, Json>, String> {
        let id = self.view_for(request)?;
        let selector = Self::selector(request)?;
        let (x, y, w, h) = self.resolve_rect(id, &selector)?;
        if w <= 0.0 || h <= 0.0 {
            return Err(format!(
                "selector {selector:?} matched an element with a zero-sized rect ({w}x{h}); it \
                 is not clickable"
            ));
        }
        let (cx, cy) = ((x + w / 2.0) as i32, (y + h / 2.0) as i32);
        self.supervisor
            .view(id)
            .map_err(|e| e.to_string())?
            .click(cx, cy);
        // Input dispatch is asynchronous into the web process, so a verb that
        // returned immediately would let the caller's very next `read-back`
        // observe the page BEFORE its own click landed — which reads as "the
        // click did nothing". Pump briefly so the event has been delivered and
        // handled by the time this returns.
        self.supervisor
            .pump_until(Duration::from_millis(300), |_| false);
        Ok(fields(vec![
            ("x", Json::Number(f64::from(cx))),
            ("y", Json::Number(f64::from(cy))),
        ]))
    }

    fn verb_type(&mut self, request: &Json) -> std::result::Result<BTreeMap<String, Json>, String> {
        let id = self.view_for(request)?;
        let text = request
            .get("text")
            .and_then(Json::as_str)
            .ok_or("type needs \"text\"")?
            .to_string();
        self.supervisor
            .view(id)
            .map_err(|e| e.to_string())?
            .type_text(&text)
            .map_err(|e| e.to_string())?;
        // Same asynchrony as click.
        self.supervisor
            .pump_until(Duration::from_millis(300), |_| false);
        Ok(fields(vec![("typed", Json::Number(text.chars().count() as f64))]))
    }

    fn verb_read_back(
        &mut self,
        request: &Json,
    ) -> std::result::Result<BTreeMap<String, Json>, String> {
        let id = self.view_for(request)?;
        let selector = Self::selector(request)?;
        let script = format!(
            r#"(() => {{
                 const nodes = document.querySelectorAll({sel});
                 if (nodes.length !== 1) return {{ count: nodes.length }};
                 const el = nodes[0];
                 return {{ count: 1, text: el.textContent, value: ("value" in el ? el.value : null) }};
               }})()"#,
            sel = json_string(&selector),
        );
        let raw = self
            .supervisor
            .eval(id, &script, Self::timeout(request))
            .map_err(|e| e.to_string())?;
        let value = parse(&raw).map_err(|e| format!("engine returned unparseable JSON: {e}"))?;
        let count = value.get("count").and_then(Json::as_f64).unwrap_or(0.0) as i64;
        if count != 1 {
            return Err(format!(
                "selector {selector:?} matched {count} elements; refusing to guess which one"
            ));
        }
        Ok(fields(vec![
            ("text", value.get("text").cloned().unwrap_or(Json::Null)),
            ("value", value.get("value").cloned().unwrap_or(Json::Null)),
        ]))
    }

    fn verb_capture(
        &mut self,
        request: &Json,
        element: bool,
    ) -> std::result::Result<BTreeMap<String, Json>, String> {
        let id = self.view_for(request)?;
        let path = request
            .get("path")
            .and_then(Json::as_str)
            .ok_or("capture needs a \"path\"")?
            .to_string();

        let rect = if element {
            Some(self.resolve_rect(id, &Self::selector(request)?)?)
        } else {
            None
        };

        let view = self.supervisor.view(id).map_err(|e| e.to_string())?;
        let frame = view
            .last_frame()
            .ok_or("this view has not painted, so there is nothing to capture")?;
        let frame = match rect {
            Some((x, y, w, h)) => frame
                .crop(x as i32, y as i32, w as u32, h as u32)
                .ok_or("the element's rect does not overlap the viewport")?,
            None => frame.clone(),
        };
        let png = frame.to_png();
        let bytes = png.len();
        std::fs::write(&path, png).map_err(|e| format!("writing {path}: {e}"))?;
        Ok(fields(vec![
            ("path", s(path)),
            ("width", Json::Number(f64::from(frame.width()))),
            ("height", Json::Number(f64::from(frame.height()))),
            ("bytes", Json::Number(bytes as f64)),
        ]))
    }

    fn verb_restart(
        &mut self,
        request: &Json,
    ) -> std::result::Result<BTreeMap<String, Json>, String> {
        let id = self.view_for(request)?;
        let before = self.supervisor.web_process_of(id);
        self.supervisor
            .restart(id, Self::timeout(request))
            .map_err(|e| e.to_string())?;
        Ok(fields(vec![
            (
                "previous_web_process",
                before.map(|p| Json::Number(f64::from(p))).unwrap_or(Json::Null),
            ),
            (
                "web_process",
                self.supervisor
                    .web_process_of(id)
                    .map(|p| Json::Number(f64::from(p)))
                    .unwrap_or(Json::Null),
            ),
        ]))
    }

    fn verb_status(&mut self) -> std::result::Result<BTreeMap<String, Json>, String> {
        // Pump first: a web process that died a moment ago has a signal waiting
        // in the main context, and a status that has not drained it would call
        // a dead view healthy.
        self.supervisor.pump_until(Duration::from_millis(50), |_| false);

        let mut views = Vec::new();
        for (key, id) in &self.sessions {
            let Ok(view) = self.supervisor.view(*id) else {
                continue;
            };
            views.push(obj(vec![
                ("session", s(key)),
                ("view", Json::Number(id.index() as f64)),
                ("uri", s(view.uri())),
                ("title", s(view.title())),
                ("painted", Json::Bool(view.painted_current_document())),
                ("frames", Json::Number(f64::from(view.frames_exported()))),
                (
                    "blank_frames_skipped",
                    Json::Number(f64::from(view.blank_frames_skipped())),
                ),
                // Named honestly, and never acted on here.
                (
                    "web_process_terminated",
                    Json::Bool(view.web_process_terminated()),
                ),
                (
                    "web_process",
                    self.supervisor
                        .web_process_of(*id)
                        .map(|p| Json::Number(f64::from(p)))
                        .unwrap_or(Json::Null),
                ),
            ]));
        }
        let processes = self
            .supervisor
            .web_processes()
            .into_iter()
            .map(|p| {
                obj(vec![
                    ("pid", Json::Number(f64::from(p.pid))),
                    ("comm", s(p.comm)),
                ])
            })
            .collect();
        Ok(fields(vec![
            ("views", Json::Array(views)),
            ("web_processes", Json::Array(processes)),
        ]))
    }
}

fn fields(pairs: Vec<(&str, Json)>) -> BTreeMap<String, Json> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn err_response(id: &str, message: &str) -> String {
    obj(vec![
        ("id", s(id)),
        ("ok", Json::Bool(false)),
        ("error", s(message)),
    ])
    .to_string()
}

/// A selector interpolated into a script must be a JSON string literal, or a
/// quote in the selector ends the literal and the rest becomes code.
fn json_string(value: &str) -> String {
    s(value).to_string()
}

/// Map a crate error to the wire's message form.
impl From<Error> for String {
    fn from(err: Error) -> String {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interpolating a selector into a script is an injection site. A selector
    /// carrying a quote must stay DATA.
    #[test]
    fn selectors_are_interpolated_as_json_literals() {
        let nasty = r#"a"]; window.pwned = 1; //"#;
        let encoded = json_string(nasty);
        assert!(encoded.starts_with('"') && encoded.ends_with('"'));
        assert!(
            encoded.contains("\\\""),
            "the embedded quote must be escaped or it terminates the literal and the rest of \
             the selector becomes executable code: {encoded}",
        );
        assert_eq!(
            parse(&encoded).expect("valid JSON").as_str(),
            Some(nasty),
            "and it must round-trip to exactly the selector the caller asked for",
        );
    }

    #[test]
    fn a_malformed_line_still_gets_a_well_formed_answer() {
        // Not `Err` and not silence: a caller waiting on a socket for a reply
        // that never comes is the worst failure mode of a line protocol.
        for bad in ["", "{", "not json", "[1,2]"] {
            let engine_free = err_response("", "x");
            assert!(parse(&engine_free).is_ok());
            let parsed = parse(bad);
            assert!(parsed.is_err() || parsed.unwrap().get("verb").is_none());
        }
    }

    #[test]
    fn an_error_response_is_valid_json_and_carries_the_id() {
        let line = err_response("42", "something \"quoted\" went wrong\n");
        let value = parse(&line).expect("error responses must be parseable");
        assert_eq!(value.get("id").and_then(Json::as_str), Some("42"));
        assert_eq!(value.get("ok").and_then(Json::as_bool), Some(false));
        assert!(
            value
                .get("error")
                .and_then(Json::as_str)
                .is_some_and(|e| e.contains("quoted")),
        );
        assert!(!line.contains('\n'), "a response must be ONE line");
    }
}
