//! The distro's virtual disk on the Windows side (`ext4.vhdx`) and its compaction.
//!
//! The VHDX grows as Linux writes but never shrinks when files are deleted. Compacting needs the
//! file released (`wsl --shutdown`) and admin rights: `Optimize-VHD` where the Hyper-V module
//! exists (Pro), otherwise `diskpart` (every edition, Home included). Both run in one elevated
//! PowerShell, so the user sees a single UAC prompt. The elevated script travels in memory
//! (`-EncodedCommand`) and diskpart reads its commands from stdin: no file in %TEMP% that another
//! process could swap while the UAC prompt waits.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;

use crate::wsl::{run, sh_in_distro, validate_distro_name};

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

/// Exit codes of [`compact_script`] when it succeeds (anything else is a failure code).
const DONE_OPTIMIZE_VHD: i32 = 100;
const DONE_DISKPART: i32 = 101;

/// The elevated script, run in memory (`wsl::run_elevated_powershell`): `Optimize-VHD` when the
/// Hyper-V module exists, else diskpart fed through **stdin** (no script file anywhere). The path
/// was validated (`validate_vhdx_path`) and `'` is doubled for the single-quoted strings.
pub fn compact_script(vhdx: &str) -> String {
    let quote = |s: &str| s.replace('\'', "''");
    format!(
        r#"$vhdx = '{path}'
try {{
  if (-not (Get-Command Optimize-VHD -ErrorAction SilentlyContinue)) {{ throw 'sem Hyper-V' }}
  Optimize-VHD -Path $vhdx -Mode Full -ErrorAction Stop
  exit {DONE_OPTIMIZE_VHD}
}} catch {{
  $commands = '{commands}'
  $commands | & "$env:SystemRoot\System32\diskpart.exe" | Out-Null
  if ($LASTEXITCODE -eq 0) {{ exit {DONE_DISKPART} }}
  exit $LASTEXITCODE
}}
"#,
        path = quote(vhdx),
        commands = quote(&diskpart_script(vhdx)),
    )
}

/// `Ok(method)` for the success codes of [`compact_script`], the user-facing error otherwise.
pub fn interpret_compact_exit(code: i32) -> Result<&'static str, String> {
    match code {
        DONE_OPTIMIZE_VHD => Ok("optimize-vhd"),
        DONE_DISKPART => Ok("diskpart"),
        crate::wsl::UAC_CANCELLED => Err(
            "A permissão de administrador foi negada (ou a janela do UAC foi fechada). O disco não foi alterado; o WSL só foi desligado.".into(),
        ),
        c => Err(format!("A compactação falhou (código {c}). O arquivo pode estar em uso: feche o Docker Desktop e tente de novo.")),
    }
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

    let code = crate::wsl::run_elevated_powershell(&compact_script(&before.path), COMPACT_TIMEOUT).await?;
    let method = interpret_compact_exit(code)?.to_string();
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
    fn elevated_script_stays_in_memory() {
        // Security audit (0.1.3): nothing the elevated process runs may come from a file in %TEMP%.
        let path = r"C:\Users\o'brien\AppData\Local\Packages\Canonical\LocalState\ext4.vhdx";
        let script = compact_script(path);
        assert!(script.contains(r"$vhdx = 'C:\Users\o''brien\AppData"), "quotes doubled for PowerShell");
        assert!(script.contains(r#"select vdisk file="C:\Users\o''brien"#), "diskpart commands inline");
        assert!(script.contains(r#"| & "$env:SystemRoot\System32\diskpart.exe""#), "diskpart by absolute path, fed by stdin");
        for forbidden in ["/s ", "-File", "Set-Content", "Out-File", "TEMP", ".ps1", ".txt"] {
            assert!(!script.contains(forbidden), "{forbidden} in the elevated script");
        }
        let launcher = crate::wsl::elevation_launcher(&script);
        assert!(launcher.contains("-Verb RunAs") && launcher.contains("'-EncodedCommand'") && launcher.contains("exit $p.ExitCode"));
        assert!(launcher.contains(r"System32\WindowsPowerShell\v1.0\powershell.exe'"), "absolute PowerShell path");
        assert!(!launcher.contains(".ps1") && !launcher.contains("'-File'") && !launcher.contains("-File "));
    }

    #[test]
    fn interprets_compaction_exit_codes() {
        assert_eq!(interpret_compact_exit(DONE_OPTIMIZE_VHD), Ok("optimize-vhd"));
        assert_eq!(interpret_compact_exit(DONE_DISKPART), Ok("diskpart"));
        assert!(interpret_compact_exit(crate::wsl::UAC_CANCELLED).unwrap_err().contains("negada"));
        assert!(interpret_compact_exit(0).unwrap_err().contains("código 0"), "0 is not a success code here");
        assert!(interpret_compact_exit(-2147024864).unwrap_err().contains("em uso"));
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
