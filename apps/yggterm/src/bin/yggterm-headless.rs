use anyhow::Result;
use yggterm_core::{SessionStore, detect_install_context, refresh_desktop_integration};
use yggterm_server::{
    cleanup_legacy_daemons, default_endpoint, detect_ghostty_host, ping, run_attach, run_daemon,
    run_remote_generation_context, run_remote_preview, run_remote_protocol_version,
    run_remote_resume_codex, run_remote_scan, run_remote_stage_clipboard_png,
    run_remote_upsert_generated_copy, shutdown, status,
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
        let _ = cleanup_legacy_daemons(&endpoint, &current_exe);
        let host = detect_ghostty_host();
        return run_daemon(&endpoint, host);
    }
    if args.len() == 3 && args[0] == "server" && args[1] == "attach" {
        return run_attach(&args[2]);
    }
    if args.as_slice() == ["server", "remote", "stage-clipboard-png"] {
        return run_remote_stage_clipboard_png();
    }
    if args.as_slice() == ["server", "remote", "protocol-version"] {
        return run_remote_protocol_version();
    }
    if args.len() >= 4 && args[0] == "server" && args[1] == "remote" && args[2] == "resume-codex" {
        return run_remote_resume_codex(
            &args[3],
            args.get(4).map(String::as_str).filter(|value| !value.is_empty()),
        );
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "remote" && args[2] == "scan" {
        return run_remote_scan(args.get(3).map(String::as_str));
    }
    if args.len() == 4 && args[0] == "server" && args[1] == "remote" && args[2] == "preview" {
        return run_remote_preview(&args[3]);
    }
    if args.len() == 4
        && args[0] == "server"
        && args[1] == "remote"
        && args[2] == "generation-context"
    {
        return run_remote_generation_context(&args[3]);
    }
    if args.len() == 4 && args[0] == "server" && args[1] == "remote" && args[2] == "upsert-generated-copy" {
        return run_remote_upsert_generated_copy(&args[3]);
    }
    if args.as_slice() == ["server", "shutdown"] {
        let endpoint = default_endpoint(store.home_dir());
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
        }
        return Ok(());
    }
    if args.as_slice() == ["server", "ping"] {
        let endpoint = default_endpoint(store.home_dir());
        ping(&endpoint)?;
        println!("pong");
        return Ok(());
    }
    if args.as_slice() == ["server", "status"] {
        let endpoint = default_endpoint(store.home_dir());
        let runtime = status(&endpoint)?;
        println!("{}", serde_json::to_string_pretty(&runtime)?);
        return Ok(());
    }

    anyhow::bail!("yggterm-headless only supports server subcommands");
}
