use anyhow::Result;
use std::io::Read;
use std::process::{Command, Stdio};
use yggterm_core::SessionStore;
use yggterm_server::{
    default_endpoint, detect_ghostty_host, ping, run_attach, run_daemon, shutdown, status,
};

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
    if args.len() == 3 && args[0] == "server" && args[1] == "attach" {
        return run_attach(&args[2]);
    }
    if args.as_slice() == ["server", "shutdown"] {
        let endpoint = default_endpoint(store.home_dir());
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
        }
        return Ok(());
    }
    if let Some(command) = args.first()
        && command == "doc"
    {
        return run_document_cli(&store, &args[1..]);
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
    let daemon_connected = ping(&endpoint).is_ok();
    let daemon_status = if daemon_connected {
        status(&endpoint).ok()
    } else {
        None
    };
    if !daemon_connected {
        spawn_server_daemon()?;
    }

    let launch_result = yggterm_ui::launch_shell(yggterm_ui::ShellBootstrap {
        tree,
        browser_tree,
        settings,
        settings_path,
        server_endpoint: endpoint.clone(),
        initial_server_snapshot: None,
        theme,
        ghostty_bridge_enabled: host.bridge_enabled,
        ghostty_embedded_surface_supported: host.embedded_surface_supported,
        ghostty_bridge_detail: host.detail.clone(),
        server_daemon_detail: if let Some(status) = daemon_status {
            format!("server {} connected", status.server_version)
        } else if daemon_connected {
            "server connected".to_string()
        } else {
            "starting server…".to_string()
        },
        prefer_ghostty_backend,
    });
    let _ = shutdown(&endpoint);
    launch_result
}

fn spawn_server_daemon() -> Result<()> {
    let current_exe = std::env::current_exe()?;
    Command::new(current_exe)
        .arg("server")
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn run_document_cli(store: &SessionStore, args: &[String]) -> Result<()> {
    match args {
        [command] if command == "list" || command == "ls" => {
            for document in store.list_documents()? {
                println!("{}\t{}", document.virtual_path, document.title);
            }
            Ok(())
        }
        [command, path] if command == "cat" => {
            if let Some(document) = store.load_document(path)? {
                print!("{}", document.body);
            }
            Ok(())
        }
        [command, path] if command == "write" => {
            let mut body = String::new();
            std::io::stdin().read_to_string(&mut body)?;
            store.save_document(path, None, &body)?;
            println!("saved {}", path);
            Ok(())
        }
        [command, path, title] if command == "write" => {
            let mut body = String::new();
            std::io::stdin().read_to_string(&mut body)?;
            store.save_document(path, Some(title), &body)?;
            println!("saved {}", path);
            Ok(())
        }
        _ => {
            eprintln!(
                "usage:\n  yggterm doc list\n  yggterm doc cat <virtual-path>\n  yggterm doc write <virtual-path> [title] < body.md"
            );
            Ok(())
        }
    }
}
