use crate::{
    GhosttyHostSupport, PersistedDaemonState, ServerUiSnapshot, SessionKind, TerminalManager,
    WorkspaceViewMode, YggtermServer,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use yggterm_core::{SessionStore, UiTheme};

pub const SERVER_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEndpoint {
    #[cfg(unix)]
    UnixSocket(PathBuf),
    Tcp {
        host: String,
        port: u16,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRuntimeStatus {
    pub server_version: String,
    pub host_kind: String,
    pub host_detail: String,
    pub embedded_surface_supported: bool,
    pub bridge_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalStreamChunk {
    pub seq: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerRequest {
    Ping,
    Status,
    Snapshot,
    OpenStoredSession {
        session_kind: SessionKind,
        path: String,
        session_id: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
    },
    ConnectSsh {
        target_ix: usize,
    },
    ConnectSshCustom {
        target: String,
        prefix: Option<String>,
    },
    OpenRemoteSession {
        machine_key: String,
        session_id: String,
        cwd: Option<String>,
        title_hint: Option<String>,
    },
    RefreshRemoteMachine {
        machine_key: String,
    },
    StartLocalSession {
        session_kind: SessionKind,
        cwd: Option<String>,
        title_hint: Option<String>,
    },
    SwitchAgentSessionMode {
        path: String,
        session_kind: SessionKind,
    },
    StartCommandSession {
        cwd: Option<String>,
        title_hint: Option<String>,
        launch_command: String,
        source_label: Option<String>,
    },
    FocusLive {
        key: String,
    },
    SetViewMode {
        mode: WorkspaceViewMode,
    },
    TogglePreviewBlock {
        block_ix: usize,
    },
    SetAllPreviewBlocksFolded {
        folded: bool,
    },
    RequestTerminalLaunch,
    TerminalEnsure {
        path: String,
    },
    TerminalRead {
        path: String,
        cursor: u64,
    },
    TerminalWrite {
        path: String,
        data: String,
    },
    TerminalResize {
        path: String,
        cols: u16,
        rows: u16,
    },
    SyncExternalWindow,
    RaiseExternalWindow,
    SyncTheme {
        theme: UiTheme,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerResponse {
    Pong,
    Status(ServerRuntimeStatus),
    Snapshot {
        snapshot: ServerUiSnapshot,
        message: Option<String>,
    },
    TerminalStream {
        cursor: u64,
        chunks: Vec<TerminalStreamChunk>,
    },
    Ack {
        message: Option<String>,
    },
    Error {
        message: String,
    },
}

struct DaemonRuntime {
    support: GhosttyHostSupport,
    state_path: PathBuf,
    store: SessionStore,
    server: YggtermServer,
    terminals: TerminalManager,
}

impl DaemonRuntime {
    fn load(support: GhosttyHostSupport) -> Result<Self> {
        let store = SessionStore::open_or_init()?;
        let settings = store.load_settings().unwrap_or_default();
        let tree = store
            .load_codex_tree(&settings)
            .or_else(|_| store.load_tree())?;
        let mut server = YggtermServer::new(
            &tree,
            settings.prefer_ghostty_backend,
            support.clone(),
            settings.theme,
        );
        let state_path = store.home_dir().join("server-state.json");
        if let Some(saved) = load_persisted_state(&state_path)? {
            server.restore_persisted_state(saved, Some(&store));
        }
        Ok(Self {
            support,
            state_path,
            store,
            server,
            terminals: TerminalManager::new(),
        })
    }

    fn status(&self) -> ServerRuntimeStatus {
        ServerRuntimeStatus {
            server_version: SERVER_PROTOCOL_VERSION.to_string(),
            host_kind: self.support.kind.as_str().to_string(),
            host_detail: self.support.detail.clone(),
            embedded_surface_supported: self.support.embedded_surface_supported,
            bridge_enabled: self.support.bridge_enabled,
        }
    }

    fn snapshot_response(&self, message: Option<String>) -> ServerResponse {
        ServerResponse::Snapshot {
            snapshot: self.server.snapshot(),
            message,
        }
    }

    fn ensure_terminal_for_path(&mut self, path: &str) -> Result<()> {
        self.server.request_terminal_launch_for_path(path);
        let Some((launch_command, cwd)) = self.server.terminal_spec(path) else {
            bail!("no terminal spec for session: {path}");
        };
        self.terminals
            .ensure_session(path, &launch_command, cwd.as_deref())?;
        Ok(())
    }

    fn ensure_terminal_for_active(&mut self) -> Result<()> {
        let Some(path) = self.server.active_session_path().map(ToOwned::to_owned) else {
            bail!("no active session");
        };
        self.ensure_terminal_for_path(&path)
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.state_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating daemon state dir {}", parent.display()))?;
        }
        let state = self.server.persisted_state();
        let json = serde_json::to_string_pretty(&state).context("serializing daemon state")?;
        fs::write(&self.state_path, json)
            .with_context(|| format!("writing daemon state {}", self.state_path.display()))?;
        Ok(())
    }

    fn handle_request(&mut self, request: ServerRequest) -> Result<ServerResponse> {
        let response = match request {
            ServerRequest::Ping => ServerResponse::Pong,
            ServerRequest::Status => ServerResponse::Status(self.status()),
            ServerRequest::Snapshot => self.snapshot_response(None),
            ServerRequest::OpenStoredSession {
                session_kind,
                path,
                session_id,
                cwd,
                title_hint,
            } => {
                let document = if session_kind == SessionKind::Document {
                    self.store.load_document(&path)?
                } else {
                    None
                };
                self.server.open_or_focus_session(
                    session_kind,
                    &path,
                    session_id.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    document.as_ref(),
                );
                self.server.set_view_mode(WorkspaceViewMode::Rendered);
                self.persist()?;
                self.snapshot_response(Some(format!("opened {path}")))
            }
            ServerRequest::ConnectSsh { target_ix } => {
                let target = self.server.ssh_targets().get(target_ix).cloned();
                let (key, reused) = self.server.connect_ssh_target(target_ix);
                if let Some(target) = target.as_ref() {
                    let _ = self.server.refresh_remote_machine_for_ssh_target(target);
                }
                self.persist()?;
                self.snapshot_response(key.map(|key| {
                    if reused {
                        format!("focused existing {key}")
                    } else {
                        format!("connected {key}")
                    }
                }))
            }
            ServerRequest::ConnectSshCustom { target, prefix } => {
                let (key, reused) = self.server.connect_ssh_custom(&target, prefix.as_deref())?;
                if let Some(target) = self
                    .server
                    .ssh_targets()
                    .iter()
                    .find(|candidate| candidate.ssh_target == target && candidate.prefix.as_deref() == prefix.as_deref())
                    .cloned()
                {
                    let _ = self.server.refresh_remote_machine_for_ssh_target(&target);
                }
                self.persist()?;
                self.snapshot_response(Some(if reused {
                    format!("focused existing {key}")
                } else {
                    format!("connected {key}")
                }))
            }
            ServerRequest::OpenRemoteSession {
                machine_key,
                session_id,
                cwd,
                title_hint,
            } => {
                let key = self.server.open_remote_scanned_session(
                    &machine_key,
                    &session_id,
                    cwd.as_deref(),
                    title_hint.as_deref(),
                )?;
                self.persist()?;
                self.snapshot_response(Some(format!("opened {key}")))
            }
            ServerRequest::RefreshRemoteMachine { machine_key } => {
                self.server.refresh_remote_machine_by_key(&machine_key)?;
                self.persist()?;
                self.snapshot_response(Some(format!("refreshed {machine_key}")))
            }
            ServerRequest::StartLocalSession {
                session_kind,
                cwd,
                title_hint,
            } => {
                let key = self.server.start_local_session(
                    session_kind,
                    cwd.as_deref(),
                    title_hint.as_deref(),
                );
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::SwitchAgentSessionMode { path, session_kind } => {
                if self
                    .terminals
                    .recent_activity(&path, std::time::Duration::from_secs(4))
                {
                    bail!("session is still active; wait for it to settle before switching modes");
                }
                let stop_command = self.server.terminal_stop_command(&path);
                self.server.switch_agent_session_mode(&path, session_kind)?;
                if let Some((launch_command, cwd)) = self.server.terminal_spec(&path) {
                    if self.terminals.has_session(&path) {
                        self.terminals.restart_session(
                            &path,
                            &launch_command,
                            cwd.as_deref(),
                            stop_command.as_deref(),
                        )?;
                    }
                }
                self.persist()?;
                self.snapshot_response(Some(format!(
                    "switched to {}",
                    match session_kind {
                        SessionKind::Codex => "codex",
                        SessionKind::CodexLiteLlm => "codex-litellm",
                        SessionKind::Shell => "shell",
                        SessionKind::SshShell => "ssh",
                        SessionKind::Document => "document",
                    }
                )))
            }
            ServerRequest::StartCommandSession {
                cwd,
                title_hint,
                launch_command,
                source_label,
            } => {
                let key = self.server.start_command_session(
                    cwd.as_deref(),
                    title_hint.as_deref(),
                    &launch_command,
                    source_label.as_deref(),
                );
                self.persist()?;
                self.snapshot_response(Some(format!("started {key}")))
            }
            ServerRequest::FocusLive { key } => {
                self.server.focus_live_session(&key);
                self.persist()?;
                self.snapshot_response(Some(format!("focused {key}")))
            }
            ServerRequest::SetViewMode { mode } => {
                self.server.set_view_mode(mode);
                if mode == WorkspaceViewMode::Terminal {
                    self.ensure_terminal_for_active()?;
                }
                self.persist()?;
                self.snapshot_response(Some(match mode {
                    WorkspaceViewMode::Rendered => "preview mode".to_string(),
                    WorkspaceViewMode::Terminal => "terminal mode".to_string(),
                }))
            }
            ServerRequest::TogglePreviewBlock { block_ix } => {
                self.server.toggle_preview_block(block_ix);
                self.persist()?;
                self.snapshot_response(Some(format!("preview block {}", block_ix + 1)))
            }
            ServerRequest::SetAllPreviewBlocksFolded { folded } => {
                self.server.set_all_preview_blocks_folded(folded);
                self.persist()?;
                self.snapshot_response(Some(if folded {
                    "collapsed preview".to_string()
                } else {
                    "expanded preview".to_string()
                }))
            }
            ServerRequest::RequestTerminalLaunch => {
                self.ensure_terminal_for_active()?;
                self.server.set_view_mode(WorkspaceViewMode::Terminal);
                self.persist()?;
                self.snapshot_response(Some("requested terminal".to_string()))
            }
            ServerRequest::TerminalEnsure { path } => {
                self.ensure_terminal_for_path(&path)?;
                ServerResponse::Ack {
                    message: Some("terminal ready".to_string()),
                }
            }
            ServerRequest::TerminalRead { path, cursor } => {
                let stream = self.terminals.read(&path, cursor)?;
                ServerResponse::TerminalStream {
                    cursor: stream.cursor,
                    chunks: stream
                        .chunks
                        .into_iter()
                        .map(|chunk| TerminalStreamChunk {
                            seq: chunk.seq,
                            data: chunk.data,
                        })
                        .collect(),
                }
            }
            ServerRequest::TerminalWrite { path, data } => {
                self.terminals.write(&path, &data)?;
                ServerResponse::Ack { message: None }
            }
            ServerRequest::TerminalResize { path, cols, rows } => {
                self.terminals.resize(&path, cols, rows)?;
                ServerResponse::Ack { message: None }
            }
            ServerRequest::SyncExternalWindow => {
                let message = self.server.sync_external_terminal_window_for_active();
                self.persist()?;
                self.snapshot_response(Some(message))
            }
            ServerRequest::RaiseExternalWindow => {
                let message = self.server.raise_external_terminal_window_for_active();
                self.persist()?;
                self.snapshot_response(Some(message))
            }
            ServerRequest::SyncTheme { theme } => {
                self.server.sync_theme(theme);
                self.persist()?;
                self.snapshot_response(Some("theme synced".to_string()))
            }
            ServerRequest::Shutdown => {
                let summary = self
                    .terminals
                    .shutdown_all(|path| self.server.terminal_stop_command(path));
                self.persist()?;
                ServerResponse::Ack {
                    message: Some(if summary.errors.is_empty() {
                        format!("stopped {} terminal sessions", summary.stopped)
                    } else {
                        format!(
                            "stopped {} terminal sessions, {} errors",
                            summary.stopped,
                            summary.errors.len()
                        )
                    }),
                }
            }
        };
        Ok(response)
    }
}

pub fn default_endpoint(home_dir: &Path) -> ServerEndpoint {
    #[cfg(unix)]
    {
        ServerEndpoint::UnixSocket(home_dir.join(format!(
            "server-{}.sock",
            SERVER_PROTOCOL_VERSION.replace('.', "-")
        )))
    }

    #[cfg(not(unix))]
    {
        let _ = home_dir;
        ServerEndpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: versioned_tcp_port(),
        }
    }
}

#[cfg(not(unix))]
fn versioned_tcp_port() -> u16 {
    let mut parts = SERVER_PROTOCOL_VERSION.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(2);
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0)
        .min(9);
    58000 + major.saturating_mul(100) + minor.saturating_mul(10) + patch
}

pub fn ping(endpoint: &ServerEndpoint) -> Result<()> {
    match send_request(endpoint, &ServerRequest::Ping)? {
        ServerResponse::Pong => Ok(()),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected ping response: {:?}", other),
    }
}

pub fn status(endpoint: &ServerEndpoint) -> Result<ServerRuntimeStatus> {
    match send_request(endpoint, &ServerRequest::Status)? {
        ServerResponse::Status(status) => Ok(status),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected status response: {:?}", other),
    }
}

pub fn snapshot(endpoint: &ServerEndpoint) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::Snapshot)?)
}

pub fn open_stored_session(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::OpenStoredSession {
            session_kind: kind,
            path: path.to_string(),
            session_id: session_id.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
        },
    )?)
}

