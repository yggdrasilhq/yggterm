//! OpenCode v2's service plane — the typed edge yggterm integrates with.
//!
//! opencode2 is CLIENT-SERVER: the TUI is a client; a per-user background
//! service OWNS the sessions. Presence ("which tabs are open"), per-session
//! metadata (title, directory, tokens, idle/viewed times) and per-session
//! delivery live on the SERVICE, not in the row's PTY — one PTY hosts N tabs
//! (docs/cli-integration.md, Issue Heading 26). The row's PTY is a view;
//! THIS plane is the truth about the sessions.
//!
//! Discovery + auth mirror the CLI's own (`packages/cli/src/services/
//! daemon.ts` in the opencode repo): a registration file in the state dir
//! carries the URL and the shared password, and requests authenticate with
//! HTTP Basic (`opencode` : password). The installed preview writes
//! `service.json`; older spellings are probed as fallbacks.

use serde_json::Value;
use std::path::PathBuf;

/// One session the service's store lists, with the working set marked on it.
/// The list is the tab mirror's universe; `running` is a per-session status
/// (a turn in flight), not a membership filter — [11.6.3-a].
#[derive(Debug, Clone, PartialEq)]
pub struct OpencodeServiceSession {
    /// The CLI's own id (`ses_…`) — the id a cold resume spells after
    /// `--session`, and the identity tab rows carry.
    pub id: String,
    pub title: Option<String>,
    pub directory: Option<String>,
    pub updated_epoch_ms: u128,
    /// When the service last recorded a view. ⛔ API-WRITER-ONLY on the
    /// installed beta-19271 — no TUI flow writes it (measured, falsifier (a)),
    /// so this is a dormant signal: viewing truth is the OSC window title.
    /// `0` = the service did not say.
    pub viewed_epoch_ms: u128,
    /// In the service's working set = a turn is in flight on this session
    /// (a STATUS on the list — never a membership filter; [11.6.3-a]).
    pub running: bool,
}

/// The registration the service published: where it listens and the one
/// private credential discovered clients present.
#[derive(Debug, Clone)]
pub struct OpencodeServiceRegistration {
    pub url: String,
    pub password: String,
}

/// Candidate registration files, most-authoritative first. The v2 preview
/// installs under the `beta` dist-tag, which namespaces some state under
/// `beta/` while the registration stayed at the top level — both spellings
/// are probed rather than assumed.
fn registration_candidates(home: &PathBuf) -> Vec<PathBuf> {
    let base = home.join(".local/state/opencode");
    [
        base.join("service.json"),
        base.join("server.json"),
        base.join("beta/service.json"),
        base.join("beta/server.json"),
    ]
    .into_iter()
    .collect()
}

/// Read the service registration from the CLI's own state dir. `None` = the
/// service has never run here / no readable registration — the plane is
/// simply absent, never an error.
pub fn service_registration(home: &PathBuf) -> Option<OpencodeServiceRegistration> {
    for path in registration_candidates(home) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let url = value.get("url")?.as_str()?.to_string();
        if !url.starts_with("http") {
            continue;
        }
        let password = value
            .get("password")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        return Some(OpencodeServiceRegistration { url, password });
    }
    None
}

fn service_get(registration: &OpencodeServiceRegistration, path: &str) -> Option<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;
    let value: Value = client
        .get(format!("{}{}", registration.url, path))
        .basic_auth("opencode", Some(registration.password.clone()))
        .send()
        .ok()?
        .json()
        .ok()?;
    Some(value)
}

