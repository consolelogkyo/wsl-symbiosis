//! Interactive terminal sessions: `wsl.exe` running inside a Windows pseudoconsole (ConPTY,
//! via `portable-pty`), streamed to xterm.js through Tauri events.
//!
//! Exception to the CREATE_NO_WINDOW rule (CLAUDE.md): the child is attached to the
//! pseudoconsole (`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`), so it never gets a console window
//! of its own, even from the GUI-subsystem release binary. Verified by
//! `examples/pty_no_window.rs`.
//!
//! Lifecycle per session:
//! - reader thread: PTY output → UTF-8 (multi-byte chars split across reads are carried
//!   over) → `terminal-output` events;
//! - waiter thread: waits for the child, lets the reader drain, then drops the PTY (ConPTY
//!   only signals EOF once the pseudoconsole is closed) and emits `terminal-exit`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

use crate::explorer::normalize_linux_path;
use crate::wsl::validate_distro_name;

const MAX_SESSIONS: usize = 16;
const READ_BUFFER: usize = 16 * 1024;
/// Time for the reader to drain the last output after the child exits.
const DRAIN_DELAY: Duration = Duration::from_millis(200);
const READER_JOIN_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Events (mirrored in src/lib/terminal.ts)
// ---------------------------------------------------------------------------

pub const OUTPUT_EVENT: &str = "terminal-output";
pub const EXIT_EVENT: &str = "terminal-exit";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputPayload {
    pub session_id: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitPayload {
    pub session_id: String,
    /// `None` when the process was killed / its status is unknown.
    pub code: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Output(TerminalOutputPayload),
    Exit(TerminalExitPayload),
}

/// Where session events go: Tauri events in the app, a channel in tests.
pub type EventSink = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

// ---------------------------------------------------------------------------
// UTF-8 streaming
// ---------------------------------------------------------------------------

/// Decodes a byte stream whose reads may split multi-byte characters. Incomplete trailing
/// sequences are kept for the next chunk; invalid bytes become U+FFFD.
#[derive(Default)]
pub struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::with_capacity(self.pending.len());
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    rest = &[];
                    break;
                }
                Err(e) => {
                    let (valid, after) = rest.split_at(e.valid_up_to());
                    out.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match e.error_len() {
                        Some(len) => {
                            out.push('\u{FFFD}');
                            rest = &after[len..];
                        }
                        // Incomplete sequence at the end: wait for the next read.
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        out
    }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

struct PtySession {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    /// Windows PID of the `wsl.exe` running in the pseudoconsole.
    pub pid: Option<u32>,
}

/// All open sessions, shared with each session's waiter thread (which removes it on exit).
#[derive(Default, Clone)]
pub struct TerminalManager {
    sessions: Arc<Mutex<HashMap<String, Arc<PtySession>>>>,
    counter: Arc<AtomicU64>,
}

pub fn clamp_size(cols: u16, rows: u16) -> PtySize {
    PtySize { cols: cols.clamp(2, 1000), rows: rows.clamp(1, 500), pixel_width: 0, pixel_height: 0 }
}

/// Appends `TERM`/`COLORTERM` to WSLENV so they cross from Windows into Linux.
pub fn merge_wslenv(existing: Option<&str>) -> String {
    let mut parts: Vec<&str> = existing.unwrap_or("").split(':').filter(|p| !p.is_empty()).collect();
    for var in ["TERM", "COLORTERM"] {
        if !parts.iter().any(|p| p.split('/').next() == Some(var)) {
            parts.push(var);
        }
    }
    parts.join(":")
}

impl TerminalManager {
    fn get(&self, id: &str) -> Result<Arc<PtySession>, String> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| format!("Sessão de terminal não encontrada: {id}"))
    }

    pub fn count(&self) -> usize {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Opens `wsl.exe -d <distro> --cd <cwd|~>` in a new pseudoconsole. Unlike background
    /// probes, this may boot a stopped distro: the user explicitly asked for a shell.
    pub fn create(
        &self,
        distro: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        sink: EventSink,
    ) -> Result<SessionInfo, String> {
        validate_distro_name(distro)?;
        let cwd = cwd.map(normalize_linux_path).transpose()?;
        if self.count() >= MAX_SESSIONS {
            return Err(format!("Limite de {MAX_SESSIONS} terminais abertos"));
        }

        let pair = native_pty_system()
            .openpty(clamp_size(cols, rows))
            .map_err(|e| format!("Falha ao criar o pseudoconsole: {e}"))?;

        let mut cmd = CommandBuilder::new("wsl.exe");
        cmd.args(["-d", distro, "--cd", cwd.as_deref().unwrap_or("~")]);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("WSLENV", merge_wslenv(std::env::var("WSLENV").ok().as_deref()));

        let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("Falha ao iniciar o wsl.exe: {e}"))?;
        drop(pair.slave);
        let pid = child.process_id();
        let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let killer = child.clone_killer();

        let id = format!("pty-{}", self.counter.fetch_add(1, Ordering::Relaxed) + 1);
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(
            id.clone(),
            Arc::new(PtySession { master: Mutex::new(pair.master), writer: Mutex::new(writer), killer: Mutex::new(killer) }),
        );

        let (done_tx, done_rx) = mpsc::channel::<()>();
        spawn_reader(id.clone(), reader, sink.clone(), done_tx);

        let sessions = Arc::clone(&self.sessions);
        let session_id = id.clone();
        std::thread::Builder::new()
            .name(format!("{id}-waiter"))
            .spawn(move || {
                let code = child.wait().ok().map(|s| s.exit_code());
                std::thread::sleep(DRAIN_DELAY);
                // Dropping the session closes the pseudoconsole, which unblocks the reader.
                sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&session_id);
                let _ = done_rx.recv_timeout(READER_JOIN_TIMEOUT);
                sink(TerminalEvent::Exit(TerminalExitPayload { session_id, code }));
            })
            .map_err(|e| e.to_string())?;

        Ok(SessionInfo { session_id: id, pid })
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let session = self.get(id)?;
        let mut writer = session.writer.lock().unwrap_or_else(|e| e.into_inner());
        writer.write_all(data.as_bytes()).and_then(|_| writer.flush()).map_err(|e| format!("Falha ao escrever no terminal: {e}"))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let session = self.get(id)?;
        let master = session.master.lock().unwrap_or_else(|e| e.into_inner());
        master.resize(clamp_size(cols, rows)).map_err(|e| format!("Falha ao redimensionar: {e}"))
    }

    /// Kills the process; its waiter thread then closes the PTY and emits `terminal-exit`.
    pub fn close(&self, id: &str) -> Result<(), String> {
        let session = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let session = session.ok_or_else(|| format!("Sessão de terminal não encontrada: {id}"))?;
        let _ = session.killer.lock().unwrap_or_else(|e| e.into_inner()).kill();
        Ok(())
    }

    pub fn close_all(&self) {
        let ids: Vec<String> = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect();
        for id in ids {
            let _ = self.close(&id);
        }
    }
}