pub fn connect_ssh(
    endpoint: &ServerEndpoint,
    target_ix: usize,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ConnectSsh { target_ix },
    )?)
}

pub fn connect_ssh_custom(
    endpoint: &ServerEndpoint,
    target: &str,
    prefix: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ConnectSshCustom {
            target: target.to_string(),
            prefix: prefix.map(ToOwned::to_owned),
        },
    )?)
}

pub fn open_remote_session(
    endpoint: &ServerEndpoint,
    machine_key: &str,
    session_id: &str,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::OpenRemoteSession {
            machine_key: machine_key.to_string(),
            session_id: session_id.to_string(),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
        },
    )?)
}

pub fn refresh_remote_machine(
    endpoint: &ServerEndpoint,
    machine_key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RefreshRemoteMachine {
            machine_key: machine_key.to_string(),
        },
    )?)
}

pub fn start_local_session(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    start_local_session_at(endpoint, kind, None, None)
}

pub fn start_local_session_at(
    endpoint: &ServerEndpoint,
    kind: SessionKind,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartLocalSession {
            session_kind: kind,
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
        },
    )?)
}

pub fn start_command_session(
    endpoint: &ServerEndpoint,
    cwd: Option<&str>,
    title_hint: Option<&str>,
    launch_command: &str,
    source_label: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::StartCommandSession {
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
            launch_command: launch_command.to_string(),
            source_label: source_label.map(ToOwned::to_owned),
        },
    )?)
}