/// POST with a JSON body — the per-session delivery/focus verbs. `false` =
/// the service refused or could not be reached; a soft failure by contract,
/// never a reason to block a restore.
fn service_post(registration: &OpencodeServiceRegistration, path: &str, body: &Value) -> bool {
    let Some(client) = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()
    else {
        return false;
    };
    client
        .post(format!("{}{}", registration.url, path))
        .basic_auth("opencode", Some(registration.password.clone()))
        .json(body)
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// The service's session list with the working set marked — the tab mirror's
/// universe and its per-session status in one read.
///
/// ⛔ MEASURED TRUTH (the 2026-09-10 decode, installed beta-19271): the
/// service's `/api/session/active` is the WORKING set — the run-coordinator's
/// map of in-flight executions — never "the open tabs". A session enters it
/// ≤0.1 s after a prompt and leaves when the turn settles; an idle fleet
/// answers `{"data":{}}` BY DESIGN. This function used to join on that set
/// and call the result "the open tabs", so the tab mirror starved (rows only
/// existed while a turn ran) and the viewing signal emptied with it —
/// defect [11.6.3-a]. Now the UNIVERSE is the store list (`/api/session`,
/// `session_v2` — the service's own db) and the working set is a per-session
/// STATUS on it (`running`). Viewing is deliberately NOT answered here: the
/// viewed times are API-writer-only on this build (no TUI flow writes them —
/// measured), so viewing truth lives on the OSC window title, which the
/// mirror binds.
pub fn service_sessions(home: &PathBuf) -> Option<Vec<OpencodeServiceSession>> {
    let registration = service_registration(home)?;
    let listed = service_get(&registration, "/api/session")?;
    let sessions = join_working_set(decode_session_list(&listed), service_get(&registration, "/api/session/active").as_ref());
    Some(sessions)
}

/// Mark each listed session `running` when the working set holds it — the
/// status a phase read consumes — and keep a working id whose store detail
/// has not landed yet (a session created between the two reads): presence
/// outranks metadata. The result is ordered by store recency, which on this
/// build IS turn recency (`time.updated`; the viewed column is dead and must
/// order nothing).
fn join_working_set(
    mut sessions: Vec<OpencodeServiceSession>,
    active: Option<&Value>,
) -> Vec<OpencodeServiceSession> {
    let working: Vec<String> = active
        .and_then(|a| a.get("data"))
        .and_then(|d| d.as_object())
        .map(|map| {
            map.keys()
                .filter(|id| !id.trim().is_empty())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for ses in &mut sessions {
        ses.running = working.contains(&ses.id);
    }
    for id in &working {
        if !sessions.iter().any(|s| s.id == *id) {
            sessions.push(OpencodeServiceSession {
                id: id.clone(),
                title: None,
                directory: None,
                updated_epoch_ms: 0,
                viewed_epoch_ms: 0,
                running: true,
            });
        }
    }
    sessions.sort_by(|a, b| b.updated_epoch_ms.cmp(&a.updated_epoch_ms));
    sessions
}

fn decode_session_list(value: &Value) -> Vec<OpencodeServiceSession> {
    let empty = Vec::new();
    let array = value
        .as_array()
        .or_else(|| value.get("data").and_then(|d| d.as_array()))
        .or_else(|| value.get("sessions").and_then(|s| s.as_array()))
        .unwrap_or(&empty);
    array
        .iter()
        .filter_map(|s| {
            let id = s.get("id")?.as_str()?.to_string();
            let time = s.get("time");
            Some(OpencodeServiceSession {
                id,
                title: s.get("title").and_then(|t| t.as_str()).map(str::to_string),
                directory: s
                    .get("location")
                    .and_then(|l| l.get("directory"))
                    .and_then(|d| d.as_str())
                    .map(str::to_string),
                updated_epoch_ms: time
                    .and_then(|t| t.get("updated"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u128,
                viewed_epoch_ms: time
                    .and_then(|t| t.get("viewed"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u128,
                running: false,
            })
        })
        .collect()
}

/// Mark a session viewed — the service-side half of "focus this tab". The
/// TUI's own focus path reports the same thing (`time.viewed`), so a row
/// click and a human's tab switch converge on one signal. Best-effort: some
/// builds shape this route differently, and a failure costs nothing.
///
/// [11.6.3-b] FIXED IN CODE: the route REQUIRES `{"idle": <int>}` and
/// persists the value VERBATIM into `session_v2.time_viewed` (measured on
/// beta-19271: idle:0 → stored 0, idle:5000 → stored 5000; the TUI sends
/// epoch-ms). The old `{}` body was a guaranteed 400 — this verb could never
/// work on the installed build. "Viewed" here means now, so the body carries
/// the current epoch-ms.
pub fn view_session(home: &PathBuf, session_id: &str) -> bool {
    let Some(registration) = service_registration(home) else {
        return false;
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    service_post(
        &registration,
        &format!("/api/session/{session_id}/view"),
        &serde_json::json!({ "idle": now_ms }),
    )
}

/// Deliver a message to ONE session — per-session addressing for the fleet
/// verbs. The service queues it in the session's inbox (steer/queue), which
/// is the same contract yggterm's row-plane submit documents for a busy row.
pub fn send_prompt(home: &PathBuf, session_id: &str, text: &str) -> bool {
    let Some(registration) = service_registration(home) else {
        return false;
    };
    // ⛔ Body shape is BUILD-specific: the installed beta-18684 wants
    // `{"text": …}` at the TOP level (verified live 2026-08-29 — the nested
    // `{"prompt": {…}}` from the repo's openapi revision 400s with
    // `Missing key at ["text"]`). 200 returns the created user message.
    service_post(
        &registration,
        &format!("/api/session/{session_id}/prompt"),
        &serde_json::json!({ "text": text }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_is_read_from_the_state_dir_and_auth_carries_the_password() {
        let home = std::env::temp_dir().join(format!("ygg-oc-svc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let state = home.join(".local/state/opencode");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("service.json"),
            r#"{"id":"3794906e-9b72-4569-9d50-a5e436cd7a44","version":"0.0.0-beta-18684",
               "url":"http://127.0.0.1:49374","pid":303299,
               "password":"shared-secret-1"}"#,
        )
        .unwrap();
        let reg = service_registration(&home).expect("registration readable");
        assert_eq!(reg.url, "http://127.0.0.1:49374");
        assert_eq!(reg.password, "shared-secret-1");
        // The join under the measured truth: the LIST is the universe, the
        // working set is a status, and order is store recency (updated), not
        // the dead viewed column.
        let active = serde_json::json!({
            "data": {
                "ses_a0000000000000000000000001": {"type": "running"},
                "ses_b0000000000000000000000002": {"type": "running"}
            }
        });
        let listed = serde_json::json!([
            {"id": "ses_a0000000000000000000000001", "title": "older view",
             "location": {"directory": "/home/user/proj"},
             "time": {"updated": 1000, "viewed": 1000}},
            {"id": "ses_b0000000000000000000000002", "title": "focused now",
             "location": {"directory": "/home/user/proj"},
             "time": {"updated": 2000, "viewed": 9000}},
            {"id": "ses_c0000000000000000000000003", "title": "quiet in the store",
             "location": {"directory": "/home/user/proj"},
             "time": {"updated": 3000, "viewed": 3000}}
        ]);
        let out = join_working_set(decode_session_list(&listed), Some(&active));
        // The idle-in-the-store session SURVIVES the join — the old filter
        // dropped it, which is exactly how the mirror starved ([11.6.3-a]).
        assert_eq!(out.len(), 3, "the store list is the universe, working or not");
        assert!(out.iter().find(|s| s.id == "ses_c0000000000000000000000003").unwrap().running == false,
            "listed but not working = an idle session, not an absent one");
        assert!(out.iter().find(|s| s.id == "ses_a0000000000000000000000001").unwrap().running);
        // Ordering is turn recency: ses_c (updated 3000) first even though
        // its viewed time is the oldest — viewed orders nothing.
        assert_eq!(out[0].id, "ses_c0000000000000000000000003", "store recency orders");
        assert_eq!(out[0].directory.as_deref(), Some("/home/user/proj"));
        // A working id the list has not caught up with still shows (presence
        // outranks metadata), marked running.
        let race = serde_json::json!({"data": {"ses_new000000000000000000009": {"type": "running"}}});
        let out = join_working_set(decode_session_list(&listed), Some(&race));
        assert_eq!(out.len(), 4);
        assert!(out.iter().find(|s| s.id == "ses_new000000000000000000009").unwrap().running);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// [11.6.3-a]'s red case: an IDLE fleet (`{"data":{}}`) must not empty the
    /// answer — the old early-return turned "nothing is running" into "no
    /// tabs exist" and the mirror retired its rows every quiet tick.
    #[test]
    fn an_idle_working_set_leaves_the_store_list_whole() {
        let idle = serde_json::json!({"data": {}});
        let listed = serde_json::json!([
            {"id": "ses_a0000000000000000000000001", "title": "one",
             "time": {"updated": 1000, "viewed": 0}},
            {"id": "ses_b0000000000000000000000002", "title": "two",
             "time": {"updated": 2000, "viewed": 0}}
        ]);
        let out = join_working_set(decode_session_list(&listed), Some(&idle));
        assert_eq!(out.len(), 2, "idle is a status, not an absence");
        assert!(out.iter().all(|s| !s.running));
        // A missing active answer (fetch refused) degrades to unmarked, not
        // to None — the universe is the list.
        let out = join_working_set(decode_session_list(&listed), None);
        assert_eq!(out.len(), 2);
    }
}

#[test]
#[ignore] // hits the LIVE service on this host — run explicitly: cargo test -p yggterm-core --lib -- --ignored opencode_live
fn opencode_live_service_fetch() {
    let home = std::env::var("HOME").expect("HOME is set for a live-service probe");
    let reg = service_registration(&std::path::PathBuf::from(&home));
    println!("registration: {:?}", reg.as_ref().map(|r| r.url.clone()));
    assert!(reg.is_some(), "no service registration — is opencode2 running?");
    let sessions = service_sessions(&std::path::PathBuf::from(&home));
    match sessions {
        Some(list) => {
            println!("service sessions: {}", list.len());
            for s in &list {
                println!("  {} | {:?} | dir {:?}", &s.id[..s.id.len().min(24)], s.title.as_deref().unwrap_or(""), s.directory.as_deref().unwrap_or(""));
            }
            assert!(!list.is_empty(), "service reachable but the store lists zero sessions");
        }
        None => panic!("service_sessions returned None — fetch failed inside the client (this is the daemon's exact code path)"),
    }
}

/// The TUI's own client-side tab state — `.local/state/opencode/latest/tui/
/// tabs.json` — as `cwd → [sessionID]`, the window/cwd keys flattened.
///
/// ⛔ MEASURED 2026-09-17 (2.0.3, the muse lab host, [11.134] residual work):
/// this client state file is the ONE surface that names which session a
/// window BINDS without OSC parsing — the 2026-09-10 decode found no SERVER
/// surface for it (still true), and the OSC route title barely paints (three
/// drives captured only the generic `OpenCode`). Measured shape:
/// `{"global":{"tabs":[],…},"cwd":{"/abs/cwd":{"tabs":[{"sessionID":"ses_…",
/// "title":"…"}],"unread":{}}}}`. The entry lands within the launch's first
/// seconds, persists after the TUI exits, and a `--session <id>` resume from
/// a DIFFERENT project's cwd writes the pinned id under THAT cwd (measured —
/// a vouch-passing cross-project resume binds correctly, which falsifies the
/// mis-bind class [11.134]'s residual feared). The `latest/` dir is
/// last-writer-wins across TUIs; a bind verdict that must survive concurrent
/// windows should read every sibling instance dir, not just `latest`.
/// `None` = absent or unparseable — the plane is simply not there, never an
/// error.
fn parse_tui_tabs(text: &str) -> Option<std::collections::BTreeMap<String, Vec<String>>> {
    let value: Value = serde_json::from_str(text).ok()?;
    let cwd = value.get("cwd")?.as_object()?;
    let mut map = std::collections::BTreeMap::new();
    for (directory, entry) in cwd {
        let Some(tabs) = entry.get("tabs").and_then(|t| t.as_array()) else {
            continue;
        };
        let ids: Vec<String> = tabs
            .iter()
            .filter_map(|t| t.get("sessionID").and_then(|s| s.as_str()))
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .collect();
        if !ids.is_empty() {
            map.insert(directory.clone(), ids);
        }
    }
    Some(map)
}

pub fn tui_tabs(home: &PathBuf) -> Option<std::collections::BTreeMap<String, Vec<String>>> {
    parse_tui_tabs(
        &std::fs::read_to_string(home.join(".local/state/opencode/latest/tui/tabs.json")).ok()?,
    )
}

/// Every instance dir's tab map — `.local/state/opencode/*/tui/tabs.json` —
/// sorted by instance name. `latest/` is last-writer-wins across concurrent
/// TUIs, so a bind verdict that must survive several windows reads every
/// sibling, not just the newest pointer (measured on the muse lab host
/// 2026-09-18: `beta/tui` held the only tabs.json while `latest/tui` sat
/// empty — a latest-only read answers from the wrong generation there). A
/// malformed or unreadable instance is skipped, never an error; an empty
/// Vec = the plane is not there at all.
pub fn tui_tabs_across_instances(
    home: &PathBuf,
) -> Vec<(String, std::collections::BTreeMap<String, Vec<String>>)> {
    let root = home.join(".local/state/opencode");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("tui/tabs.json").is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let text =
                std::fs::read_to_string(root.join(&name).join("tui/tabs.json")).ok()?;
            parse_tui_tabs(&text).map(|map| (name, map))
        })
        .collect()
}

/// The service session id embedded in a mirror tab row's path, if it is one.
///
/// Mirror tab rows are keyed `opencode-runtime://<ses_id>` — the id is the
/// SERVICE's own (`ses_…`), which is what per-session delivery needs. The
/// anchor row is uuid-keyed and never matches; no other scheme matches.
pub fn tab_session_id(session_path: &str) -> Option<&str> {
    let rest = session_path.strip_prefix("opencode-runtime://")?;
    rest.starts_with("ses_").then_some(rest)
}

#[cfg(test)]
mod tui_tabs_tests {
    use super::{tui_tabs, tui_tabs_across_instances};

    /// The measured 2.0.3 file, verbatim shape (the muse lab host,
    /// /tmp/opencode-suite-home-8ynxD7, 2026-09-17): cwd-keyed tabs with
    /// sessionID each, a global bucket the reader must skip.
    #[test]
    fn the_tui_tabs_reader_reads_the_measured_client_state() {
        let home = std::env::temp_dir().join(format!("ygg-oc-tabs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".local/state/opencode/latest/tui")).unwrap();
        std::fs::write(
            home.join(".local/state/opencode/latest/tui/tabs.json"),
            r#"{"global":{"tabs":[],"unread":{}},"cwd":{"/tmp/oc-probe-ws3":{"tabs":[{"sessionID":"ses_f546febb5ffeFdsMXaTv5cRZ6X","title":"New session"}],"unread":{}},"/tmp/bindB":{"tabs":[{"sessionID":"ses_f546febb5ffeFdsMXaTv5cRZ6X","title":"New session"}],"unread":{}}}}"#,
        )
        .unwrap();
        let map = tui_tabs(&home).expect("readable");
        assert_eq!(
            map.get("/tmp/oc-probe-ws3"),
            Some(&vec!["ses_f546febb5ffeFdsMXaTv5cRZ6X".to_string()])
        );
        // The cross-project resume wrote the SAME pinned id under the OTHER
        // cwd — the measured bind-correct fact, locked.
        assert_eq!(
            map.get("/tmp/bindB"),
            Some(&vec!["ses_f546febb5ffeFdsMXaTv5cRZ6X".to_string()])
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_absent_or_malformed_tui_tabs_is_none_never_an_error() {
        let bare = std::env::temp_dir().join(format!("ygg-oc-tabs-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        assert_eq!(tui_tabs(&bare), None, "absent file = the plane is not there");
        let bad = std::env::temp_dir().join(format!("ygg-oc-tabs-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bad);
        std::fs::create_dir_all(bad.join(".local/state/opencode/latest/tui")).unwrap();
        std::fs::write(
            bad.join(".local/state/opencode/latest/tui/tabs.json"),
            "{not json",
        )
        .unwrap();
        assert_eq!(tui_tabs(&bad), None, "unparseable = cannot say");
        let _ = std::fs::remove_dir_all(&bad);
    }

    /// The sibling law (measured muse lab host 2026-09-18): every instance
    /// dir speaks, a broken sibling is skipped rather than fatal, and an
    /// absent plane is an empty answer — never an error.
    #[test]
    fn the_sibling_reader_reads_every_instance_dir_and_skips_a_broken_one() {
        let home = std::env::temp_dir().join(format!("ygg-oc-tabs-sib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(
            tui_tabs_across_instances(&home).is_empty(),
            "no state dir at all = the plane is not there"
        );
        std::fs::create_dir_all(home.join(".local/state/opencode/beta/tui")).unwrap();
        std::fs::write(
            home.join(".local/state/opencode/beta/tui/tabs.json"),
            r#"{"cwd":{"/tmp/ws-beta":{"tabs":[{"sessionID":"ses_beta00000000000000000001","title":"b"}],"unread":{}}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".local/state/opencode/latest/tui")).unwrap();
        std::fs::create_dir_all(home.join(".local/state/opencode/broken/tui")).unwrap();
        std::fs::write(
            home.join(".local/state/opencode/broken/tui/tabs.json"),
            "{not json",
        )
        .unwrap();
        let read = tui_tabs_across_instances(&home);
        assert_eq!(read.len(), 1, "only the parseable instance speaks");
        assert_eq!(read[0].0, "beta");
        assert_eq!(
            read[0].1.get("/tmp/ws-beta"),
            Some(&vec!["ses_beta00000000000000000001".to_string()])
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[cfg(test)]
mod tab_id_tests {
    use super::tab_session_id;

    #[test]
    fn tab_rows_match_by_service_id_and_anchors_never_do() {
        assert_eq!(
            tab_session_id("opencode-runtime://ses_fb29241e2ffefXIbrLj4IVdZ8t"),
            Some("ses_fb29241e2ffefXIbrLj4IVdZ8t")
        );
        // The anchor row is uuid-keyed — never a tab.
        assert_eq!(
            tab_session_id("opencode-runtime://81e3b48a-9ef5-41bf-88bb-ae9ac0b8d0a5"),
            None
        );
        // Other schemes never match.
        assert_eq!(tab_session_id("remote-opencode://dev/ses_abc"), None);
        assert_eq!(tab_session_id("local://ses_abc"), None);
    }
}
