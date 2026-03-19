use anyhow::Result;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use yggterm_core::SessionStore;
use yggterm_server::{default_endpoint, detect_ghostty_host, ping, run_daemon, status};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let store = SessionStore::open_or_init()?;
    if args.as_slice() == ["server", "daemon"] {
        let endpoint = default_endpoint(store.home_dir());
        let host = detect_ghostty_host();
        return run_daemon(&endpoint, host);
    }

    let tree = store.load_tree()?;
    let settings = store.load_settings().unwrap_or_default();
    let browser_tree = store
        .load_codex_tree(&settings)
        .unwrap_or_else(|_| tree.clone());
    let settings_path = store.settings_path();
    let theme = settings.theme;
    let prefer_ghostty_backend = settings.prefer_ghostty_backend;
    let endpoint = default_endpoint(store.home_dir());
    let host = detect_ghostty_host();
    let server_daemon_detail = ensure_server_daemon(&endpoint)?;
    let server_runtime = status(&endpoint).ok();

    yggterm_ui::launch_shell(yggterm_ui::ShellBootstrap {
        tree,
        browser_tree,
        settings,
        settings_path,
        theme,
        ghostty_bridge_enabled: host.bridge_enabled,
        ghostty_embedded_surface_supported: host.embedded_surface_supported,
        ghostty_bridge_detail: match &server_runtime {
            Some(runtime) => format!("{} · server {}", host.detail, runtime.host_kind),
            None => host.detail.clone(),
        },
        server_daemon_detail,
        prefer_ghostty_backend,
    })
}

fn ensure_server_daemon(endpoint: &yggterm_server::ServerEndpoint) -> Result<String> {
    if ping(endpoint).is_ok() {
        return Ok(endpoint_detail(endpoint, true));
    }

    let current_exe = std::env::current_exe()?;
    Command::new(current_exe)
        .arg("server")
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    for _ in 0..20 {
        thread::sleep(Duration::from_millis(150));
        if ping(endpoint).is_ok() {
            return Ok(endpoint_detail(endpoint, true));
        }
    }

    Ok(endpoint_detail(endpoint, false))
}

fn endpoint_detail(endpoint: &yggterm_server::ServerEndpoint, connected: bool) -> String {
    let status = if connected { "connected" } else { "unavailable" };
    match endpoint {
        #[cfg(unix)]
        yggterm_server::ServerEndpoint::UnixSocket(path) => {
            format!("server {status} via {}", path.display())
        }
        yggterm_server::ServerEndpoint::Tcp { host, port } => {
            format!("server {status} via {host}:{port}")
        }
    }
}
