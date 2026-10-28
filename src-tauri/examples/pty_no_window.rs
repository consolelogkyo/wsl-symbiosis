//! Proof that terminal sessions never flash a console window: built as a GUI-subsystem
//! binary (no console at all, exactly like the release app), it opens real PTY sessions with
//! the app's `TerminalManager`, runs a command in each and writes the result to a log file.
//! Run it while watching visible windows (see docs/FASE-5-RELATORIO.md):
//!
//!   cargo run --example pty_no_window -- Ubuntu C:\path\to\log.txt
#![windows_subsystem = "windows"]

use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use wsl_symbiosis_lib::terminal::{EventSink, TerminalEvent, TerminalManager};

const SESSIONS: usize = 3;

fn main() {
    let mut args = std::env::args().skip(1);
    let distro = args.next().unwrap_or_else(|| "Ubuntu".into());
    let log_path = args.next().expect("log path");
    let mut log = String::new();

    let manager = TerminalManager::default();
    for n in 1..=SESSIONS {
        let output = Arc::new(Mutex::new(String::new()));
        let (exit_tx, exit_rx) = mpsc::channel();
        let exit_tx = Mutex::new(exit_tx);
        let (out, m) = (Arc::clone(&output), manager.clone());
        let sink: EventSink = Arc::new(move |event| match event {
            TerminalEvent::Output(p) => {
                // Answer ConPTY's cursor position request, as xterm.js does in the app.
                if p.data.contains("\x1b[6n") {
                    let _ = m.write(&p.session_id, "\x1b[1;1R");
                }
                out.lock().unwrap().push_str(&p.data);
            }
            TerminalEvent::Exit(p) => {
                let _ = exit_tx.lock().unwrap().send(p.code);
            }
        });

        let started = Instant::now();
        let info = match manager.create(&distro, Some("/tmp"), 100, 30, sink) {
            Ok(info) => info,
            Err(e) => {
                log.push_str(&format!("session {n}: CREATE FAILED {e}\n"));
                continue;
            }
        };
        let _ = manager.write(&info.session_id, &format!("printf 'NOWINDOW_%s_%s\\n' {n} OK; exit\r"));
        let code = exit_rx.recv_timeout(Duration::from_secs(20)).ok().flatten();
        let ok = output.lock().unwrap().contains(&format!("NOWINDOW_{n}_OK"));
        log.push_str(&format!(
            "session {n}: pid={:?} output_ok={ok} exit={code:?} in {} ms\n",
            info.pid,
            started.elapsed().as_millis()
        ));
    }
    log.push_str(&format!("remaining sessions: {}\n", manager.count()));
    std::fs::write(&log_path, log).expect("write log");
}
