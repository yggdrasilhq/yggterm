//! Exercise the real shared trace writer without compiling platform-specific
//! internal test modules. The same executable runs on Linux and Windows.
use std::fs;
use std::path::PathBuf;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn trace_writer_follows_external_rotation_with_bounded_overlap() {
    let base = std::env::var_os("YGG_TEST_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".yggterm/scratchpad/tests"));
    let home = Scratch(base.join(format!("portable-trace-{}", uuid::Uuid::new_v4())));
    fs::create_dir_all(&home.0).unwrap();
    yggterm_core::append_trace_event(&home.0, "fixture", "rotation", "before", serde_json::json!({}));
    let path = yggterm_core::event_trace_path(&home.0);
    let old = home.0.join("external-generation.jsonl");
    fs::rename(&path, &old).unwrap();
    fs::write(&path, b"").unwrap();
    let pad = "p".repeat(300);
    for i in 0..2000 {
        yggterm_core::append_trace_event(&home.0, "fixture", "rotation", "after",
                                       serde_json::json!({"i": i, "pad": pad}));
    }
    let active = fs::read_to_string(&path).unwrap();
    let previous = fs::read_to_string(&old).unwrap();
    assert!(previous.contains("\"name\":\"before\""));
    assert!(!active.contains("\"name\":\"before\""));
    assert!(active.contains("\"i\":1999"));
    assert!(!previous.contains("\"i\":1999"));
    assert!(previous.matches("\"name\":\"after\"").count() <= 900,
            "overlap must stay bounded by the coarse rotation-check interval");
}
