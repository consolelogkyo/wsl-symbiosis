//! The distro's virtual disk on the Windows side (`ext4.vhdx`) and its compaction.
//!
//! The VHDX grows as Linux writes but never shrinks when files are deleted. Compacting needs the
//! file released (`wsl --shutdown`) and admin rights: `Optimize-VHD` where the Hyper-V module
//! exists (Pro), otherwise `diskpart` (every edition, Home included). Both run in one elevated
//! PowerShell, so the user sees a single UAC prompt.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;

use crate::wsl::{hidden_command, run, sh_in_distro, validate_distro_name};

/// Big disks take minutes; the UAC prompt also waits for the user.
const COMPACT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const TRIM_TIMEOUT: Duration = Duration::from_secs(120);
/// The VM process lingers a moment after `wsl --shutdown` and keeps the file locked.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(20);

static COMPACTING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VhdxInfo {
    pub path: String,
    /// What the file takes on the Windows disk, in bytes.
    pub file_size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VhdxCompactResult {
    pub path: String,
    pub size_before: u64,
    pub size_after: u64,
    pub reclaimed_bytes: u64,
    /// `optimize-vhd` or `diskpart`.
    pub method: String,
    /// `fstrim` ran first (it marks the blocks Linux freed as reclaimable).
    pub trimmed: bool,
}

// ---------------------------------------------------------------------------
// Locate
// ---------------------------------------------------------------------------

#[cfg(windows)]
const LXSS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

/// `BasePath` may carry the verbatim prefix (`\\?\C:\Users\...`).
fn strip_verbatim(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

/// `HKCU\...\Lxss\{guid}`: `DistributionName`, `BasePath` and (newer WSL) `VhdFileName`.
#[cfg(windows)]
fn registry_vhdx(distro: &str) -> Result<PathBuf, String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let lxss = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(LXSS_KEY)
        .map_err(|_| "O WSL não tem distros registradas para este usuário".to_string())?;
    for id in lxss.enum_keys().flatten() {
        let Ok(key) = lxss.open_subkey(&id) else { continue };
        let name: String = key.get_value("DistributionName").unwrap_or_default();
        if !name.eq_ignore_ascii_case(distro) {
            continue;
        }
        let base: String = key
            .get_value("BasePath")
            .map_err(|_| format!("{distro} não tem BasePath no registro do WSL"))?;
        let file: String = key.get_value("VhdFileName").unwrap_or_else(|_| "ext4.vhdx".into());
        return Ok(Path::new(strip_verbatim(&base)).join(file));
    }
    Err(format!("{distro} não foi encontrada no registro do WSL"))
}

#[cfg(not(windows))]
fn registry_vhdx(distro: &str) -> Result<PathBuf, String> {
    Err(format!("{distro}: VHDX só existe no Windows"))
}

/// The distro's VHDX and its size. Fails for WSL1 distros (a folder, no virtual disk).
pub fn locate(distro: &str) -> Result<VhdxInfo, String> {
    validate_distro_name(distro)?;
    let path = registry_vhdx(distro)?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(VhdxInfo { path: path.display().to_string(), file_size: meta.len() })
}

// ---------------------------------------------------------------------------
// Compact
// ---------------------------------------------------------------------------

/// The path goes into a diskpart script (`file="..."`) and a PowerShell argument: only a plain
/// local `X:\...\*.vhdx` is accepted.
pub fn validate_vhdx_path(path: &str) -> Result<(), String> {
    let bytes = path.as_bytes();
    let drive = bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !drive {
        return Err(format!("Caminho do disco virtual inesperado: {path}"));
    }
    if path.chars().any(|c| c == '"' || c.is_control()) {
        return Err(format!("Caminho do disco virtual com caracteres inválidos: {path}"));
    }
    let vhdx = Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("vhdx"));
    if !vhdx || path.split('\\').any(|part| part == "..") {
        return Err(format!("Não é um arquivo .vhdx: {path}"));
    }
    Ok(())
}

