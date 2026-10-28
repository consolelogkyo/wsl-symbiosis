//! Distro backups: `wsl --export` to a `.tar` on Windows, and `wsl --import` to restore it (or to
//! clone a distro under a new name).
//!
//! Both run through wsl.exe itself (CREATE_NO_WINDOW). Exporting a big distro takes minutes and
//! WSL reports no usable progress, so the UI shows elapsed time. After an import the distro would
//! log in as root (the Store keeps the default user outside the distro, in the registry): the
//! first regular user becomes the default in `/etc/wsl.conf`, unless the backup already says so.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::provisioner::{is_system_distro, set_wsl_conf_default_user};
use crate::wsl::{list_distros, run, run_raw, sh_in_distro, validate_distro_name, wsl_command};

/// A 100 GB distro on a slow disk.
const BACKUP_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);
const SHORT_TIMEOUT: Duration = Duration::from_secs(60);

static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub file: String,
    pub size: u64,
    pub seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub distro: String,
    /// The user the restored distro opens with (`None` = root: no regular user in the backup).
    pub default_user: Option<String>,
    pub seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupDefaults {
    /// Suggested folder for new backups (`%USERPROFILE%\Documents\WSL Backups`).
    pub backup_dir: String,
    /// Where restored distros go (`%LOCALAPPDATA%\WSL`); the UI appends the distro name.
    pub install_root: String,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A local `X:\...` path without control characters or `..` (it goes to wsl.exe as an argument).
fn validate_windows_path(path: &str, what: &str) -> Result<(), String> {
    let b = path.as_bytes();
    let drive = b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
    if !drive || path.chars().any(|c| c.is_control() || c == '"') || path.split('\\').any(|p| p == "..") {
        return Err(format!("{what}: use uma pasta do Windows, como C:\\Users\\voce\\Documentos ({path})"));
    }
    Ok(())
}

fn has_tar_extension(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("tar"))
}

/// Where the backup is written: `X:\...\name.tar`, in a folder that exists.
pub fn validate_backup_destination(path: &str) -> Result<(), String> {
    validate_windows_path(path, "Destino do backup")?;
    if !has_tar_extension(path) {
        return Err("O backup precisa terminar em .tar".into());
    }
    let parent = Path::new(path).parent().filter(|p| p.is_dir());
    if parent.is_none() {
        return Err(format!("A pasta de destino não existe: {path}"));
    }
    Ok(())
}

/// The backup to restore: an existing `.tar`.
pub fn validate_backup_source(path: &str) -> Result<(), String> {
    validate_windows_path(path, "Arquivo de backup")?;
    if !has_tar_extension(path) {
        return Err("Escolha um arquivo de backup .tar".into());
    }
    if !Path::new(path).is_file() {
        return Err(format!("Arquivo não encontrado: {path}"));
    }
    Ok(())
}

/// Where the restored distro's disk goes: a new or empty-of-WSL folder.
pub fn validate_install_dir(path: &str) -> Result<(), String> {
    validate_windows_path(path, "Pasta de instalação")?;
    if Path::new(path).join("ext4.vhdx").exists() {
        return Err("Essa pasta já tem o disco de outra distro: escolha uma pasta vazia".into());
    }
    if Path::new(path).is_file() {
        return Err(format!("{path} é um arquivo, não uma pasta"));
    }
    Ok(())
}

/// The new distro's name: valid for wsl.exe and not taken (WSL compares names ignoring case).
pub fn validate_new_distro_name(name: &str, existing: &[String]) -> Result<(), String> {
    validate_distro_name(name).map_err(|_| "Nome inválido: use letras, números, ponto, hífen ou _ (sem espaços)".to_string())?;
    if existing.iter().any(|e| e.eq_ignore_ascii_case(name)) {
        return Err(format!("Já existe uma distro chamada {name}: escolha outro nome"));
    }
    Ok(())
}

