use crate::codex_cli::terminal_identity_env_pairs;
use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use yggterm_core::{append_trace_event, resolve_yggterm_home};

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 36;
const MAX_CHUNKS: usize = 512;
pub const MAX_BUFFER_BYTES: usize = 2 * 1024 * 1024;
pub const IDLE_TRIM_MAX_CHUNKS: usize = 64;
pub const IDLE_TRIM_MAX_BYTES: usize = 128 * 1024;
const INITIAL_ATTACH_MAX_CHUNKS: usize = 192;
const INITIAL_ATTACH_MAX_BYTES: usize = 512 * 1024;
const INITIAL_ATTACH_TRAILING_NOISE_CHUNKS: usize = 16;

#[derive(Debug, Clone)]
pub struct TerminalChunk {
    pub seq: u64,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct TerminalReadResult {
    pub cursor: u64,
    pub chunks: Vec<TerminalChunk>,
    pub running: bool,
    pub runtime_output_seen: bool,
    pub eof_without_output: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalBufferStats {
    pub session_count: usize,
    pub retained_chunks: usize,
    pub retained_bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalTrimSummary {
    pub trimmed_sessions: usize,
    pub reclaimed_bytes: usize,
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

    pub fn session_matches_remote_resume_spec(&self, key: &str, cwd: Option<&str>) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.matches_remote_resume_spec(cwd))
    }

    pub fn session_is_running(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.is_running())
    }

    pub fn session_has_output(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.has_output())
    }

    pub fn session_has_runtime_output(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.has_runtime_output())
    }

    pub fn session_hit_eof_without_output(&self, key: &str) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|session| session.hit_eof_without_output())
    }

    pub fn session_runtime_age_ms(&self, key: &str) -> Option<u64> {
        self.sessions.get(key).map(|session| session.age_ms())
    }

    pub fn session_snapshot(&self, key: &str) -> Option<String> {
        self.sessions.get(key).map(|session| session.snapshot())
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

    pub fn seed_session(&self, key: &str, data: &str) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .with_context(|| format!("terminal session not found: {key}"))?;
        session.seed_snapshot(data);
        Ok(())
    }

    pub fn stats(&self) -> TerminalBufferStats {
        let mut stats = TerminalBufferStats {
            session_count: self.sessions.len(),
            ..TerminalBufferStats::default()
        };
        for session in self.sessions.values() {
            let (chunks, bytes) = session.buffer_usage();
            stats.retained_chunks += chunks;
            stats.retained_bytes += bytes;
        }
        stats
    }

    pub fn trim_idle_buffers(&self, within: Duration) -> TerminalTrimSummary {
        let mut summary = TerminalTrimSummary::default();
        for session in self.sessions.values() {
            let reclaimed = session.trim_idle_buffer(within);
            if reclaimed > 0 {
                summary.trimmed_sessions += 1;
                summary.reclaimed_bytes += reclaimed;
            }
        }
        summary
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
        trace_terminal_event(
            "restart",
            serde_json::json!({
                "path": key,
                "cwd": cwd,
                "launch_command": launch_command,
                "stop_command": stop_command,
            }),
        );
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
        let worker_limit = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .clamp(1, 4);
        let mut pending = Vec::new();

        let flush_pending = |pending: &mut Vec<(String, thread::JoinHandle<Result<()>>)>,
                             stopped: &mut usize,
                             errors: &mut Vec<String>| {
            for (key, handle) in pending.drain(..) {
                match handle.join() {
                    Ok(Ok(())) => *stopped += 1,
                    Ok(Err(error)) => errors.push(format!("{key}: {error}")),
                    Err(_) => errors.push(format!("{key}: terminal shutdown thread panicked")),
                }
            }
        };

        for key in keys {
            let Some(runtime) = self.sessions.remove(&key) else {
                continue;
            };
            let stop = stop_command(&key);
            pending.push((
                key,
                thread::spawn(move || runtime.shutdown(stop.as_deref())),
            ));
            if pending.len() >= worker_limit {
                flush_pending(&mut pending, &mut stopped, &mut errors);
            }
        }
        flush_pending(&mut pending, &mut stopped, &mut errors);
        TerminalShutdownSummary { stopped, errors }
    }
}