/// Read-only attach is enough for `compact vdisk` and never lets Windows mount the ext4 volume.
pub fn diskpart_script(vhdx: &str) -> String {
    format!("select vdisk file=\"{vhdx}\"\r\nattach vdisk readonly\r\ncompact vdisk\r\ndetach vdisk\r\nexit\r\n")
}

/// Runs elevated. First line of `$out`: `<method> <exit code>`; then diskpart's output.
const COMPACT_PS1: &str = r#"param($vhdx, $script, $out)
$method = 'optimize-vhd'
$code = 0
$log = ''
try {
  if (-not (Get-Command Optimize-VHD -ErrorAction SilentlyContinue)) { throw 'sem Hyper-V' }
  Optimize-VHD -Path $vhdx -Mode Full -ErrorAction Stop
} catch {
  $method = 'diskpart'
  $log = (& diskpart.exe /s $script 2>&1 | Out-String)
  $code = $LASTEXITCODE
}
"$method $code`r`n$log" | Set-Content -Path $out -Encoding utf8
"#;

/// Parses the first line of the elevated script's report.
fn parse_report(text: &str) -> Option<(String, i64, String)> {
    let text = text.trim_start_matches('\u{FEFF}');
    let (head, log) = text.split_once('\n').unwrap_or((text, ""));
    let (method, code) = head.trim().split_once(' ')?;
    if !matches!(method, "optimize-vhd" | "diskpart") {
        return None;
    }
    Some((method.to_string(), code.trim().parse().ok()?, log.trim().to_string()))
}

/// Clears the flag even when the compaction fails or panics.
struct CompactGuard;

impl Drop for CompactGuard {
    fn drop(&mut self) {
        COMPACTING.store(false, Ordering::SeqCst);
    }
}

async fn wait_vm_released() {
    let deadline = tokio::time::Instant::now() + RELEASE_TIMEOUT;
    while crate::metrics::vm_running() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // The file handle goes a beat after the process.
    tokio::time::sleep(Duration::from_secs(1)).await;
}

