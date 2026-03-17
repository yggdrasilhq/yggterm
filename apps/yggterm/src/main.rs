use anyhow::Result;
use yggterm_core::SessionStore;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let store = SessionStore::open_or_init()?;
    let tree = store.load_tree()?;
    let browser_tree = store.load_codex_tree().unwrap_or_else(|_| tree.clone());
    let settings = store.load_settings().unwrap_or_default();
    let ghostty_bridge = yggterm_ghostty_bridge::bridge_status();
    let gtk_glue_detail = yggterm_gtk_glue::detail();

    yggterm_zed_shell::launch_gpui_shell(yggterm_zed_shell::ShellBootstrap {
        tree,
        browser_tree,
        theme: settings.theme,
        ghostty_bridge_enabled: ghostty_bridge.linked_runtime_available(),
        ghostty_embedded_surface_supported: ghostty_bridge.embedded_surface_available(),
        ghostty_bridge_detail: format!("{} {}", ghostty_bridge.detail, gtk_glue_detail),
        prefer_ghostty_backend: settings.prefer_ghostty_backend,
    })
}
