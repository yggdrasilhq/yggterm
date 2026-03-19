use crate::{
    GhosttyHostSupport, PersistedDaemonState, ServerUiSnapshot, WorkspaceViewMode, YggtermServer,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use yggterm_core::{SessionStore, UiTheme};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEndpoint {
    #[cfg(unix)]
    UnixSocket(PathBuf),
    Tcp { host: String, port: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRuntimeStatus {
    pub host_kind: String,
    pub host_detail: String,
    pub embedded_surface_supported: bool,
    pub bridge_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerRequest {
    Ping,
    Status,
    Snapshot,
    OpenStoredSession {
        path: String,
        session_id: Option<String>,
        cwd: Option<String>,
        title_hint: Option<String>,
    },
    ConnectSsh {
        target_ix: usize,
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
    SyncExternalWindow,
    RaiseExternalWindow,
    SyncTheme {
        theme: UiTheme,
    },
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
    Error { message: String },
}

struct DaemonRuntime {
    support: GhosttyHostSupport,
    state_path: PathBuf,
    server: YggtermServer,
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
            support.bridge_enabled,
            settings.theme,
        );
        let state_path = store.home_dir().join("server-state.json");
        if let Some(saved) = load_persisted_state(&state_path)? {
            server.restore_persisted_state(saved);
        }
        Ok(Self {
            support,
            state_path,
            server,
        })
    }

    fn status(&self) -> ServerRuntimeStatus {
        ServerRuntimeStatus {
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
                path,
                session_id,
                cwd,
                title_hint,
            } => {
                self.server.open_or_focus_session(
                    &path,
                    session_id.as_deref(),
                    cwd.as_deref(),
                    title_hint.as_deref(),
                );
                self.server.set_view_mode(WorkspaceViewMode::Rendered);
                self.persist()?;
                self.snapshot_response(Some(format!("opened {path}")))
            }
            ServerRequest::ConnectSsh { target_ix } => {
                let key = self.server.connect_ssh_target(target_ix);
                self.persist()?;
                self.snapshot_response(key.map(|key| format!("connected {key}")))
            }
            ServerRequest::FocusLive { key } => {
                self.server.focus_live_session(&key);
                self.persist()?;
                self.snapshot_response(Some(format!("focused {key}")))
            }
            ServerRequest::SetViewMode { mode } => {
                self.server.set_view_mode(mode);
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
                self.server.request_terminal_launch_for_active();
                self.server.set_view_mode(WorkspaceViewMode::Terminal);
                self.persist()?;
                self.snapshot_response(Some("requested ghostty".to_string()))
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
        };
        Ok(response)
    }
}

pub fn default_endpoint(home_dir: &Path) -> ServerEndpoint {
    #[cfg(unix)]
    {
        ServerEndpoint::UnixSocket(home_dir.join("server.sock"))
    }

    #[cfg(not(unix))]
    {
        let _ = home_dir;
        ServerEndpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: 58593,
        }
    }
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
    path: &str,
    session_id: Option<&str>,
    cwd: Option<&str>,
    title_hint: Option<&str>,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::OpenStoredSession {
            path: path.to_string(),
            session_id: session_id.map(ToOwned::to_owned),
            cwd: cwd.map(ToOwned::to_owned),
            title_hint: title_hint.map(ToOwned::to_owned),
        },
    )?)
}

pub fn connect_ssh(endpoint: &ServerEndpoint, target_ix: usize) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::ConnectSsh { target_ix },
    )?)
}

pub fn focus_live(endpoint: &ServerEndpoint, key: &str) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::FocusLive {
            key: key.to_string(),
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

pub fn request_terminal_launch(endpoint: &ServerEndpoint) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::RequestTerminalLaunch)?)
}

pub fn sync_external_window(endpoint: &ServerEndpoint) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::SyncExternalWindow)?)
}

pub fn raise_external_window(endpoint: &ServerEndpoint) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(endpoint, &ServerRequest::RaiseExternalWindow)?)
}

pub fn sync_theme(
    endpoint: &ServerEndpoint,
    theme: UiTheme,
) -> Result<(ServerUiSnapshot, Option<String>)> {
    expect_snapshot(send_request(
        endpoint,
        &ServerRequest::SyncTheme { theme },
    )?)
}

pub fn run_daemon(endpoint: &ServerEndpoint, runtime: GhosttyHostSupport) -> Result<()> {
    let runtime = Arc::new(Mutex::new(DaemonRuntime::load(runtime)?));

    #[cfg(unix)]
    if let ServerEndpoint::UnixSocket(path) = endpoint {
        if path.exists() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) => warn!(path=%path.display(), error=%error, "failed to remove stale server socket"),
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
            if let Err(error) = handle_unix_stream(stream, runtime) {
                warn!(error=%error, "daemon request failed");
            }
        }
    }

    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            bail!("unix sockets are unsupported on this platform: {}", path.display())
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
                if let Err(error) = handle_tcp_stream(stream, runtime) {
                    warn!(error=%error, "daemon request failed");
                }
            }
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
    writer.write_all(b"\n").context("writing daemon response terminator")?;
    writer.flush().context("flushing daemon response")?;
    Ok(())
}

fn read_request<R: std::io::Read>(reader: R) -> Result<ServerRequest> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).context("reading daemon request")?;
    if bytes == 0 {
        bail!("daemon client closed connection before sending a request");
    }
    serde_json::from_str(line.trim_end()).context("parsing daemon request")
}

#[cfg(unix)]
fn handle_unix_stream(
    mut stream: std::os::unix::net::UnixStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
) -> Result<()> {
    let request = read_request(stream.try_clone().context("cloning unix stream")?)?;
    let response = {
        let mut runtime = runtime.lock().expect("daemon runtime lock poisoned");
        runtime.handle_request(request)?
    };
    write_response(&mut stream, &response)
}

fn handle_tcp_stream(
    mut stream: std::net::TcpStream,
    runtime: Arc<Mutex<DaemonRuntime>>,
) -> Result<()> {
    let request = read_request(stream.try_clone().context("cloning tcp stream")?)?;
    let response = {
        let mut runtime = runtime.lock().expect("daemon runtime lock poisoned");
        runtime.handle_request(request)?
    };
    write_response(&mut stream, &response)
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
            reader.read_line(&mut line).context("reading daemon response")?;
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
            reader.read_line(&mut line).context("reading daemon response")?;
            serde_json::from_str(line.trim_end()).context("parsing daemon response")
        }
    }
}
