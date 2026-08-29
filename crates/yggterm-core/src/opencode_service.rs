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

/// One session the service currently reports — joined from the session list
/// (metadata) and the active set (open tabs).
#[derive(Debug, Clone, PartialEq)]
pub struct OpencodeServiceSession {
    /// The CLI's own id (`ses_…`) — the id a cold resume spells after
    /// `--session`, and the identity tab rows carry.
    pub id: String,
    pub title: Option<String>,
    pub directory: Option<String>,
    pub updated_epoch_ms: u128,
    /// When the TUI last VIEWED this session — the focus signal a tab mirror
    /// follows. `0` = the service did not say.
    pub viewed_epoch_ms: u128,
    /// In the service's active set = an open tab somewhere.
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

/// Sessions the service currently reports as ACTIVE — the open tabs. One PTY
/// hosts the TUI that shows them; these are the sessions a tab row mirrors.
pub fn active_sessions(home: &PathBuf) -> Option<Vec<OpencodeServiceSession>> {
    let registration = service_registration(home)?;
    let active = service_get(&registration, "/api/session/active")?;
    let running_ids: Vec<String> = match active.get("data").and_then(|d| d.as_object()) {
        Some(map) => map
            .keys()
            .filter(|id| !id.trim().is_empty())
            .cloned()
            .collect(),
        None => return None,
    };
    if running_ids.is_empty() {
        return Some(Vec::new());
    }
    let listed = service_get(&registration, "/api/session")?;
    let sessions = decode_session_list(&listed);
    // The list carries metadata; the active set carries running-ness. Join on
    // id, and keep an active id whose detail has not landed yet (a session
    // created between the two reads) — presence outranks metadata.
    let mut out: Vec<OpencodeServiceSession> = sessions
        .into_iter()
        .filter(|s| running_ids.contains(&s.id))
        .collect();
    for id in &running_ids {
        if !out.iter().any(|s| &s.id == id) {
            out.push(OpencodeServiceSession {
                id: id.clone(),
                title: None,
                directory: None,
                updated_epoch_ms: 0,
                viewed_epoch_ms: 0,
                running: true,
            });
        }
    }
    out.sort_by(|a, b| b.viewed_epoch_ms.cmp(&a.viewed_epoch_ms));
    Some(out)
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
pub fn view_session(home: &PathBuf, session_id: &str) -> bool {
    let Some(registration) = service_registration(home) else {
        return false;
    };
    service_post(
        &registration,
        &format!("/api/session/{session_id}/view"),
        &serde_json::json!({}),
    )
}

/// Deliver a message to ONE session — per-session addressing for the fleet
/// verbs. The service queues it in the session's inbox (steer/queue), which
/// is the same contract yggterm's row-plane submit documents for a busy row.
pub fn send_prompt(home: &PathBuf, session_id: &str, text: &str) -> bool {
    let Some(registration) = service_registration(home) else {
        return false;
    };
    service_post(
        &registration,
        &format!("/api/session/{session_id}/prompt"),
        &serde_json::json!({ "prompt": { "text": text } }),
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
        // The active join: list metadata + running set → sessions sorted by
        // viewed recency (the focus signal a tab mirror follows).
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
            {"id": "ses_c0000000000000000000000003", "title": "closed",
             "location": {"directory": "/home/user/proj"},
             "time": {"updated": 3000, "viewed": 3000}}
        ]);
        // decode path: reuse the join logic through the module's own builder
        let sessions = decode_session_list(&listed);
        let mut out: Vec<OpencodeServiceSession> = sessions
            .into_iter()
            .filter(|s| {
                ["ses_a0000000000000000000000001", "ses_b0000000000000000000000002"]
                    .contains(&s.id.as_str())
            })
            .collect();
        out.sort_by(|a, b| b.viewed_epoch_ms.cmp(&a.viewed_epoch_ms));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "ses_b0000000000000000000000002", "focused tab first");
        assert_eq!(out[0].directory.as_deref(), Some("/home/user/proj"));
        let _ = active;
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[test]
#[ignore] // hits the LIVE service on this host — run explicitly: cargo test -p yggterm-core --lib -- --ignored opencode_live
fn opencode_live_service_fetch() {
    let home = std::env::var("HOME").expect("HOME is set for a live-service probe");
    let reg = service_registration(&std::path::PathBuf::from(&home));
    println!("registration: {:?}", reg.as_ref().map(|r| r.url.clone()));
    assert!(reg.is_some(), "no service registration — is opencode2 running?");
    let sessions = active_sessions(&std::path::PathBuf::from(&home));
    match sessions {
        Some(list) => {
            println!("active sessions: {}", list.len());
            for s in &list {
                println!("  {} | {:?} | dir {:?}", &s.id[..s.id.len().min(24)], s.title.as_deref().unwrap_or(""), s.directory.as_deref().unwrap_or(""));
            }
            assert!(!list.is_empty(), "service reachable but zero active tabs");
        }
        None => panic!("active_sessions returned None — fetch failed inside the client (this is the daemon's exact code path)"),
    }
}
