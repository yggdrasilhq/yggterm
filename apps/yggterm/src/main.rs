use anyhow::Result;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;
use tracing::warn;
use yggterm_core::{
    ENV_YGGTERM_HOME, InstallContext, SessionStore, UpdatePolicy, check_for_update,
    detect_install_context, install_release_update, refresh_desktop_integration,
};
use yggterm_server::{
    SessionKind, default_endpoint, detect_ghostty_host, ping, run_attach, run_daemon, shutdown,
    start_local_session, status,
};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let current_exe = std::env::current_exe()?;
    let install_context = detect_install_context(&current_exe)?;
    let store = SessionStore::open_or_init()?;
    if args.as_slice() == ["server", "daemon"] {
        let _ = refresh_desktop_integration(&install_context);
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
    if args.as_slice() == ["server", "smoke"] {
        return run_server_smoke();
    }
    if let Some(command) = args.first()
        && command == "install"
    {
        return run_install_cli(&install_context);
    }
    if let Some(command) = args.first()
        && command == "doc"
    {
        return run_document_cli(&store, &args[1..]);
    }

    if args.is_empty() {
        if let Err(error) = refresh_desktop_integration(&install_context) {
            warn!(error=%error, "failed to refresh desktop integration");
        }
        if install_context.update_policy == UpdatePolicy::Auto
            && std::env::var_os("YGGTERM_SKIP_SELF_UPDATE").is_none()
        {
            match check_for_update(&install_context) {
                Ok(Some(update)) => match install_release_update(&install_context, &update) {
                    Ok(next_exe) => {
                        Command::new(&next_exe).spawn()?;
                        return Ok(());
                    }
                    Err(error) => warn!(error=%error, "failed to install direct update"),
                },
                Ok(None) => {}
                Err(error) => warn!(error=%error, "failed to check for update"),
            }
        }
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
        install_context,
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

fn run_install_cli(context: &InstallContext) -> Result<()> {
    let args = std::env::args().skip(2).collect::<Vec<_>>();
    match args.as_slice() {
        [command] if command == "integrate" => {
            for note in refresh_desktop_integration(context)? {
                println!("{note}");
            }
            Ok(())
        }
        [command] if command == "state" => {
            println!("{}", serde_json::to_string_pretty(context)?);
            Ok(())
        }
        [command] if command == "self-update" => {
            if context.update_policy != UpdatePolicy::Auto {
                println!("self-update disabled for this install channel");
                return Ok(());
            }
            if let Some(update) = check_for_update(context)? {
                let next = install_release_update(context, &update)?;
                println!("installed {} at {}", update.version, next.display());
            } else {
                println!("already up to date");
            }
            Ok(())
        }
        _ => {
            eprintln!(
                "usage:\n  yggterm install integrate\n  yggterm install state\n  yggterm install self-update"
            );
            Ok(())
        }
    }
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

fn run_server_smoke() -> Result<()> {
    let temp_home = std::env::temp_dir().join(format!(
        "yggterm-smoke-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
    ));
    fs::create_dir_all(&temp_home)?;
    let endpoint = default_endpoint(&temp_home);
    let current_exe = std::env::current_exe()?;
    let mut child = Command::new(current_exe)
        .arg("server")
        .arg("daemon")
        .env(ENV_YGGTERM_HOME, &temp_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let result = (|| -> Result<()> {
        for _ in 0..40 {
            if ping(&endpoint).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        ping(&endpoint)?;
        let runtime = status(&endpoint)?;
        let _ = start_local_session(&endpoint, SessionKind::Shell)?;
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
        }
        println!("server {} smoke ok", runtime.server_version);
        Ok(())
    })();

    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&temp_home);
    result
}