/// Whether `/etc/wsl.conf` lacks `[user] default=...` (then the restored distro would open as root).
pub fn wsl_conf_needs_default_user(conf: &str) -> bool {
    let mut in_user = false;
    for line in conf.lines().map(str::trim) {
        if line.starts_with('[') {
            in_user = line.eq_ignore_ascii_case("[user]");
        } else if in_user && !line.starts_with('#') && line.split('=').next().is_some_and(|k| k.trim().eq_ignore_ascii_case("default")) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Clears the busy flag even when the operation fails.
struct Busy;

impl Busy {
    fn take() -> Result<Self, String> {
        if BUSY.swap(true, Ordering::SeqCst) {
            Err("Já existe um backup ou uma restauração em andamento".into())
        } else {
            Ok(Busy)
        }
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

pub(crate) async fn export(distro: &str, destination: &str) -> Result<BackupResult, String> {
    validate_distro_name(distro)?;
    if is_system_distro(distro) {
        return Err(format!("{distro} pertence a outro programa (Docker, Rancher...): faça o backup por ele"));
    }
    validate_backup_destination(destination)?;
    let _busy = Busy::take()?;
    let started = Instant::now();
    let mut cmd = wsl_command();
    cmd.args(["--export", distro, destination]);
    run(cmd, BACKUP_TIMEOUT).await.map_err(|e| format!("O WSL não conseguiu exportar {distro}: {e}"))?;
    let size = std::fs::metadata(destination).map(|m| m.len()).map_err(|e| format!("Backup não encontrado depois de exportar: {e}"))?;
    Ok(BackupResult { file: destination.to_string(), size, seconds: started.elapsed().as_secs() })
}

/// The first regular user of the restored distro becomes its default (when the backup's
/// wsl.conf doesn't name one). Returns the user it opens with.
async fn restore_default_user(distro: &str) -> Result<Option<String>, String> {
    let probe = r#"awk -F: '$3 >= 1000 && $3 < 60000 { print $1; exit }' /etc/passwd 2>/dev/null
echo @@conf
cat /etc/wsl.conf 2>/dev/null
exit 0"#;
    let text = run(sh_in_distro(distro, Some("root"), probe, &[]), SHORT_TIMEOUT).await?;
    let (user, conf) = text.split_once("@@conf").unwrap_or((&text, ""));
    let conf = conf.strip_prefix('\n').unwrap_or(conf);
    let Some(user) = user.lines().map(str::trim).find(|u| !u.is_empty()).map(str::to_string) else {
        return Ok(None);
    };
    if wsl_conf_needs_default_user(conf) {
        let next = set_wsl_conf_default_user(conf, &user);
        let write = "cat > /etc/wsl.conf.tmp && mv /etc/wsl.conf.tmp /etc/wsl.conf";
        run_raw(sh_in_distro(distro, Some("root"), write, &[]), SHORT_TIMEOUT, Some(next.as_bytes())).await?;
        // wsl.conf is read when the distro boots.
        let mut terminate = wsl_command();
        terminate.args(["--terminate", distro]);
        run(terminate, SHORT_TIMEOUT).await?;
    }
    Ok(Some(user))
}

pub(crate) async fn import(name: &str, install_dir: &str, source: &str) -> Result<ImportResult, String> {
    let existing: Vec<String> = list_distros().await?.into_iter().map(|d| d.name).collect();
    validate_new_distro_name(name, &existing)?;
    validate_backup_source(source)?;
    validate_install_dir(install_dir)?;
    let _busy = Busy::take()?;
    std::fs::create_dir_all(install_dir).map_err(|e| format!("Falha ao criar {install_dir}: {e}"))?;
    let started = Instant::now();
    let mut cmd = wsl_command();
    cmd.args(["--import", name, install_dir, source, "--version", "2"]);
    run(cmd, BACKUP_TIMEOUT).await.map_err(|e| format!("O WSL não conseguiu restaurar o backup: {e}"))?;
    // Best effort: the distro exists already, opening as root is not a failure.
    let default_user = restore_default_user(name).await.unwrap_or(None);
    Ok(ImportResult { distro: name.to_string(), default_user, seconds: started.elapsed().as_secs() })
}

fn defaults() -> BackupDefaults {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    BackupDefaults {
        backup_dir: format!("{}\\Documents\\WSL Backups", env("USERPROFILE")),
        install_root: format!("{}\\WSL", env("LOCALAPPDATA")),
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn export_distro_backup(distro: String, destination_file: String) -> Result<BackupResult, String> {
    export(&distro, &destination_file).await
}

#[tauri::command]
pub async fn import_distro_backup(new_distro_name: String, install_dir: String, source_tar_file: String) -> Result<ImportResult, String> {
    import(&new_distro_name, &install_dir, &source_tar_file).await
}

/// Suggested folders; the backup folder is created so the save dialog can open in it.
#[tauri::command]
pub fn get_backup_defaults() -> BackupDefaults {
    let d = defaults();
    let _ = std::fs::create_dir_all(&d.backup_dir);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_backup_paths() {
        let dir = std::env::temp_dir().join(format!("wsls-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tar = dir.join("ubuntu.tar");
        std::fs::write(&tar, b"x").unwrap();
        let tar = tar.display().to_string();

        assert!(validate_backup_destination(&dir.join("novo backup.TAR").display().to_string()).is_ok());
        assert!(validate_backup_destination(&dir.join("x.zip").display().to_string()).unwrap_err().contains(".tar"));
        assert!(validate_backup_destination(&dir.join("nao-existe").join("x.tar").display().to_string()).unwrap_err().contains("não existe"));
        for bad in [r"\\server\share\x.tar", r"backups\x.tar", "C:\\a\\..\\x.tar", "C:\\a\"b.tar", "C:\\x.tar\n--import"] {
            assert!(validate_backup_destination(bad).is_err(), "{bad:?}");
            assert!(validate_backup_source(bad).is_err(), "{bad:?}");
        }

        assert!(validate_backup_source(&tar).is_ok());
        assert!(validate_backup_source(&dir.join("sumiu.tar").display().to_string()).unwrap_err().contains("não encontrado"));

        assert!(validate_install_dir(&dir.join("Ubuntu-Copia").display().to_string()).is_ok(), "a new folder");
        std::fs::write(dir.join("ext4.vhdx"), b"x").unwrap();
        assert!(validate_install_dir(&dir.display().to_string()).unwrap_err().contains("outra distro"));
        assert!(validate_install_dir(&tar).unwrap_err().contains("arquivo"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validates_new_names() {
        let existing = vec!["Ubuntu".to_string(), "docker-desktop".to_string()];
        assert!(validate_new_distro_name("Ubuntu-Backup", &existing).is_ok());
        assert!(validate_new_distro_name("ubuntu", &existing).unwrap_err().contains("Já existe"), "WSL ignores case");
        for bad in ["", "meu ubuntu", "--shutdown", "a;b"] {
            assert!(validate_new_distro_name(bad, &existing).is_err(), "{bad:?}");
        }
    }

    /// Round trip with a tiny throwaway distro (Alpine minirootfs): import it raw, export it with
    /// `export`, restore the .tar under a new name with `import` (with a regular user, which must
    /// become the default), then unregister both. Refuses names that already exist.
    /// `WSL_E2E_ROOTFS=C:\...\alpine-minirootfs.tar.gz cargo test --lib live_backup -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_backup_roundtrip() {
        let rootfs = std::env::var("WSL_E2E_ROOTFS").expect("set WSL_E2E_ROOTFS");
        let (src, copy) = ("SymbiosisBackupSrc", "SymbiosisBackupCopy");
        let names: Vec<String> = list_distros().await.unwrap().into_iter().map(|d| d.name).collect();
        assert!(!names.iter().any(|n| n == src || n == copy), "test distros already exist");
        let dir = std::env::temp_dir().join(format!("wsls-backup-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wsl = |args: &[&str]| {
            let mut c = wsl_command();
            c.args(args);
            c
        };
        /// Unregisters the test distros even when an assertion panics.
        struct Cleanup(Vec<&'static str>, std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                use std::os::windows::process::CommandExt;
                for name in &self.0 {
                    let _ = std::process::Command::new("wsl.exe").args(["--unregister", name]).creation_flags(0x0800_0000).status();
                }
                let _ = std::fs::remove_dir_all(&self.1);
            }
        }
        let _cleanup = Cleanup(vec![src, copy], dir.clone());

        let src_dir = dir.join("src").display().to_string();
        run(wsl(&["--import", src, &src_dir, &rootfs, "--version", "2"]), BACKUP_TIMEOUT).await.expect("raw import");
        run(sh_in_distro(src, Some("root"), "adduser -D -u 1000 kyo && echo marcador > /root/prova", &[]), SHORT_TIMEOUT).await.unwrap();

        let tar = dir.join("backup.tar").display().to_string();
        let exported = export(src, &tar).await.expect("export");
        println!("{exported:?}");
        assert!(exported.size > 1_000_000);
        assert!(import(src, &dir.join("x").display().to_string(), &tar).await.unwrap_err().contains("Já existe"));

        let imported = import(copy, &dir.join("copy").display().to_string(), &tar).await.expect("import");
        println!("{imported:?}");
        assert_eq!(imported.default_user.as_deref(), Some("kyo"));
        let who = run(sh_in_distro(copy, None, "id -un", &[]), SHORT_TIMEOUT).await.unwrap();
        assert_eq!(who.trim(), "kyo", "opens as the regular user");
        let marker = run(sh_in_distro(copy, Some("root"), "cat /root/prova", &[]), SHORT_TIMEOUT).await.unwrap();
        assert_eq!(marker.trim(), "marcador", "same content as the original");
    }

    #[test]
    fn detects_missing_default_user() {
        assert!(wsl_conf_needs_default_user(""));
        assert!(wsl_conf_needs_default_user("[boot]\nsystemd=true\n"));
        assert!(wsl_conf_needs_default_user("[user]\n# default=kyo\n"));
        assert!(!wsl_conf_needs_default_user("[boot]\nsystemd=true\n[user]\ndefault=kyo\n"));
        assert!(wsl_conf_needs_default_user("[network]\ndefault=x\n"), "only [user] counts");
    }
}