pub fn focus_live(
    endpoint: &ServerEndpoint,
    key: &str,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::FocusLive {
            key: key.to_string(),
        },
    )?)
}

pub fn switch_agent_session_mode(
    endpoint: &ServerEndpoint,
    path: &str,
    kind: SessionKind,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SwitchAgentSessionMode {
            path: path.to_string(),
            session_kind: kind,
        },
    )?)
}

pub fn set_view_mode(
    endpoint: &ServerEndpoint,
    mode: WorkspaceViewMode,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetViewMode { mode },
    )?)
}

pub fn toggle_preview_block(
    endpoint: &ServerEndpoint,
    block_ix: usize,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::TogglePreviewBlock { block_ix },
    )?)
}

pub fn set_all_preview_blocks_folded(
    endpoint: &ServerEndpoint,
    folded: bool,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SetAllPreviewBlocksFolded { folded },
    )?)
}

pub fn request_terminal_launch(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::RequestTerminalLaunch,
    )?)
}

pub fn terminal_ensure(endpoint: &ServerEndpoint, path: &str) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalEnsure {
            path: path.to_string(),
        },
    )?)
}

pub fn terminal_read(
    endpoint: &ServerEndpoint,
    path: &str,
    cursor: u64,
) -> Result<(u64, Vec<TerminalStreamChunk>)> {
    match send_request(
        endpoint,
        &ServerRequest::TerminalRead {
            path: path.to_string(),
            cursor,
        },
    )? {
        ServerResponse::TerminalStream { cursor, chunks } => Ok((cursor, chunks)),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected terminal stream response: {:?}", other),
    }
}

