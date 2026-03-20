use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 36;
const MAX_CHUNKS: usize = 4096;

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

    pub fn ensure_session(&mut self, key: &str, launch_command: &str, cwd: Option<&str>) -> Result<()> {
        if self.sessions.contains_key(key) {
            return Ok(());
        }
        let runtime = PtySessionRuntime::spawn(key, launch_command, cwd)?;
        self.sessions.insert(key.to_string(), runtime);
        Ok(())
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
        let reader_chunks = Arc::clone(&chunks);
        let reader_seq = Arc::clone(&seq);
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
        })
    }

    fn read(&self, cursor: u64) -> TerminalReadResult {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let next_cursor = self.seq.load(Ordering::SeqCst);
        let chunks = chunks
            .iter()
            .filter(|chunk| chunk.seq > cursor)
            .cloned()
            .collect();
        TerminalReadResult {
            cursor: next_cursor,
            chunks,
        }
    }

    fn write(&self, data: &str) -> Result<()> {
        let mut writer = self.writer.lock().expect("pty writer lock poisoned");
        writer
            .write_all(data.as_bytes())
            .context("writing to pty")?;
        writer.flush().context("flushing pty writer")?;
        Ok(())
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
        if let Some(command) = stop_command
            && !command.is_empty()
        {
            let _ = self.write(command);
            thread::sleep(Duration::from_millis(180));
        }

        let mut child = self.child.lock().expect("pty child lock poisoned");
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

fn shell_command(launch_command: &str, cwd: Option<&str>) -> CommandBuilder {
    if cfg!(windows) {
        let mut command = CommandBuilder::new("cmd.exe");
        command.arg("/C");
        command.arg(launch_command);
        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }
        return command;
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut command = CommandBuilder::new(shell);
    command.arg("-lc");
    command.arg(launch_command);
    if let Some(cwd) = cwd {
        command.cwd(cwd);
    }
    command
}
