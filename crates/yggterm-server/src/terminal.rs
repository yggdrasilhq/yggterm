use crate::codex_cli::terminal_identity_env_pairs;
use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 36;
const MAX_CHUNKS: usize = 4096;
const INITIAL_ATTACH_MAX_CHUNKS: usize = 192;
const INITIAL_ATTACH_MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone)]
pub struct TerminalChunk {
    pub seq: u64,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct TerminalReadResult {
    pub cursor: u64,
    pub chunks: Vec<TerminalChunk>,
}

pub struct TerminalManager {
    sessions: HashMap<String, PtySessionRuntime>,
}

#[derive(Debug, Clone)]
pub struct TerminalShutdownSummary {
    pub stopped: usize,
    pub errors: Vec<String>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn ensure_session(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
    ) -> Result<()> {
        if self.sessions.contains_key(key) {
            return Ok(());
        }
        let runtime = PtySessionRuntime::spawn(key, launch_command, cwd)?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(())
    }

    pub fn session_matches_spec(&self, key: &str, launch_command: &str, cwd: Option<&str>) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.matches_spec(launch_command, cwd))
    }

    pub fn read(&self, key: &str, cursor: u64) -> Result<TerminalReadResult> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        Ok(session.read(cursor))
    }

    pub fn write(&self, key: &str, data: &str) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        session.write(data)
    }

    pub fn resize(&self, key: &str, cols: u16, rows: u16) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        session.resize(cols, rows)
    }

    pub fn has_session(&self, key: &str) -> bool {
        self.sessions.contains_key(key)
    }

    pub fn recent_activity(&self, key: &str, within: Duration) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.recent_activity(within))
    }

    pub fn restart_session(
        &mut self,
        key: &str,
        launch_command: &str,
        cwd: Option<&str>,
        stop_command: Option<&str>,
    ) -> Result<()> {
        if let Some(runtime) = self.sessions.remove(key) {
            runtime.shutdown(stop_command)?;
        }
        let runtime = PtySessionRuntime::spawn(key, launch_command, cwd)?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(())
    }

    pub fn remove_session(&mut self, key: &str, stop_command: Option<&str>) -> Result<bool> {
        let Some(runtime) = self.sessions.remove(key) else {
            return Ok(false);
        };
        runtime.shutdown(stop_command)?;
        Ok(true)
    }

    pub fn shutdown_all<F>(&mut self, stop_command: F) -> TerminalShutdownSummary
    where
        F: Fn(&str) -> Option<String>,
    {
        let keys = self.sessions.keys().cloned().collect::<Vec<_>>();
        let mut stopped = 0usize;
        let mut errors = Vec::new();
        for key in keys {
            let Some(runtime) = self.sessions.remove(&key) else {
                continue;
            };
            match runtime.shutdown(stop_command(&key).as_deref()) {
                Ok(()) => stopped += 1,
                Err(error) => errors.push(format!("{key}: {error}")),
            }
        }
        TerminalShutdownSummary { stopped, errors }
    }
}

struct PtySessionRuntime {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    chunks: Arc<Mutex<VecDeque<TerminalChunk>>>,
    seq: Arc<AtomicU64>,
    last_activity_ms: Arc<AtomicU64>,
    launch_command: String,
    cwd: Option<String>,
}

impl PtySessionRuntime {
    fn spawn(key: &str, launch_command: &str, cwd: Option<&str>) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("opening pty")?;