struct PtySessionRuntime {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    chunks: Arc<Mutex<VecDeque<TerminalChunk>>>,
    retained_bytes: Arc<AtomicUsize>,
    seq: Arc<AtomicU64>,
    started_at_ms: u64,
    last_activity_ms: Arc<AtomicU64>,
    runtime_output_seen: Arc<std::sync::atomic::AtomicBool>,
    eof_without_output: Arc<std::sync::atomic::AtomicBool>,
    launch_command: String,
    cwd: Option<String>,
}

impl PtySessionRuntime {
    fn spawn(key: &str, launch_command: &str, cwd: Option<&str>) -> Result<Self> {
        trace_terminal_event(
            "spawn",
            serde_json::json!({
                "path": key,
                "cwd": cwd,
                "launch_command": launch_command,
            }),
        );
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
        let retained_bytes = Arc::new(AtomicUsize::new(0));
        let seq = Arc::new(AtomicU64::new(0));
        let started_at_ms = now_millis();
        let last_activity_ms = Arc::new(AtomicU64::new(started_at_ms));
        let runtime_output_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let eof_without_output = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_chunks = Arc::clone(&chunks);
        let reader_retained_bytes = Arc::clone(&retained_bytes);
        let reader_seq = Arc::clone(&seq);
        let reader_activity = Arc::clone(&last_activity_ms);
        let reader_runtime_output_seen = Arc::clone(&runtime_output_seen);
        let reader_eof_without_output = Arc::clone(&eof_without_output);
        let key_label = key.to_string();
        let launch_command_label = launch_command.to_string();

        thread::Builder::new()
            .name(format!("pty-reader-{key}"))
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                let mut saw_any_output = false;
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(bytes) => {
                            let data = String::from_utf8_lossy(&buffer[..bytes]).to_string();
                            if data.is_empty() {
                                continue;
                            }
                            if !saw_any_output {
                                saw_any_output = true;
                                trace_terminal_event(
                                    "first_bytes",
                                    serde_json::json!({
                                        "path": key_label,
                                        "bytes": bytes,
                                        "launch_command": launch_command_label,
                                        "visible_text": terminal_chunk_has_visible_text(&data),
                                        "sample": truncate_terminal_trace_sample(&strip_terminal_control_sequences(&data)),
                                    }),
                                );
                            }
                            reader_runtime_output_seen.store(true, Ordering::SeqCst);
                            reader_activity.store(now_millis(), Ordering::SeqCst);
                            let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
                            let mut chunks = reader_chunks.lock().expect("pty chunk lock poisoned");
                            let mut retained = reader_retained_bytes.load(Ordering::SeqCst);
                            chunks.push_back(TerminalChunk {
                                seq: seq_value,
                                data,
                            });
                            retained = retained.saturating_add(chunks.back().map(|chunk| chunk.data.len()).unwrap_or(0));
                            trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
                            reader_retained_bytes.store(retained, Ordering::SeqCst);
                        }
                        Err(error) => {
                            if !saw_any_output {
                                trace_terminal_event(
                                    "reader_error_before_output",
                                    serde_json::json!({
                                        "path": key_label,
                                        "launch_command": launch_command_label,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                            reader_runtime_output_seen.store(true, Ordering::SeqCst);
                            reader_activity.store(now_millis(), Ordering::SeqCst);
                            let seq_value = reader_seq.fetch_add(1, Ordering::SeqCst) + 1;
                            let mut chunks = reader_chunks.lock().expect("pty chunk lock poisoned");
                            let mut retained = reader_retained_bytes.load(Ordering::SeqCst);
                            chunks.push_back(TerminalChunk {
                                seq: seq_value,
                                data: format!("\r\n[yggterm] terminal reader stopped for {key_label}: {error}\r\n"),
                            });
                            retained = retained.saturating_add(chunks.back().map(|chunk| chunk.data.len()).unwrap_or(0));
                            trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
                            reader_retained_bytes.store(retained, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                if !saw_any_output {
                    reader_eof_without_output.store(true, Ordering::SeqCst);
                    trace_terminal_event(
                        "eof_without_output",
                        serde_json::json!({
                            "path": key_label,
                            "launch_command": launch_command_label,
                        }),
                    );
                }
            })
            .context("spawning pty reader thread")?;

        Ok(Self {
            master: Arc::new(Mutex::new(pair.master)),
            writer: Arc::new(Mutex::new(writer)),
            child: Arc::new(Mutex::new(child)),
            chunks,
            retained_bytes,
            seq,
            started_at_ms,
            last_activity_ms,
            runtime_output_seen,
            eof_without_output,
            launch_command: launch_command.to_string(),
            cwd: cwd.map(|value| value.to_string()),
        })
    }

    fn matches_spec(&self, launch_command: &str, cwd: Option<&str>) -> bool {
        self.launch_command == launch_command && self.cwd.as_deref() == cwd
    }

    fn matches_remote_resume_spec(&self, cwd: Option<&str>) -> bool {
        self.cwd.as_deref() == cwd
            && launch_command_looks_like_remote_resume_attach(&self.launch_command)
    }

    fn is_running(&self) -> bool {
        let mut child = self.child.lock().expect("pty child lock poisoned");
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(_) => false,
        }
    }

    fn has_output(&self) -> bool {
        self.seq.load(Ordering::SeqCst) > 0
            || self.retained_bytes.load(Ordering::SeqCst) > 0
            || !self
                .chunks
                .lock()
                .expect("pty chunk lock poisoned")
                .is_empty()
    }

    fn has_runtime_output(&self) -> bool {
        self.runtime_output_seen.load(Ordering::SeqCst)
    }

    fn hit_eof_without_output(&self) -> bool {
        self.eof_without_output.load(Ordering::SeqCst)
    }

    fn age_ms(&self) -> u64 {
        now_millis().saturating_sub(self.started_at_ms)
    }

    fn snapshot(&self) -> String {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        chunks
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>()
    }

    fn read(&self, cursor: u64) -> TerminalReadResult {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let next_cursor = self.seq.load(Ordering::SeqCst);
        let chunks = if cursor == 0 {
            select_initial_attach_chunks(&chunks)
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
            running: self.is_running(),
            runtime_output_seen: self.has_runtime_output(),
            eof_without_output: self.eof_without_output.load(Ordering::SeqCst),
        }
    }

    fn buffer_usage(&self) -> (usize, usize) {
        let chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        (chunks.len(), self.retained_bytes.load(Ordering::SeqCst))
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

    fn seed_snapshot(&self, data: &str) {
        if data.is_empty() {
            return;
        }
        self.runtime_output_seen.store(true, Ordering::SeqCst);
        self.last_activity_ms.store(now_millis(), Ordering::SeqCst);
        let seq_value = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let mut retained = self.retained_bytes.load(Ordering::SeqCst);
        chunks.push_back(TerminalChunk {
            seq: seq_value,
            data: data.to_string(),
        });
        retained = retained.saturating_add(data.len());
        trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
        self.retained_bytes.store(retained, Ordering::SeqCst);
    }

    fn recent_activity(&self, within: Duration) -> bool {
        let now = now_millis();
        let last = self.last_activity_ms.load(Ordering::SeqCst);
        now.saturating_sub(last) <= within.as_millis() as u64
    }

    fn trim_idle_buffer(&self, within: Duration) -> usize {
        if self.recent_activity(within) {
            return 0;
        }
        let mut chunks = self.chunks.lock().expect("pty chunk lock poisoned");
        let mut retained = self.retained_bytes.load(Ordering::SeqCst);
        let before = retained;
        trim_chunk_buffer(
            &mut chunks,
            &mut retained,
            IDLE_TRIM_MAX_CHUNKS,
            IDLE_TRIM_MAX_BYTES,
        );
        self.retained_bytes.store(retained, Ordering::SeqCst);
        before.saturating_sub(retained)
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
            let normalized = command.trim();
            let (attempts, sleep_ms) = if normalized == "exit" {
                (2usize, 50u64)
            } else {
                (4usize, 75u64)
            };
            for _ in 0..attempts {
                if child
                    .try_wait()
                    .context("checking terminal exit state")?
                    .is_some()
                {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(sleep_ms));
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

fn truncate_terminal_trace_sample(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 180 {
        return trimmed.to_string();
    }
    trimmed.chars().take(180).collect::<String>()
}

fn trace_terminal_event(name: &str, payload: serde_json::Value) {
    if let Ok(home) = resolve_yggterm_home() {
        append_trace_event(&home, "server", "terminal_runtime", name, payload);
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

fn launch_command_looks_like_remote_resume_attach(launch_command: &str) -> bool {
    launch_command.contains("server'\\'' '\\''remote'\\'' '\\''resume-codex")
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

fn trim_chunk_buffer(
    chunks: &mut VecDeque<TerminalChunk>,
    retained_bytes: &mut usize,
    max_chunks: usize,
    max_bytes: usize,
) {
    while chunks.len() > max_chunks || *retained_bytes > max_bytes {
        let Some(chunk) = chunks.pop_front() else {
            *retained_bytes = 0;
            break;
        };
        *retained_bytes = retained_bytes.saturating_sub(chunk.data.len());
    }
}

fn select_initial_attach_chunks(chunks: &VecDeque<TerminalChunk>) -> Vec<TerminalChunk> {
    if chunks.is_empty() {
        return Vec::new();
    }

    let mut trailing_noise = Vec::new();
    let mut anchor_index = None;
    for (ix, chunk) in chunks.iter().enumerate().rev() {
        if terminal_chunk_has_visible_text(&chunk.data) {
            anchor_index = Some(ix);
            break;
        }
        if trailing_noise.len() < INITIAL_ATTACH_TRAILING_NOISE_CHUNKS {
            trailing_noise.push(ix);
        }
    }

    let Some(anchor_index) = anchor_index else {
        return select_initial_attach_tail(chunks, None);
    };

    let preserved_trailing = trailing_noise.into_iter().rev().collect::<Vec<_>>();
    let trailing_chunk_budget = preserved_trailing.len();
    let trailing_byte_budget = preserved_trailing
        .iter()
        .filter_map(|ix| chunks.get(*ix))
        .map(|chunk| chunk.data.len())
        .sum::<usize>();

    let available_chunk_budget =
        INITIAL_ATTACH_MAX_CHUNKS.saturating_sub(trailing_chunk_budget.max(1));
    let available_byte_budget = INITIAL_ATTACH_MAX_BYTES.saturating_sub(trailing_byte_budget);
    let leading = select_initial_attach_tail(
        chunks,
        Some((anchor_index, available_chunk_budget, available_byte_budget)),
    );

    let mut selected = leading;
    for ix in preserved_trailing {
        if let Some(chunk) = chunks.get(ix).cloned() {
            selected.push(chunk);
        }
    }
    selected
}

fn select_initial_attach_tail(
    chunks: &VecDeque<TerminalChunk>,
    anchor: Option<(usize, usize, usize)>,
) -> Vec<TerminalChunk> {
    let mut selected = Vec::new();
    let mut bytes = 0usize;
    let (limit_index, chunk_budget, byte_budget) = anchor.unwrap_or((
        chunks.len().saturating_sub(1),
        INITIAL_ATTACH_MAX_CHUNKS,
        INITIAL_ATTACH_MAX_BYTES,
    ));
    for (ix, chunk) in chunks.iter().enumerate().rev() {
        if ix > limit_index {
            continue;
        }
        let chunk_len = chunk.data.len();
        if !selected.is_empty()
            && (selected.len() >= chunk_budget || bytes.saturating_add(chunk_len) > byte_budget)
        {
            break;
        }
        bytes = bytes.saturating_add(chunk_len);
        selected.push(chunk.clone());
    }
    selected.reverse();
    selected
}

fn terminal_chunk_has_visible_text(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    stripped.chars().any(|ch| !ch.is_whitespace())
}

fn strip_terminal_control_sequences(input: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Normal,
        Escape,
        Csi,
        Osc,
        OscEscape,
        StringTerminator,
    }

    let mut state = State::Normal;
    let mut out = String::with_capacity(input.len());

    for ch in input.chars() {
        match state {
            State::Normal => {
                if ch == '\u{1b}' {
                    state = State::Escape;
                } else if !ch.is_control() || matches!(ch, '\n' | '\r' | '\t') {
                    out.push(ch);
                }
            }
            State::Escape => match ch {
                '[' => state = State::Csi,
                ']' => state = State::Osc,
                'P' | 'X' | '^' | '_' => state = State::StringTerminator,
                _ => state = State::Normal,
            },
            State::Csi => {
                if ('@'..='~').contains(&ch) {
                    state = State::Normal;
                }
            }
            State::Osc => match ch {
                '\u{7}' => state = State::Normal,
                '\u{1b}' => state = State::OscEscape,
                _ => {}
            },
            State::OscEscape => {
                state = if ch == '\\' {
                    State::Normal
                } else {
                    State::Osc
                };
            }
            State::StringTerminator => {
                if ch == '\u{1b}' {
                    state = State::OscEscape;
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_chunk_buffer_enforces_byte_budget() {
        let mut chunks = VecDeque::from([
            TerminalChunk {
                seq: 1,
                data: "a".repeat(900_000),
            },
            TerminalChunk {
                seq: 2,
                data: "b".repeat(900_000),
            },
            TerminalChunk {
                seq: 3,
                data: "c".repeat(900_000),
            },
        ]);
        let mut retained = 2_700_000usize;
        trim_chunk_buffer(&mut chunks, &mut retained, MAX_CHUNKS, MAX_BUFFER_BYTES);
        assert!(retained <= MAX_BUFFER_BYTES);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks.front().map(|chunk| chunk.seq), Some(2));
    }

    #[test]
    fn trim_chunk_buffer_enforces_idle_budget() {
        let mut chunks = VecDeque::from(
            (0..96)
                .map(|ix| TerminalChunk {
                    seq: ix,
                    data: "x".repeat(4096),
                })
                .collect::<Vec<_>>(),
        );
        let mut retained = 96 * 4096;
        trim_chunk_buffer(
            &mut chunks,
            &mut retained,
            IDLE_TRIM_MAX_CHUNKS,
            IDLE_TRIM_MAX_BYTES,
        );
        assert!(chunks.len() <= IDLE_TRIM_MAX_CHUNKS);
        assert!(retained <= IDLE_TRIM_MAX_BYTES);
    }

    #[test]
    fn initial_attach_selection_keeps_last_meaningful_surface_ahead_of_trailing_noise() {
        let mut chunks = VecDeque::new();
        chunks.push_back(TerminalChunk {
            seq: 1,
            data: "saved transcript line\n".to_string(),
        });
        chunks.push_back(TerminalChunk {
            seq: 2,
            data: "\u{1b}[2J\u{1b}[HOpenAI Codex (v0.118.0)\n/model to change\n".to_string(),
        });
        for seq in 3..260 {
            chunks.push_back(TerminalChunk {
                seq,
                data: "\u{1b}[20;3H \r \n".to_string(),
            });
        }

        let selected = select_initial_attach_chunks(&chunks);
        let combined = selected
            .iter()
            .map(|chunk| chunk.data.as_str())
            .collect::<String>();

        assert!(combined.contains("OpenAI Codex"));
    }

    #[test]
    fn terminal_chunk_visible_text_ignores_ansi_noise() {
        assert!(!terminal_chunk_has_visible_text(
            "\u{1b}[20;3H \r \n\u{1b}[K"
        ));
        assert!(terminal_chunk_has_visible_text(
            "\u{1b}[2J\u{1b}[HOpenAI Codex (v0.118.0)\n"
        ));
    }

    #[test]
    fn launch_command_detects_remote_resume_attach() {
        let launch_command = "ssh -tt jojo 'exec $HOME/.yggterm/bin/yggterm '\\''server'\\'' '\\''remote'\\'' '\\''resume-codex'\\'' '\\''019ce5d8-c94c-7b62-ae19-3818ae400b65'\\'' '\\''/home/pi'\\'''";

        assert!(launch_command_looks_like_remote_resume_attach(
            launch_command
        ));
        assert!(!launch_command_looks_like_remote_resume_attach(
            "bash -lc 'ls'"
        ));
    }
}
