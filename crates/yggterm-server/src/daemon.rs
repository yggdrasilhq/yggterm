use crate::GhosttyHostSupport;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerResponse {
    Pong,
    Status(ServerRuntimeStatus),
    Error { message: String },
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

pub fn run_daemon(endpoint: &ServerEndpoint, runtime: GhosttyHostSupport) -> Result<()> {
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
        info!(path=%path.display(), host=%runtime.kind.as_str(), "yggterm server daemon listening");
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
            info!(host=%host, port, host_kind=%runtime.kind.as_str(), "yggterm server daemon listening");
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

fn build_status(runtime: GhosttyHostSupport) -> ServerRuntimeStatus {
    ServerRuntimeStatus {
        host_kind: runtime.kind.as_str().to_string(),
        host_detail: runtime.detail,
        embedded_surface_supported: runtime.embedded_surface_supported,
        bridge_enabled: runtime.bridge_enabled,
    }
}

fn handle_request(request: ServerRequest, runtime: GhosttyHostSupport) -> ServerResponse {
    match request {
        ServerRequest::Ping => ServerResponse::Pong,
        ServerRequest::Status => ServerResponse::Status(build_status(runtime)),
    }
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
    runtime: GhosttyHostSupport,
) -> Result<()> {
    let request = read_request(stream.try_clone().context("cloning unix stream")?)?;
    let response = handle_request(request, runtime);
    write_response(&mut stream, &response)
}

fn handle_tcp_stream(mut stream: std::net::TcpStream, runtime: GhosttyHostSupport) -> Result<()> {
    let request = read_request(stream.try_clone().context("cloning tcp stream")?)?;
    let response = handle_request(request, runtime);
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
