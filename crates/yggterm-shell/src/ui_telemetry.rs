use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use yggterm_core::{
    DIAGNOSTIC_RETENTION_MAX_AGE_MS, JsonlRetention, SessionStore, append_retained_jsonl_record,
    append_trace_event,
};

const UI_TELEMETRY_FILENAME: &str = "ui-telemetry.jsonl";
const UI_TELEMETRY_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Rotated ui-telemetry generations: at most 3 days, 96 MiB total.
const UI_TELEMETRY_RETENTION: JsonlRetention = JsonlRetention {
    live_max_bytes: UI_TELEMETRY_MAX_BYTES,
    generations_max_bytes: 96 * 1024 * 1024,
    max_age_ms: DIAGNOSTIC_RETENTION_MAX_AGE_MS,
};
const UI_TELEMETRY_DUPLICATE_THROTTLE_MS: u64 = 2_000;

pub(crate) fn ui_telemetry_should_record(
    recent_ui_telemetry: &mut HashMap<String, (String, u64)>,
    event: &str,
    payload_text: &str,
    now_ms: u64,
) -> bool {
    if let Some((last_payload, last_ms)) = recent_ui_telemetry.get(event)
        && last_payload == payload_text
        && now_ms.saturating_sub(*last_ms) < UI_TELEMETRY_DUPLICATE_THROTTLE_MS
    {
        return false;
    }
    recent_ui_telemetry.insert(event.to_string(), (payload_text.to_string(), now_ms));
    true
}

pub(crate) fn append_ui_telemetry_event(event: &str, payload: Value) {
    let telemetry = json!({
        "ts": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
            .to_string(),
        "event": event,
        "payload": payload,
    });
    if let Ok(store) = SessionStore::open_or_init() {
        let path = store.home_dir().join(UI_TELEMETRY_FILENAME);
        append_retained_jsonl_record(&path, UI_TELEMETRY_RETENTION, &telemetry);
        append_trace_event(store.home_dir(), "ui", "ui_telemetry", event, telemetry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_telemetry_throttles_duplicate_payloads_per_event() {
        let mut recent = HashMap::new();
        assert!(ui_telemetry_should_record(
            &mut recent,
            "terminal_open_attempt",
            r#"{"state":"begin"}"#,
            10
        ));
        assert!(!ui_telemetry_should_record(
            &mut recent,
            "terminal_open_attempt",
            r#"{"state":"begin"}"#,
            1_000
        ));
        assert!(ui_telemetry_should_record(
            &mut recent,
            "terminal_open_attempt",
            r#"{"state":"ready"}"#,
            1_500
        ));
        assert!(ui_telemetry_should_record(
            &mut recent,
            "terminal_open_attempt",
            r#"{"state":"ready"}"#,
            3_501
        ));
    }

    #[test]
    fn ui_telemetry_throttle_is_event_scoped() {
        let mut recent = HashMap::new();
        assert!(ui_telemetry_should_record(
            &mut recent,
            "restore_debug",
            r#"{"same":true}"#,
            10
        ));
        assert!(ui_telemetry_should_record(
            &mut recent,
            "preview_debug",
            r#"{"same":true}"#,
            20
        ));
    }
}