fn spawn_reader(id: String, mut reader: Box<dyn Read + Send>, sink: EventSink, done: mpsc::Sender<()>) {
    let _ = std::thread::Builder::new().name(format!("{id}-reader")).spawn(move || {
        let mut buf = vec![0u8; READ_BUFFER];
        let mut decoder = Utf8Stream::default();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = decoder.push(&buf[..n]);
                    if !data.is_empty() {
                        sink(TerminalEvent::Output(TerminalOutputPayload { session_id: id.clone(), data }));
                    }
                }
            }
        }
        let _ = done.send(());
    });
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn create_terminal_session(
    app: tauri::AppHandle,
    manager: tauri::State<'_, TerminalManager>,
    distro: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    use tauri::Emitter;
    let sink: EventSink = Arc::new(move |event| {
        let _ = match event {
            TerminalEvent::Output(p) => app.emit(OUTPUT_EVENT, p),
            TerminalEvent::Exit(p) => app.emit(EXIT_EVENT, p),
        };
    });
    manager.create(&distro, cwd.as_deref(), cols, rows, sink).map(|s| s.session_id)
}

#[tauri::command]
pub async fn write_terminal(manager: tauri::State<'_, TerminalManager>, session_id: String, data: String) -> Result<(), String> {
    manager.write(&session_id, &data)
}

#[tauri::command]
pub async fn resize_terminal(
    manager: tauri::State<'_, TerminalManager>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    manager.resize(&session_id, cols, rows)
}