pub fn terminal_write(endpoint: &ServerEndpoint, path: &str, data: &str) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalWrite {
            path: path.to_string(),
            data: data.to_string(),
        },
    )?)
}

pub fn terminal_resize(
    endpoint: &ServerEndpoint,
    path: &str,
    cols: u16,
    rows: u16,
) -> Result<Option<String>> {
    expect_ack(send_request(
        endpoint,
        &ServerRequest::TerminalResize {
            path: path.to_string(),
            cols,
            rows,
        },
    )?)
}

pub fn sync_external_window(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::SyncExternalWindow)?)
}

pub fn raise_external_window(
    endpoint: &ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::RaiseExternalWindow)?)
}

pub fn sync_theme(
    endpoint: &ServerEndpoint,
    theme: UiTheme,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::SyncTheme { theme })?)
}

pub fn shutdown(endpoint: &ServerEndpoint) -> Result<Option<String>> {
    expect_ack(send_request(endpoint, &ServerRequest::Shutdown)?)
}

pub fn run_daemon(endpoint: &ServerEndpoint, runtime: GhosttyHostSupport) -> Result<()> {
    let runtime = Arc::new(Mutex::new(DaemonRuntime::load(runtime)?));

    #[cfg(unix)]
    if let ServerEndpoint::UnixSocket(path) = endpoint {
        if path.exists() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) => {
                    warn!(path=%path.display(), error=%error, "failed to remove stale server socket")
                }
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating server socket dir {}", parent.display()))?;
        }
        let listener = std::os::unix::net::UnixListener::bind(path)
            .with_context(|| format!("binding server socket {}", path.display()))?;
        let host = {
            let runtime = runtime.lock().expect("daemon runtime lock poisoned");
            runtime.support.kind.as_str().to_string()
        };
        info!(path=%path.display(), host=%host, "yggterm server daemon listening");
        loop {
            let (stream, _) = listener.accept().context("accepting daemon client")?;
            let runtime = runtime.clone();
            match handle_unix_stream(stream, runtime) {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => warn!(error=%error, "daemon request failed"),
            }
        }
        return Ok(());
    }

    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            bail!(
                "unix sockets are unsupported on this platform: {}",
                path.display()
            )
        }
        ServerEndpoint::Tcp { host, port } => {
            let listener = std::net::TcpListener::bind((host.as_str(), *port))
                .with_context(|| format!("binding server tcp endpoint {}:{}", host, port))?;
            let host_kind = {
                let runtime = runtime.lock().expect("daemon runtime lock poisoned");
                runtime.support.kind.as_str().to_string()
            };
            info!(host=%host, port, host_kind=%host_kind, "yggterm server daemon listening");
            loop {
                let (stream, _) = listener.accept().context("accepting daemon client")?;
                let runtime = runtime.clone();
                match handle_tcp_stream(stream, runtime) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => warn!(error=%error, "daemon request failed"),
                }
            }
            Ok(())
        }
    }
}