async fn run_elevated_compaction(vhdx: &str) -> Result<(String, i64, String), String> {
    let dir = std::env::temp_dir().join(format!("wsl-symbiosis-vhdx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ps1 = dir.join("compact.ps1");
    let script = dir.join("compact.txt");
    let out = dir.join("result.txt");
    let _ = std::fs::remove_file(&out);
    std::fs::write(&ps1, COMPACT_PS1).map_err(|e| e.to_string())?;
    std::fs::write(&script, diskpart_script(vhdx)).map_err(|e| e.to_string())?;

    // Windows paths can't contain `"`; `'` is doubled for the PowerShell single-quoted string.
    let inner = format!(
        "-NoProfile -ExecutionPolicy Bypass -File \"{}\" \"{vhdx}\" \"{}\" \"{}\"",
        ps1.display(),
        script.display(),
        out.display()
    );
    let command = format!(
        "Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '{}'",
        inner.replace('\'', "''")
    );
    let mut cmd = hidden_command("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &command]);
    let output = tokio::time::timeout(COMPACT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "Tempo esgotado compactando o disco virtual".to_string())?
        .map_err(|e| format!("Falha ao executar o PowerShell: {e}"))?;
    let report = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    match parse_report(&report) {
        Some(parsed) => Ok(parsed),
        None if !output.status.success() || report.trim().is_empty() => Err(
            "A permissão de administrador foi negada (ou a janela do UAC foi fechada). O disco não foi alterado; o WSL só foi desligado."
                .into(),
        ),
        None => Err(format!("Resposta inesperada da compactação: {}", report.trim())),
    }
}

pub(crate) async fn compact(app: &AppHandle, distro: &str) -> Result<VhdxCompactResult, String> {
    validate_distro_name(distro)?;
    if COMPACTING.swap(true, Ordering::SeqCst) {
        return Err("Já existe uma compactação em andamento".into());
    }
    let _guard = CompactGuard;

    let before = locate(distro)?;
    validate_vhdx_path(&before.path)?;

    // TRIM first: ext4 tells the virtual disk which blocks it freed, otherwise there is little
    // to reclaim. Best effort (busybox distros may lack fstrim); boots a stopped distro, which is
    // fine here: the user asked for this and the shutdown below follows.
    let trim = sh_in_distro(distro, Some("root"), "fstrim / >/dev/null 2>&1", &[]);
    let trimmed = run(trim, TRIM_TIMEOUT).await.is_ok();

    crate::wsl::shutdown_all(app).await?;
    wait_vm_released().await;

    let (method, code, log) = run_elevated_compaction(&before.path).await?;
    if code != 0 {
        let detail = if log.is_empty() { String::new() } else { format!("\n{log}") };
        return Err(format!("O {method} falhou (código {code}). O arquivo pode estar em uso: feche o Docker Desktop e tente de novo.{detail}"));
    }
    let after = locate(distro)?;
    Ok(VhdxCompactResult {
        path: before.path,
        size_before: before.file_size,
        size_after: after.file_size,
        reclaimed_bytes: before.file_size.saturating_sub(after.file_size),
        method,
        trimmed,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn compact_distro_vhdx(app: AppHandle, distro: String) -> Result<VhdxCompactResult, String> {
    compact(&app, &distro).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_vhdx_paths() {
        for ok in [
            r"C:\Users\kyo\AppData\Local\Packages\CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc\LocalState\ext4.vhdx",
            r"D:\WSL\Ubuntu Dev\EXT4.VHDX",
            r"C:\Users\o'brien\wsl\ext4.vhdx",
        ] {
            assert!(validate_vhdx_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            r"\\server\share\ext4.vhdx",
            r"\\?\C:\wsl\ext4.vhdx",
            r"wsl\ext4.vhdx",
            r"C:\wsl\ext4.vhd",
            r"C:\wsl\ext4.vhdx.bak",
            "C:\\wsl\\a\"b.vhdx",
            "C:\\wsl\\ext4.vhdx\r\ncompact vdisk",
            r"C:\wsl\..\Windows\x.vhdx",
            "",
        ] {
            assert!(validate_vhdx_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn builds_diskpart_script() {
        let s = diskpart_script(r"C:\WSL\Ubuntu Dev\ext4.vhdx");
        let lines: Vec<&str> = s.split("\r\n").collect();
        assert_eq!(
            lines,
            [
                r#"select vdisk file="C:\WSL\Ubuntu Dev\ext4.vhdx""#,
                "attach vdisk readonly",
                "compact vdisk",
                "detach vdisk",
                "exit",
                "",
            ]
        );
    }

    #[test]
    fn strips_verbatim_prefix() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\kyo\wsl"), r"C:\Users\kyo\wsl");
        assert_eq!(strip_verbatim(r"C:\wsl"), r"C:\wsl");
    }

    #[test]
    fn parses_elevated_report() {
        assert_eq!(parse_report("\u{FEFF}optimize-vhd 0\r\n"), Some(("optimize-vhd".into(), 0, String::new())));
        let (method, code, log) = parse_report("diskpart 0\r\nDiskPart concluiu a compactação.\r\n").unwrap();
        assert_eq!((method.as_str(), code), ("diskpart", 0));
        assert!(log.contains("compacta"));
        assert_eq!(parse_report("diskpart -2147024864\r\nem uso").map(|r| r.1), Some(-2147024864));
        assert_eq!(parse_report(""), None);
        assert_eq!(parse_report("rm 0"), None);
    }

    /// Read-only: finds this machine's VHDX through the registry.
    /// `WSL_TEST_DISTRO=Ubuntu cargo test --lib live_vhdx -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_vhdx_locate() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let info = locate(&distro).expect("locate");
        println!("{info:#?}");
        validate_vhdx_path(&info.path).expect("path accepted by the compactor");
        assert!(info.file_size > 0);
    }
}