#[tauri::command]
pub async fn close_terminal_session(manager: tauri::State<'_, TerminalManager>, session_id: String) -> Result<(), String> {
    manager.close(&session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn utf8_stream_handles_split_chars() {
        let mut d = Utf8Stream::default();
        let bytes = "olá ✓ 🚀".as_bytes();
        // Split inside 'á' (2 bytes), '✓' (3 bytes) and '🚀' (4 bytes).
        let mut out = String::new();
        for chunk in [&bytes[..3], &bytes[3..7], &bytes[7..10], &bytes[10..]] {
            out.push_str(&d.push(chunk));
        }
        assert_eq!(out, "olá ✓ 🚀");
        assert!(d.pending.is_empty());
    }

    #[test]
    fn utf8_stream_replaces_invalid_bytes() {
        let mut d = Utf8Stream::default();
        assert_eq!(d.push(b"a\xffb"), "a\u{FFFD}b");
        // Incomplete tail is held back, not replaced.
        assert_eq!(d.push(b"c\xe2\x9c"), "c");
        assert_eq!(d.push(b"\x93"), "✓");
    }

    #[test]
    fn clamps_sizes() {
        let s = clamp_size(0, 0);
        assert_eq!((s.cols, s.rows), (2, 1));
        let s = clamp_size(5000, 5000);
        assert_eq!((s.cols, s.rows), (1000, 500));
        let s = clamp_size(120, 30);
        assert_eq!((s.cols, s.rows), (120, 30));
    }

    #[test]
    fn merges_wslenv() {
        assert_eq!(merge_wslenv(None), "TERM:COLORTERM");
        assert_eq!(merge_wslenv(Some("USERPROFILE/p:")), "USERPROFILE/p:TERM:COLORTERM");
        assert_eq!(merge_wslenv(Some("TERM/u:FOO")), "TERM/u:FOO:COLORTERM");
    }

    #[test]
    fn rejects_bad_input_before_spawning() {
        let m = TerminalManager::default();
        let sink: EventSink = Arc::new(|_| {});
        assert!(m.create("--shutdown", None, 80, 24, sink.clone()).is_err());
        assert!(m.create("Ubuntu", Some("relative/path"), 80, 24, sink.clone()).is_err());
        assert!(m.create("Ubuntu", Some("/tmp\n; rm -rf ~"), 80, 24, sink).is_err());
        assert_eq!(m.count(), 0);
        assert!(m.write("pty-404", "ls\r").is_err());
        assert!(m.resize("pty-404", 80, 24).is_err());
        assert!(m.close("pty-404").is_err());
    }

    // ---- Live tests (real ConPTY + wsl.exe). Run with:
    //   WSL_TEST_DISTRO=Ubuntu cargo test --lib -- --ignored --nocapture --test-threads=1 terminal::tests::live_

    /// Collects output and answers ConPTY's cursor position request (ESC[6n), which xterm.js
    /// does on its own in the app.
    struct Harness {
        manager: TerminalManager,
        output: Arc<Mutex<String>>,
        exit: mpsc::Receiver<TerminalExitPayload>,
        info: SessionInfo,
    }

    impl Harness {
        fn start(distro: &str, cwd: Option<&str>) -> Harness {
            let manager = TerminalManager::default();
            let output = Arc::new(Mutex::new(String::new()));
            let (exit_tx, exit) = mpsc::channel();
            let exit_tx = Mutex::new(exit_tx);
            let (out, m) = (Arc::clone(&output), manager.clone());
            let answered = Arc::new(Mutex::new(false));
            let sink: EventSink = Arc::new(move |event| match event {
                TerminalEvent::Output(p) => {
                    out.lock().unwrap().push_str(&p.data);
                    if p.data.contains("\x1b[6n") && !*answered.lock().unwrap() {
                        *answered.lock().unwrap() = true;
                        let _ = m.write(&p.session_id, "\x1b[1;1R");
                    }
                }
                TerminalEvent::Exit(p) => {
                    let _ = exit_tx.lock().unwrap().send(p);
                }
            });
            let info = manager.create(distro, cwd, 120, 30, sink).expect("create session");
            Harness { manager, output, exit, info }
        }

        fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
            let start = Instant::now();
            while start.elapsed() < timeout {
                if self.output.lock().unwrap().contains(needle) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        }
    }

    fn windows_pid_alive(pid: u32) -> bool {
        let mut sys = sysinfo::System::new();
        let pid = sysinfo::Pid::from_u32(pid);
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        sys.process(pid).is_some()
    }

    #[test]
    #[ignore]
    fn live_pty_lifecycle() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let h = Harness::start(&distro, Some("/tmp"));
        let pid = h.info.pid.expect("wsl.exe pid");
        assert!(windows_pid_alive(pid));
        h.manager.resize(&h.info.session_id, 100, 40).expect("resize");

        // The typed text contains `%s`, so seeing the joined word proves the shell ran it.
        h.manager
            .write(&h.info.session_id, "printf 'SYMBIOSIS_PTY_%s CWD=%s\\n' OK \"$PWD\"\r")
            .expect("write");
        assert!(h.wait_for("SYMBIOSIS_PTY_OK CWD=/tmp", Duration::from_secs(20)), "output: {:?}", h.output.lock().unwrap());

        h.manager.write(&h.info.session_id, "exit\r").expect("write exit");
        let exit = h.exit.recv_timeout(Duration::from_secs(15)).expect("terminal-exit event");
        println!("exit = {exit:?}");
        assert_eq!(exit.code, Some(0));
        assert_eq!(h.manager.count(), 0, "session must be removed");
        std::thread::sleep(Duration::from_millis(500));
        assert!(!windows_pid_alive(pid), "wsl.exe {pid} leaked");
    }

    #[test]
    #[ignore]
    fn live_pty_close_kills_process() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let h = Harness::start(&distro, None);
        let pid = h.info.pid.expect("wsl.exe pid");
        h.manager.write(&h.info.session_id, "printf 'READY_%s\\n' 1\r").unwrap();
        assert!(h.wait_for("READY_1", Duration::from_secs(20)));

        h.manager.close(&h.info.session_id).expect("close");
        let exit = h.exit.recv_timeout(Duration::from_secs(15)).expect("terminal-exit after close");
        println!("exit after close = {exit:?}");
        assert!(h.manager.write(&h.info.session_id, "x").is_err(), "closed session must reject writes");
        std::thread::sleep(Duration::from_millis(500));
        assert!(!windows_pid_alive(pid), "wsl.exe {pid} leaked after close");
    }
}
