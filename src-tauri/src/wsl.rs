//! All interaction with `wsl.exe` goes through this module.
//!
//! Rules (see CLAUDE.md):
//! - every process is spawned with `CREATE_NO_WINDOW` so no console flashes on screen;
//! - `WSL_UTF8=1` is always set, and output is still decoded defensively (UTF-16LE or UTF-8).

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const START_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum DistroState {
    Running,
    Stopped,
    Installing,
    Converting,
    Uninstalling,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Distro {
    pub name: String,
    pub state: DistroState,
    /// Raw state text as printed by wsl.exe (may be localized).
    pub state_label: String,
    /// WSL architecture version: 1 or 2.
    pub version: u8,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslInfo {
    /// e.g. "2.3.24.0". `None` on old inbox WSL builds without `--version`.
    pub wsl_version: Option<String>,
    pub kernel_version: Option<String>,
}

/// Keep-alive processes for distros started from the app. WSL2 stops a distro a few
/// seconds after its last process exits, so "start" holds a `cat` blocked on an open
/// stdin pipe (portable, unlike `sleep infinity` on busybox distros).
/// Children are `kill_on_drop`, so they die with the app.
#[derive(Default)]
pub struct KeepAlive(Mutex<HashMap<String, Child>>);

/// Any Windows console program (wsl.exe, .cmd launchers, wt.exe...) without a
/// flashing console window. Never use a bare `Command::new` for these.
pub(crate) fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`: always the system's own, never
/// a `powershell.exe` that happens to come first in PATH (or in the current directory).
pub(crate) fn powershell_path() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// `-EncodedCommand` payload: Base64 of the script in UTF-16LE.
pub fn encode_powershell(script: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Exit code [`run_elevated_powershell`] reports when the UAC prompt was refused or closed
/// (`ERROR_CANCELLED`).
pub const UAC_CANCELLED: i32 = 1223;

/// The non-elevated launcher: starts the elevated PowerShell with `inner` as `-EncodedCommand`
/// and exits with its exit code. Everything lives in the command line, in memory: no script or
/// result file in %TEMP% that another process could swap while the UAC prompt waits (TOCTOU).
pub fn elevation_launcher(inner: &str) -> String {
    let ps = powershell_path().display().to_string().replace('\'', "''");
    format!(
        "try {{ $p = Start-Process -FilePath '{ps}' -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{}' \
         -Verb RunAs -WindowStyle Hidden -PassThru -ErrorAction Stop; $null = $p.Handle; $p.WaitForExit(); exit $p.ExitCode }} \
         catch {{ exit {UAC_CANCELLED} }}",
        encode_powershell(inner)
    )
}

/// Runs `inner` elevated (one UAC prompt) and returns its exit code; [`UAC_CANCELLED`] when the
/// user said no. The elevated window is hidden; output can't cross the elevation boundary, so the
/// script reports through its exit code.
pub(crate) async fn run_elevated_powershell(inner: &str, timeout: Duration) -> Result<i32, String> {
    let mut cmd = hidden_command(powershell_path());
    cmd.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encode_powershell(&elevation_launcher(inner))])
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| "Tempo esgotado aguardando a tarefa de administrador".to_string())?
        .map_err(|e| format!("Falha ao executar o PowerShell: {e}"))?;
    Ok(output.status.code().unwrap_or(-1))
}

/// Base `wsl.exe` command: hidden console window + UTF-8 output.
pub fn wsl_command() -> Command {
    let mut cmd = hidden_command("wsl.exe");
    cmd.env("WSL_UTF8", "1").kill_on_drop(true);
    cmd
}

/// Decodes wsl.exe output, which is UTF-16LE unless `WSL_UTF8=1` was honored.
pub fn decode_output(bytes: &[u8]) -> String {
    let looks_utf16 = bytes.starts_with(&[0xFF, 0xFE])
        || (bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).take(64).all(|&b| b == 0));

    let text = if looks_utf16 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };

    text.trim_start_matches('\u{FEFF}').replace('\0', "")
}

/// Rejects names that could be parsed as wsl.exe flags or are otherwise invalid.
pub fn validate_distro_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(format!("Nome de distro inválido: {name:?}"))
    }
}

/// Runs a wsl.exe command with a timeout and returns decoded stdout.
pub(crate) async fn run(cmd: Command, timeout: Duration) -> Result<String, String> {
    run_raw(cmd, timeout, None).await.map(|bytes| decode_output(&bytes))
}

/// Most stdout (and stderr) kept from one command. File reads ask for far less (4 MB); this only
/// guards against a runaway command filling the app's memory.
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Reads a whole stream keeping at most `cap` bytes. The rest is still read (and dropped), so
/// the child never blocks on a full pipe.
pub(crate) async fn read_capped(mut reader: impl tokio::io::AsyncRead + Unpin, cap: usize) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(out);
        }
        let room = cap.saturating_sub(out.len());
        out.extend_from_slice(&chunk[..n.min(room)]);
    }
}

/// Like [`run`], but returns stdout bytes untouched (file contents) and optionally
/// feeds `stdin` to the process. Output is capped at [`MAX_OUTPUT_BYTES`] per stream.
pub(crate) async fn run_raw(
    mut cmd: Command,
    timeout: Duration,
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let task = async {
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd.spawn()?;
        let input = child.stdin.take();
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        // Written while the output is read: a big input never deadlocks against a full pipe.
        let write = async move {
            if let (Some(data), Some(mut pipe)) = (stdin, input) {
                use tokio::io::AsyncWriteExt;
                pipe.write_all(data).await?;
                // Dropping the pipe sends EOF.
            }
            Ok::<(), std::io::Error>(())
        };
        let out = async {
            match stdout {
                Some(s) => read_capped(s, MAX_OUTPUT_BYTES).await,
                None => Ok(Vec::new()),
            }
        };
        let err = async {
            match stderr {
                Some(s) => read_capped(s, MAX_OUTPUT_BYTES).await,
                None => Ok(Vec::new()),
            }
        };
        let (_, out, err) = tokio::try_join!(write, out, err)?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, out, err))
    };
    let (status, stdout, stderr) = tokio::time::timeout(timeout, task)
        .await
        .map_err(|_| "Tempo esgotado aguardando o wsl.exe".to_string())?
        .map_err(|e| format!("Falha ao executar wsl.exe: {e}"))?;

    if status.success() {
        Ok(stdout)
    } else {
        let stderr = decode_output(&stderr);
        let msg = if stderr.trim().is_empty() { decode_output(&stdout) } else { stderr };
        Err(msg.trim().to_string())
    }
}

fn parse_state(label: &str) -> DistroState {
    match label.to_ascii_lowercase().as_str() {
        "running" => DistroState::Running,
        "stopped" => DistroState::Stopped,
        "installing" => DistroState::Installing,
        "converting" => DistroState::Converting,
        "uninstalling" => DistroState::Uninstalling,
        // Localized builds: "Em execução", "Parado", "En cours d'exécution", ...
        l if l.contains("exec") || l.contains("ejec") || l.contains("lauf") => DistroState::Running,
        l if l.contains("parad") || l.contains("arr") || l.contains("beendet") => {
            DistroState::Stopped
        }
        _ => DistroState::Unknown,
    }
}

/// Parses `wsl --list --verbose`. The header is localized, so it is skipped by position;
/// each row is `[*] NAME STATE... VERSION` where STATE may contain spaces when localized.
pub fn parse_list_verbose(text: &str) -> Vec<Distro> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (is_default, rest) = match line.strip_prefix('*') {
                Some(rest) => (true, rest.trim_start()),
                None => (false, line),
            };
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.len() < 3 {
                return None;
            }
            let version = tokens[tokens.len() - 1].parse::<u8>().ok()?;
            let state_label = tokens[1..tokens.len() - 1].join(" ");
            Some(Distro {
                name: tokens[0].to_string(),
                state: parse_state(&state_label),
                state_label,
                version,
                is_default,
            })
        })
        .collect()
}

/// Extracts the first dotted version number found after a `:` on a line.
fn version_after_colon(line: &str) -> Option<String> {
    let value = line.split_once(':')?.1.trim();
    let token = value.split_whitespace().next()?;
    token
        .chars()
        .next()
        .filter(char::is_ascii_digit)
        .map(|_| token.to_string())
}

/// Parses `wsl --version`. Labels are localized, so rely on line order:
/// line 1 = WSL version, line 2 = kernel version.
pub fn parse_version(text: &str) -> WslInfo {
    let mut versions = text.lines().filter_map(version_after_colon);
    WslInfo {
        wsl_version: versions.next(),
        kernel_version: versions.next(),
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_distros() -> Result<Vec<Distro>, String> {
    let mut cmd = wsl_command();
    cmd.args(["--list", "--verbose"]);
    match run(cmd, DEFAULT_TIMEOUT).await {
        Ok(text) => Ok(parse_list_verbose(&text)),
        // wsl.exe exits non-zero when no distro is installed.
        Err(e) if e.contains("WSL_E_DEFAULT_DISTRO_NOT_FOUND") => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Whether `name` is currently running. Used to avoid `wsl -d` calls that would boot a
/// stopped distro as a side effect.
pub(crate) async fn is_running(name: &str) -> Result<bool, String> {
    Ok(list_distros()
        .await?
        .iter()
        .any(|d| d.name == name && d.state == DistroState::Running))
}

/// Validates the name and fails unless the distro is running (see CLAUDE.md 3.1).
pub(crate) async fn ensure_running(distro: &str) -> Result<(), String> {
    validate_distro_name(distro)?;
    if is_running(distro).await? {
        Ok(())
    } else {
        Err(format!("A distro {distro} não está em execução"))
    }
}

/// `wsl.exe -d <distro> [-u <user>] --exec sh -c <script> sh <args...>`.
/// Values reach the script as positional parameters ($1, $2...), never interpolated.
pub(crate) fn sh_in_distro(distro: &str, user: Option<&str>, script: &str, args: &[&str]) -> Command {
    let mut cmd = wsl_command();
    cmd.args(["-d", distro]);
    if let Some(user) = user {
        cmd.args(["-u", user]);
    }
    cmd.args(["--exec", "sh", "-c", script, "sh"]).args(args);
    cmd
}

#[tauri::command]
pub async fn get_wsl_info() -> Result<WslInfo, String> {
    let mut cmd = wsl_command();
    cmd.arg("--version");
    // Inbox WSL (pre-Store) does not support --version; report unknown instead of failing.
    Ok(run(cmd, DEFAULT_TIMEOUT)
        .await
        .map(|text| parse_version(&text))
        .unwrap_or(WslInfo {
            wsl_version: None,
            kernel_version: None,
        }))
}

#[tauri::command]
pub async fn start_distro(
    name: String,
    keep_alive: tauri::State<'_, KeepAlive>,
) -> Result<(), String> {
    start(&name, &keep_alive).await
}

#[tauri::command]
pub async fn shutdown_wsl(app: tauri::AppHandle) -> Result<(), String> {
    shutdown_all(&app).await
}

/// `wsl --shutdown` the clean way, from anywhere (Titlebar, tray, .wslconfig, VHDX compaction).
/// Emits `wsl-changed` so the window refreshes even when the action came from the tray.
pub(crate) async fn shutdown_all(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    // Close shells first so they exit cleanly instead of dying with the VM.
    app.state::<crate::terminal::TerminalManager>().close_all();
    // Running projects die with the VM: report them as stopped, not as failures.
    app.state::<crate::projects::ProjectRunner>().mark_all_stopping();
    let result = shutdown(&app.state::<KeepAlive>()).await;
    let _ = app.emit("wsl-changed", ());
    result
}

async fn start(name: &str, keep_alive: &KeepAlive) -> Result<(), String> {
    validate_distro_name(name)?;

    // Boot the distro and wait until it answers.
    let mut boot = wsl_command();
    boot.args(["-d", name, "--exec", "/bin/true"]);
    run(boot, START_TIMEOUT).await?;

    let mut children = keep_alive.0.lock().await;
    let alive = match children.get_mut(name) {
        Some(child) => matches!(child.try_wait(), Ok(None)),
        None => false,
    };
    if !alive {
        let mut hold = wsl_command();
        hold.args(["-d", name, "--exec", "cat"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = hold
            .spawn()
            .map_err(|e| format!("Falha ao manter a distro ativa: {e}"))?;
        children.insert(name.to_string(), child);
    }
    Ok(())
}

async fn shutdown(keep_alive: &KeepAlive) -> Result<(), String> {
    keep_alive.0.lock().await.clear();
    let mut cmd = wsl_command();
    cmd.arg("--shutdown");
    run(cmd, DEFAULT_TIMEOUT).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_verbose() {
        let text = "  NAME              STATE           VERSION\n\
                    * Ubuntu            Running         2\n  \
                    docker-desktop    Stopped         2\n";
        let distros = parse_list_verbose(text);
        assert_eq!(distros.len(), 2);
        assert_eq!(distros[0].name, "Ubuntu");
        assert!(distros[0].is_default);
        assert_eq!(distros[0].state, DistroState::Running);
        assert_eq!(distros[1].name, "docker-desktop");
        assert!(!distros[1].is_default);
        assert_eq!(distros[1].state, DistroState::Stopped);
        assert_eq!(distros[1].version, 2);
    }

    #[test]
    fn parses_localized_state() {
        let text = "  NOME    ESTADO        VERSÃO\n* Ubuntu  Em execução   2\n";
        let distros = parse_list_verbose(text);
        assert_eq!(distros[0].state, DistroState::Running);
        assert_eq!(distros[0].state_label, "Em execução");
    }

    #[test]
    fn decodes_utf16le() {
        let utf16: Vec<u8> = "  NAME\r\n* Ubuntu"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(decode_output(&utf16), "  NAME\r\n* Ubuntu");
        assert_eq!(decode_output(b"plain"), "plain");
    }

    #[test]
    fn parses_localized_version() {
        let text = "Versão do WSL: 2.3.24.0\nVersão do kernel: 5.15.153.1-2\nVersão do WSLg: 1.0.65\n";
        let info = parse_version(text);
        assert_eq!(info.wsl_version.as_deref(), Some("2.3.24.0"));
        assert_eq!(info.kernel_version.as_deref(), Some("5.15.153.1-2"));
    }

    #[test]
    fn encodes_powershell_commands() {
        // `powershell -EncodedCommand` wants Base64 of UTF-16LE.
        assert_eq!(encode_powershell("a"), "YQA=");
        assert_eq!(encode_powershell("exit 0"), "ZQB4AGkAdAAgADAA");
        assert_eq!(encode_powershell("ç"), "5wA=");
        assert_eq!(encode_powershell(""), "");
        assert!(powershell_path().ends_with(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
        assert!(powershell_path().is_absolute());
    }

    /// The real powershell.exe decodes our Base64 (accents, quotes, newlines) and its exit code
    /// comes back: the same path the elevated launcher uses, minus the UAC prompt.
    #[tokio::test]
    async fn encoded_command_round_trip() {
        let script = "$s = 'ação d''água'\nif ($s.Length -eq 11) { exit 42 } else { exit 7 }";
        let mut cmd = hidden_command(powershell_path());
        cmd.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encode_powershell(script)]);
        let status = cmd.output().await.expect("powershell").status;
        assert_eq!(status.code(), Some(42));
    }

    #[tokio::test]
    async fn output_is_capped_and_fully_drained() {
        // Security audit (0.1.3): a runaway command can't fill the app's memory.
        let data = vec![b'x'; 3 * 1024 * 1024];
        let kept = read_capped(data.as_slice(), 1024 * 1024).await.unwrap();
        assert_eq!(kept.len(), 1024 * 1024);
        assert_eq!(read_capped(&b"curto"[..], MAX_OUTPUT_BYTES).await.unwrap(), b"curto");
    }

    /// A real process writing more than the cap still finishes (the excess is drained, not left
    /// in the pipe) and run_raw returns exactly MAX_OUTPUT_BYTES.
    #[tokio::test]
    async fn run_raw_caps_a_real_process() {
        let mut cmd = hidden_command(powershell_path());
        let script = format!("[Console]::Out.Write('x' * {}); [Console]::Out.Flush()", MAX_OUTPUT_BYTES + 3 * 1024 * 1024);
        cmd.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encode_powershell(&script)]);
        let out = run_raw(cmd, Duration::from_secs(60), None).await.expect("finishes");
        assert_eq!(out.len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn rejects_flag_like_names() {
        assert!(validate_distro_name("Ubuntu-22.04").is_ok());
        assert!(validate_distro_name("--shutdown").is_err());
        assert!(validate_distro_name("a b").is_err());
    }

    // Live tests against the real wsl.exe. Run with:
    //   cargo test -- --ignored --nocapture --test-threads=1 live_
    // Set WSL_TEST_DISTRO to a stopped distro to exercise start.

    #[tokio::test]
    #[ignore]
    async fn live_list_and_info() {
        let distros = list_distros().await.expect("list_distros");
        println!("{distros:#?}");
        assert!(!distros.is_empty());
        assert_eq!(distros.iter().filter(|d| d.is_default).count(), 1);
        assert!(distros.iter().all(|d| d.state != DistroState::Unknown));
        let info = get_wsl_info().await.expect("get_wsl_info");
        println!("{info:#?}");
        assert!(info.wsl_version.is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn live_start_keeps_distro_running() {
        let name = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let keep_alive = KeepAlive::default();
        start(&name, &keep_alive).await.expect("start");

        // WSL2 would stop an idle distro within ~15s; it must still be running after 20s.
        tokio::time::sleep(Duration::from_secs(20)).await;
        let state = |list: Vec<Distro>| list.into_iter().find(|d| d.name == name).unwrap().state;
        assert_eq!(state(list_distros().await.unwrap()), DistroState::Running);

        // Dropping the keep-alive must let it stop again.
        drop(keep_alive);
        let mut stopped = false;
        for _ in 0..12 {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if state(list_distros().await.unwrap()) == DistroState::Stopped {
                stopped = true;
                break;
            }
        }
        println!("stopped after keep-alive drop: {stopped}");
    }

    #[tokio::test]
    #[ignore]
    async fn live_shutdown() {
        shutdown(&KeepAlive::default()).await.expect("shutdown");
        let distros = list_distros().await.unwrap();
        assert!(distros.iter().all(|d| d.state == DistroState::Stopped));
    }
}