fn expect_snapshot(response: ServerResponse) -> Result<(ServerUiSnapshot, Option<String>)> {
    match response {
        ServerResponse::Snapshot { snapshot, message } => Ok((snapshot, message)),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected snapshot response: {:?}", other),
    }
}

fn expect_ack(response: ServerResponse) -> Result<Option<String>> {
    match response {
        ServerResponse::Ack { message } => Ok(message),
        ServerResponse::Error { message } => bail!(message),
        other => bail!("unexpected ack response: {:?}", other),
    }
}

fn load_persisted_state(path: &Path) -> Result<Option<PersistedDaemonState>> {
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(path)
        .with_context(|| format!("reading daemon state {}", path.display()))?;
    let state = serde_json::from_str(&json)
        .with_context(|| format!("parsing daemon state {}", path.display()))?;
    Ok(Some(state))
}

fn write_response<W: Write>(writer: &mut W, response: &ServerResponse) -> Result<()> {
    serde_json::to_writer(&mut *writer, response).context("serializing daemon response")?;
    writer
        .write_all(b"\n")
        .context("writing daemon response terminator")?;
    writer.flush().context("flushing daemon response")?;
    Ok(())
}

fn read_request<R: std::io::Read>(reader: R) -> Result<ServerRequest> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .context("reading daemon request")?;
    if bytes == 0 {
        bail!("daemon client closed connection before sending a request");
    }
    serde_json::from_str(line.trim_end()).context("parsing daemon request")
}