        let command = shell_command(launch_command, cwd);
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("spawning terminal session {key}"))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("cloning pty reader")?;
        let writer = pair.master.take_writer().context("taking pty writer")?;
        let chunks = Arc::new(Mutex::new(VecDeque::new()));
        let seq = Arc::new(AtomicU64::new(0));
        let last_activity_ms = Arc::new(AtomicU64::new(now_millis()));
        let reader_chunks = Arc::clone(&chunks);
        let reader_seq = Arc::clone(&seq);
        let reader_activity = Arc::clone(&last_activity_ms);
        let key_label = key.to_string();

        thread::Builder::new()
            .name(format!("pty-reader-{key}"))
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(bytes) => {
                            let data = String::from_utf8_lossy(&buffer[..bytes]).to_string();
                            if data.is_empty() {
                                continue;
                            }
                            reader_activity.store(now_millis(), Ordering::SeqCst);
                            let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
                            let mut chunks = reader_chunks.lock().expect("pty chunk lock poisoned");
                            chunks.push_back(TerminalChunk {
                                seq: seq_value,
                                data,
                            });
                            while chunks.len() > MAX_CHUNKS {
                                chunks.pop_front();
                            }
                        }
                        Err(error) => {
                            reader_activity.store(now_millis(), Ordering::SeqCst);
                            let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
                            let mut chunks = reader_chunks.lock().expect("pty chunk lock poisoned");
                            chunks.push_back(TerminalChunk {
                                seq: seq_value,
                                data: format!("\r\n[yggterm] terminal reader stopped for {key_label}: {error}\r\n"),
                            });
                            while chunks.len() > MAX_CHUNKS {
                                chunks.pop_front();
                            }
                            break;
                        }
                    }
                }
            })
            .context("spawning pty reader thread")?;

        Ok(Self {
            master: Arc::new(Mutex::new(pair.master)),
            writer: Arc::new(Mutex::new(writer)),
            child: Arc::new(Mutex::new(child)),
            chunks,
            seq,
            last_activity_ms,
            launch_command: launch_command.to_string(),
            cwd: cwd.map(|value| value.to_string()),
        })
    }

    fn matches_spec(&self, launch_command: &str, cwd: Option<&str>) -> bool {
        self.launch_command == launch_command && self.cwd.as_deref() == cwd
    }

    fn read(&self, cursor: u64) -> TerminalReadResult {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let next_cursor = self.seq.load(Ordering::SeqCst);
        let chunks = if cursor == 0 {
            let mut selected = Vec::new();
            let mut bytes = 0usize;
            for chunk in chunks.iter().rev() {
                let chunk_len = chunk.data.len();
                if !selected.is_empty()
                    && (selected.len() >= INITIAL_ATTACH_MAX_CHUNKS
                        || bytes.saturating_add(chunk_len) > INITIAL_ATTACH_MAX_BYTES)
                {
                    break;
                }
                bytes = bytes.saturating_add(chunk_len);
                selected.push(chunk.clone());
            }
            selected.reverse();
            selected
        } else {
            chunks
                .iter()
                .filter(|chunk| chunk.seq > cursor)
                .cloned()
                .collect()
        };
        TerminalReadResult {
            cursor: next_cursor,
            chunks,
        }
    }

    fn write(&self, data: &str) -> Result<()> {
        let mut writer = self.writer.lock().expect("pty writer lock poisoned");
        self.last_activity_ms.store(now_millis(), Ordering::SeqCst);
        writer
            .write_all(data.as_bytes())
            .context("writing to pty")?;
        writer.flush().context("flushing pty writer")?;
        Ok(())
    }

    fn recent_activity(&self, within: Duration) -> bool {
        let now = now_millis();
        let last = self.last_activity_ms.load(Ordering::SeqCst);
        now.saturating_sub(last) <= within.as_millis() as u64
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        if cols == 0 || rows == 0 {
            bail!("terminal size must be greater than zero");
        }
        let master = self.master.lock().expect("pty master lock poisoned");
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("resizing pty")?;
        Ok(())
    }

    fn shutdown(&self, stop_command: Option<&str>) -> Result<()> {
        let mut child = self.child.lock().expect("pty child lock poisoned");
        if let Some(command) = stop_command
            && !command.is_empty()
        {
            let _ = self.write(command);
            for _ in 0..12 {
                if child
                    .try_wait()
                    .context("checking terminal exit state")?
                    .is_some()
                {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(120));
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn shell_command(launch_command: &str, cwd: Option<&str>) -> CommandBuilder {
    if cfg!(windows) {
        let mut command = CommandBuilder::new("cmd.exe");
        command.arg("/C");
        command.arg(launch_command);
        for (key, value) in terminal_identity_env_pairs() {
            command.env(key, value);
        }
        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }
        return command;
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut command = CommandBuilder::new(shell);
    command.arg("-c");
    command.arg(launch_command);
    for (key, value) in terminal_identity_env_pairs() {
        command.env(key, value);
    }
    if let Some(cwd) = cwd {
        if shell_uses_bash_prompt_cwd() {
            command.env("YGGTERM_START_CWD", cwd);
            command.env(
                "PROMPT_COMMAND",
                r#"cd -- "$YGGTERM_START_CWD"; unset PROMPT_COMMAND"#,
            );
        }
        command.cwd(cwd);
    }
    command
}

fn shell_uses_bash_prompt_cwd() -> bool {
    std::env::var("SHELL")
        .ok()
        .and_then(|value| {
            std::path::Path::new(&value)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("bash"))
        })
        .unwrap_or(true)
}