#[cfg(unix)]
fn handle_unix_stream(
    mut stream: std::os::unix::net::UnixStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
) -> Result<bool> {
    let request = read_request(stream.try_clone().context("cloning unix stream")?)?;
    let should_shutdown = matches!(request, ServerRequest::Shutdown);
    let response = {
        let mut runtime = runtime.lock().expect("daemon runtime lock poisoned");
        runtime.handle_request(request)?
    };
    write_response(&mut stream, &response)?;
    Ok(should_shutdown)
}

fn handle_tcp_stream(
    mut stream: std::net::TcpStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
) -> Result<bool> {
    let request = read_request(stream.try_clone().context("cloning tcp stream")?)?;
    let should_shutdown = matches!(request, ServerRequest::Shutdown);
    let response = {
        let mut runtime = runtime.lock().expect("daemon runtime lock poisoned");
        runtime.handle_request(request)?
    };
    write_response(&mut stream, &response)?;
    Ok(should_shutdown)
}

fn send_request(endpoint: &ServerEndpoint, request: &ServerRequest) -> Result<ServerResponse> {
    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            let mut stream = std::os::unix::net::UnixStream::connect(path)
                .with_context(|| format!("connecting to {}", path.display()))?;
            serde_json::to_writer(&mut stream, request).context("serializing daemon request")?;
            stream
                .write_all(b"\n")
                .context("writing daemon request terminator")?;
            stream.flush().context("flushing daemon request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading daemon response")?;
            serde_json::from_str(line.trim_end()).context("parsing daemon response")
        }
        ServerEndpoint::Tcp { host, port } => {
            let mut stream = std::net::TcpStream::connect((host.as_str(), *port))
                .with_context(|| format!("connecting to {}:{}", host, port))?;
            serde_json::to_writer(&mut stream, request).context("serializing daemon request")?;
            stream
                .write_all(b"\n")
                .context("writing daemon request terminator")?;
            stream.flush().context("flushing daemon request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading daemon response")?;
            serde_json::from_str(line.trim_end()).context("parsing daemon response")
        }
    }
}
