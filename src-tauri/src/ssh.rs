//! Git identity (`user.name` / `user.email`) and the SSH key the distro uses for GitHub/GitLab.
//!
//! Everything runs as the distro's default user, inside its own `~/.ssh` (ext4: real Unix
//! permissions). Private keys never travel as arguments: imported keys go through stdin. Nothing
//! is ever overwritten: an existing `id_ed25519` blocks generation, existing Linux keys block the
//! import of a Windows key with the same name, and `~/.ssh/config` only gets a block prepended
//! (with a backup of the original on the first edit).

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::packages::run_user_script;
use crate::provisioner::is_system_distro;
use crate::wsl::{ensure_running, run, run_raw, sh_in_distro};

const READ_TIMEOUT: Duration = Duration::from_secs(20);
const KEYGEN_TIMEOUT: Duration = Duration::from_secs(60);
const SSH_TEST_TIMEOUT: Duration = Duration::from_secs(40);

/// Key files looked for, in order of preference (Linux and Windows).
pub const KEY_NAMES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];
/// Hosts that get `StrictHostKeyChecking accept-new`, so the first clone/pull over SSH doesn't
/// fail on the "Are you sure you want to continue connecting?" question (there is no terminal).
pub const TRUSTED_HOSTS: [&str; 2] = ["github.com", "gitlab.com"];

/// Scripts here only need Linux tools: Windows' `ssh.exe` / `git.exe` must never be picked up.
const LINUX_PATH: &str = "export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n";

// ---------------------------------------------------------------------------
// Types (mirrored in src/lib/ssh.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyInfo {
    /// `id_ed25519`
    pub name: String,
    /// `ED25519`, `ECDSA`, `RSA` (from the public key).
    pub kind: Option<String>,
    /// The `.pub` line to paste into GitHub; `None` when the .pub file is missing.
    pub public_key: Option<String>,
    /// `SHA256:...`
    pub fingerprint: Option<String>,
    /// Octal mode of the private key (`600`).
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowsKey {
    pub name: String,
    pub has_public: bool,
    /// A Linux key with the same name exists: importing would skip it.
    pub exists_in_linux: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitIdentityStatus {
    pub name: Option<String>,
    pub email: Option<String>,
    pub git_installed: bool,
    pub keygen_installed: bool,
    /// Octal mode of `~/.ssh`; `None` when it doesn't exist.
    pub ssh_dir_mode: Option<String>,
    pub keys: Vec<SshKeyInfo>,
    /// `~/.ssh` or a private key is readable/writable by other users: ssh refuses such keys
    /// ("UNPROTECTED PRIVATE KEY FILE").
    pub permissions_ok: bool,
    /// Keys in `%USERPROFILE%\.ssh` that can be imported.
    pub windows_keys: Vec<WindowsKey>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshImportResult {
    pub imported: Vec<String>,
    /// Already in Linux, left untouched.
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshTestResult {
    pub ok: bool,
    /// GitHub username, when authenticated.
    pub username: Option<String>,
    /// Plain-language result.
    pub message: String,
    /// What ssh printed (shown as detail).
    pub detail: String,
}

// ---------------------------------------------------------------------------
// Validation and pure rules
// ---------------------------------------------------------------------------

/// What `git config user.name` gets: 1-100 printable characters.
pub fn validate_git_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Informe o seu nome".into());
    }
    if name.chars().count() > 100 || name.chars().any(char::is_control) || name.contains(['<', '>']) {
        return Err("Nome inválido: use só letras, números e espaços (até 100 caracteres)".into());
    }
    Ok(name.to_string())
}

/// A plausible e-mail (git rejects `<`/`>`; no spaces or control characters).
pub fn validate_git_email(email: &str) -> Result<String, String> {
    let email = email.trim();
    let invalid = || Err(format!("E-mail inválido: {email}"));
    if email.is_empty() {
        return Err("Informe o seu e-mail".into());
    }
    if email.len() > 254 || email.chars().any(|c| c.is_control() || c.is_whitespace() || c == '<' || c == '>') {
        return invalid();
    }
    let Some((local, domain)) = email.split_once('@') else { return invalid() };
    let domain_ok = domain.contains('.')
        && !domain.contains('@')
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with('.')
        && !domain.contains("..");
    if local.is_empty() || !domain_ok {
        return invalid();
    }
    Ok(email.to_string())
}

fn mode_bits(mode: &str) -> Option<u32> {
    u32::from_str_radix(mode.trim(), 8).ok()
}

/// ssh refuses private keys others can read, and `~/.ssh` writable by others.
pub fn permissions_secure(dir_mode: Option<&str>, key_modes: &[&str]) -> bool {
    let dir_ok = dir_mode.is_none_or(|m| mode_bits(m).is_some_and(|b| b & 0o022 == 0));
    dir_ok && key_modes.iter().all(|m| mode_bits(m).is_some_and(|b| b & 0o077 == 0))
}

/// Mode a file in `~/.ssh` should have, from its name and first line; `None` = leave it alone.
pub fn ssh_file_mode(name: &str, first_line: &str) -> Option<&'static str> {
    if name.ends_with(".pub") || matches!(name, "config" | "known_hosts" | "known_hosts.old" | "authorized_keys") {
        Some("644")
    } else if first_line.contains("PRIVATE KEY") {
        Some("600")
    } else {
        None
    }
}

/// `~/.ssh/config` with `StrictHostKeyChecking accept-new` for the trusted hosts the user
/// hasn't configured yet, prepended (ssh uses the first value it finds). `None` = no change.
pub fn ssh_config_with_trusted_hosts(existing: &str) -> Option<String> {
    let declared: Vec<String> = existing
        .lines()
        .filter_map(|line| {
            let mut tokens = line.split(|c: char| c.is_whitespace() || c == '=').filter(|t| !t.is_empty());
            tokens.next().filter(|k| k.eq_ignore_ascii_case("host")).map(|_| tokens.map(str::to_ascii_lowercase).collect::<Vec<_>>())
        })
        .flatten()
        .collect();
    let missing: Vec<&str> = TRUSTED_HOSTS.iter().copied().filter(|h| !declared.iter().any(|d| d == h)).collect();
    if missing.is_empty() {
        return None;
    }
    let block = format!(
        "# WSL Symbiosis: o primeiro acesso a estes servidores aceita a identidade deles sem perguntar\n\
         Host {}\n    StrictHostKeyChecking accept-new\n",
        missing.join(" ")
    );
    Some(if existing.trim().is_empty() { block } else { format!("{block}\n{existing}") })
}

fn key_kind(public_key: &str) -> Option<String> {
    let algo = public_key.split_whitespace().next()?;
    Some(
        match algo {
            "ssh-ed25519" => "ED25519",
            "ssh-rsa" => "RSA",
            a if a.starts_with("ecdsa-") => "ECDSA",
            a if a.starts_with("sk-") => "Chave de segurança",
            _ => return None,
        }
        .to_string(),
    )
}

/// `@@`-line output of [`STATUS_SCRIPT`] (Windows keys are filled in afterwards).
pub fn parse_status(text: &str) -> GitIdentityStatus {
    let mut s = GitIdentityStatus {
        name: None,
        email: None,
        git_installed: false,
        keygen_installed: false,
        ssh_dir_mode: None,
        keys: Vec::new(),
        permissions_ok: true,
        windows_keys: Vec::new(),
    };
    let value = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("@@") else { continue };
        let (tag, v) = rest.split_once(' ').unwrap_or((rest, ""));
        match tag {
            "name" => s.name = value(v),
            "email" => s.email = value(v),
            "has" => match v.trim() {
                "git" => s.git_installed = true,
                "keygen" => s.keygen_installed = true,
                _ => {}
            },
            "dir" => s.ssh_dir_mode = value(v),
            "key" => {
                let (name, mode) = v.trim().split_once(' ').unwrap_or((v.trim(), ""));
                if KEY_NAMES.contains(&name) {
                    s.keys.push(SshKeyInfo { name: name.into(), kind: None, public_key: None, fingerprint: None, mode: mode.trim().into() });
                }
            }
            "pub" => {
                if let Some(key) = s.keys.last_mut() {
                    key.public_key = value(v);
                    key.kind = key.public_key.as_deref().and_then(key_kind);
                }
            }
            "fp" => {
                if let Some(key) = s.keys.last_mut() {
                    key.fingerprint = v.split_whitespace().find(|t| t.starts_with("SHA256:") || t.starts_with("MD5:")).map(str::to_string);
                }
            }
            _ => {}
        }
    }
    let modes: Vec<&str> = s.keys.iter().map(|k| k.mode.as_str()).collect();
    s.permissions_ok = permissions_secure(s.ssh_dir_mode.as_deref(), &modes);
    s
}

/// Output of `ssh -T git@github.com` (it exits 1 even when authenticated).
pub fn parse_ssh_test(output: &str) -> SshTestResult {
    let detail: String = output.lines().filter(|l| !l.starts_with("@@")).collect::<Vec<_>>().join("\n").trim().to_string();
    let has = |n: &str| output.contains(n);
    let username = output.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("Hi ")?;
        let (user, tail) = rest.split_once('!')?;
        tail.contains("successfully authenticated").then(|| user.trim().to_string())
    });
    let (ok, message) = if let Some(user) = &username {
        (true, format!("Conectado como @{user}"))
    } else if has("Permission denied (publickey)") {
        (false, "O GitHub recusou a chave: copie a chave pública e cadastre em GitHub → Chaves SSH".into())
    } else if has("Could not resolve hostname") || has("Connection timed out") || has("Network is unreachable") || has("Connection refused") {
        (false, "Sem conexão com o GitHub: confira a internet".into())
    } else if has("Host key verification failed") || has("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        (false, "A identidade do GitHub não confere com a salva em ~/.ssh/known_hosts".into())
    } else if has("UNPROTECTED PRIVATE KEY FILE") || has("bad permissions") {
        (false, "As permissões da chave estão abertas demais: use Corrigir permissões".into())
    } else if has("@@no-ssh") {
        (false, "O cliente SSH não está instalado na distro".into())
    } else {
        (false, "Não foi possível autenticar no GitHub".into())
    };
    SshTestResult { ok, username, message, detail }
}

fn explain(raw: &str) -> String {
    if raw.contains("@@error no-git") {
        "O git não está instalado nesta distro (o Setup instala para você)".into()
    } else if raw.contains("@@error no-keygen") {
        "O OpenSSH não está instalado nesta distro (sudo apt install openssh-client)".into()
    } else if raw.contains("@@error exists") {
        "Já existe uma chave id_ed25519 nesta distro: ela não é substituída".into()
    } else {
        raw.trim().to_string()
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

async fn prepare(distro: &str) -> Result<(), String> {
    if is_system_distro(distro) {
        return Err(format!("{distro} é uma distro interna de outro programa, não um ambiente de desenvolvimento"));
    }
    ensure_running(distro).await
}

async fn sh(distro: &str, script: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let script = format!("{LINUX_PATH}{script}");
    run(sh_in_distro(distro, None, &script, args), timeout).await.map_err(|e| explain(&e))
}

const STATUS_SCRIPT: &str = r#"d="$HOME/.ssh"
printf '@@name %s\n' "$(git config --global --get user.name 2>/dev/null)"
printf '@@email %s\n' "$(git config --global --get user.email 2>/dev/null)"
command -v git >/dev/null 2>&1 && echo "@@has git"
command -v ssh-keygen >/dev/null 2>&1 && echo "@@has keygen"
[ -d "$d" ] && echo "@@dir $(stat -c %a "$d")"
for k in id_ed25519 id_ecdsa id_rsa; do
  f="$d/$k"
  [ -f "$f" ] || continue
  echo "@@key $k $(stat -c %a "$f")"
  if [ -f "$f.pub" ]; then
    echo "@@pub $(head -n 1 "$f.pub")"
    echo "@@fp $(ssh-keygen -lf "$f.pub" 2>/dev/null)"
  fi
done
exit 0"#;

fn windows_ssh_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".ssh"))
}

fn windows_keys(linux: &[SshKeyInfo]) -> Vec<WindowsKey> {
    let Some(dir) = windows_ssh_dir() else { return Vec::new() };
    KEY_NAMES
        .iter()
        .filter(|name| dir.join(name).is_file())
        .map(|name| WindowsKey {
            name: name.to_string(),
            has_public: dir.join(format!("{name}.pub")).is_file(),
            exists_in_linux: linux.iter().any(|k| k.name == *name),
        })
        .collect()
}

pub(crate) async fn status(distro: &str) -> Result<GitIdentityStatus, String> {
    prepare(distro).await?;
    let text = sh(distro, STATUS_SCRIPT, &[], READ_TIMEOUT).await?;
    let mut status = parse_status(&text);
    status.windows_keys = windows_keys(&status.keys);
    Ok(status)
}

pub(crate) async fn save_identity(distro: &str, name: &str, email: &str) -> Result<(), String> {
    let name = validate_git_name(name)?;
    let email = validate_git_email(email)?;
    prepare(distro).await?;
    let script = r#"command -v git >/dev/null 2>&1 || { echo "@@error no-git"; exit 4; }
git config --global user.name "$1" && git config --global user.email "$2""#;
    sh(distro, script, &[&name, &email], READ_TIMEOUT).await.map(|_| ())
}

/// Prepends the trusted-hosts block to `~/.ssh/config` when it's missing.
async fn ensure_ssh_config(distro: &str) -> Result<(), String> {
    let existing = sh(distro, r#"cat "$HOME/.ssh/config" 2>/dev/null; exit 0"#, &[], READ_TIMEOUT).await?;
    let Some(next) = ssh_config_with_trusted_hosts(&existing) else { return Ok(()) };
    let write = format!(
        r#"{LINUX_PATH}d="$HOME/.ssh"; f="$d/config"
mkdir -p "$d" && chmod 700 "$d" || exit 1
if [ -f "$f" ] && [ ! -e "$f.symbiosis.bak" ]; then cp -p -- "$f" "$f.symbiosis.bak"; fi
cat > "$f.tmp" && mv -- "$f.tmp" "$f" && chmod 644 "$f""#
    );
    run_raw(sh_in_distro(distro, None, &write, &[]), READ_TIMEOUT, Some(next.as_bytes())).await.map(|_| ())
}

const KEYGEN_SCRIPT: &str = r#"d="$HOME/.ssh"; f="$d/id_ed25519"
if [ -e "$f" ] || [ -L "$f" ]; then echo "@@error exists"; exit 6; fi
command -v ssh-keygen >/dev/null 2>&1 || { echo "@@error no-keygen"; exit 4; }
mkdir -p "$d" && chmod 700 "$d" || exit 1
ssh-keygen -q -t ed25519 -C "$1" -f "$f" -N "" </dev/null || exit 1
chmod 600 "$f" && chmod 644 "$f.pub""#;

/// New `~/.ssh/id_ed25519` (no passphrase: the app has no terminal to type it). Refuses when one
/// exists.
pub(crate) async fn generate_key(distro: &str, email: &str) -> Result<GitIdentityStatus, String> {
    let email = validate_git_email(email)?;
    prepare(distro).await?;
    sh(distro, KEYGEN_SCRIPT, &[&email], KEYGEN_TIMEOUT).await?;
    ensure_ssh_config(distro).await?;
    status(distro).await
}

/// Writes one file into `~/.ssh` from stdin, unless it exists. `$1` = name, `$2` = mode.
const PUT_KEY_SCRIPT: &str = r#"d="$HOME/.ssh"; f="$d/$1"
mkdir -p "$d" && chmod 700 "$d" || exit 1
if [ -e "$f" ] || [ -L "$f" ]; then echo "@@skip"; cat >/dev/null; exit 0; fi
umask 077
cat > "$f.tmp" && mv -- "$f.tmp" "$f" && chmod "$2" "$f""#;

/// Copies `%USERPROFILE%\.ssh\id_*` (private + .pub) into `~/.ssh` with ssh's permissions
/// (Windows copies end up 0777 on drvfs, which ssh refuses). Keys already in Linux are skipped.
pub(crate) async fn import_windows_keys(distro: &str) -> Result<SshImportResult, String> {
    prepare(distro).await?;
    let current = status(distro).await?;
    let dir = windows_ssh_dir().ok_or("Pasta do usuário do Windows não encontrada")?;
    let mut result = SshImportResult { imported: Vec::new(), skipped: Vec::new() };
    for key in &current.windows_keys {
        if key.exists_in_linux {
            result.skipped.push(key.name.clone());
            continue;
        }
        let mut files = vec![(key.name.clone(), "600")];
        if key.has_public {
            files.push((format!("{}.pub", key.name), "644"));
        }
        for (file, mode) in files {
            let bytes = std::fs::read(dir.join(&file)).map_err(|e| format!("Falha ao ler {file} do Windows: {e}"))?;
            // Keys saved by Windows editors may have CRLF, which OpenSSH on Linux can't parse.
            let bytes = String::from_utf8_lossy(&bytes).replace("\r\n", "\n").into_bytes();
            let script = format!("{LINUX_PATH}{PUT_KEY_SCRIPT}");
            run_raw(sh_in_distro(distro, None, &script, &[&file, mode]), READ_TIMEOUT, Some(&bytes)).await?;
        }
        result.imported.push(key.name.clone());
    }
    if result.imported.is_empty() && result.skipped.is_empty() {
        return Err("Nenhuma chave SSH encontrada em %USERPROFILE%\\.ssh".into());
    }
    ensure_ssh_config(distro).await?;
    Ok(result)
}

const LIST_SSH_FILES: &str = r#"d="$HOME/.ssh"; [ -d "$d" ] || exit 0; cd "$d" || exit 0
for f in * .[!.]*; do
  [ -f "$f" ] || continue
  printf '@@f %s\t%s\n' "$f" "$(head -c 200 "$f" 2>/dev/null | head -n 1)"
done
exit 0"#;

/// `$1` = how many of the following names get 600; the rest get 644.
const CHMOD_SSH_FILES: &str = r#"cd "$HOME/.ssh" 2>/dev/null || exit 0
chmod 700 . || exit 1
n=$1; shift; i=0
for f in "$@"; do i=$((i + 1)); if [ "$i" -le "$n" ]; then chmod 600 -- "$f"; else chmod 644 -- "$f"; fi; done"#;

/// `~/.ssh` 700, private keys 600, public keys / config / known_hosts 644.
pub(crate) async fn fix_permissions(distro: &str) -> Result<(), String> {
    prepare(distro).await?;
    let listing = sh(distro, LIST_SSH_FILES, &[], READ_TIMEOUT).await?;
    let (mut private, mut public) = (Vec::new(), Vec::new());
    for line in listing.lines() {
        let Some((name, first)) = line.strip_prefix("@@f ").and_then(|l| l.split_once('\t')) else { continue };
        if name.is_empty() || name.contains('/') {
            continue;
        }
        match ssh_file_mode(name, first) {
            Some("600") => private.push(name.to_string()),
            Some(_) => public.push(name.to_string()),
            None => {}
        }
    }
    let count = private.len().to_string();
    let mut args: Vec<&str> = vec![&count];
    args.extend(private.iter().map(String::as_str));
    args.extend(public.iter().map(String::as_str));
    sh(distro, CHMOD_SSH_FILES, &args, READ_TIMEOUT).await.map(|_| ())
}

/// In the user's login shell (an ssh-agent started by the rc files counts), like git pull.
const SSH_TEST: &str = r#"command -v ssh >/dev/null 2>&1 || { echo "@@no-ssh"; exit 0; }
LC_ALL=C ssh -T -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new git@github.com 2>&1
echo "@@exit $?"
exit 0"#;

pub(crate) async fn test_github(distro: &str) -> Result<SshTestResult, String> {
    prepare(distro).await?;
    let output = run_user_script(distro, SSH_TEST, &[], SSH_TEST_TIMEOUT).await?;
    Ok(parse_ssh_test(&output))
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_git_identity_and_ssh(distro: String) -> Result<GitIdentityStatus, String> {
    status(&distro).await
}

#[tauri::command]
pub async fn save_git_identity(distro: String, name: String, email: String) -> Result<(), String> {
    save_identity(&distro, &name, &email).await
}

#[tauri::command]
pub async fn generate_ssh_key(distro: String, email: String) -> Result<GitIdentityStatus, String> {
    generate_key(&distro, &email).await
}

#[tauri::command]
pub async fn import_windows_ssh_keys(distro: String) -> Result<SshImportResult, String> {
    import_windows_keys(&distro).await
}

#[tauri::command]
pub async fn fix_ssh_permissions(distro: String) -> Result<(), String> {
    fix_permissions(&distro).await
}

#[tauri::command]
pub async fn test_ssh_github(distro: String) -> Result<SshTestResult, String> {
    test_github(&distro).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_identity() {
        assert_eq!(validate_git_name("  Kyo Silva ").as_deref(), Ok("Kyo Silva"));
        assert_eq!(validate_git_name("José Ção").as_deref(), Ok("José Ção"));
        for bad in ["", "   ", "a\nb", "Nome <x>", &"x".repeat(101)] {
            assert!(validate_git_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(validate_git_email(" ti@fleeting.com.br ").as_deref(), Ok("ti@fleeting.com.br"));
        assert!(validate_git_email("123+kyo@users.noreply.github.com").is_ok());
        for bad in ["", "kyo", "kyo@", "@x.com", "kyo@localhost", "a b@x.com", "a@b@c.com", "a@.x.com", "a@x..com", "a@x.com.", "<a@x.com>", "a@x\n.com"] {
            assert!(validate_git_email(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn checks_and_fixes_permissions() {
        assert!(permissions_secure(Some("700"), &["600"]));
        assert!(permissions_secure(Some("755"), &["600", "400"]), "a readable ~/.ssh is accepted by ssh");
        assert!(permissions_secure(None, &[]));
        assert!(!permissions_secure(Some("777"), &["600"]), "copied through the Explorer");
        assert!(!permissions_secure(Some("700"), &["644"]));
        assert!(!permissions_secure(Some("700"), &["777"]));
        assert_eq!(ssh_file_mode("id_ed25519", "-----BEGIN OPENSSH PRIVATE KEY-----"), Some("600"));
        assert_eq!(ssh_file_mode("work_key", "-----BEGIN RSA PRIVATE KEY-----"), Some("600"));
        assert_eq!(ssh_file_mode("id_ed25519.pub", "ssh-ed25519 AAAA x"), Some("644"));
        assert_eq!(ssh_file_mode("config", "Host *"), Some("644"));
        assert_eq!(ssh_file_mode("known_hosts", "|1|abc"), Some("644"));
        assert_eq!(ssh_file_mode("notes.txt", "hello"), None, "unknown files are left alone");
    }

    #[test]
    fn ssh_config_trusts_github_once() {
        let fresh = ssh_config_with_trusted_hosts("").unwrap();
        assert!(fresh.contains("Host github.com gitlab.com\n    StrictHostKeyChecking accept-new\n"));
        assert_eq!(ssh_config_with_trusted_hosts(&fresh), None, "idempotent");

        let user = "Host work\n    HostName git.corp.com\n    IdentityFile ~/.ssh/work\n";
        let next = ssh_config_with_trusted_hosts(user).unwrap();
        assert!(next.starts_with("# WSL Symbiosis"), "prepended: ssh uses the first value found");
        assert!(next.ends_with(user), "the user's config is kept as is");

        // Hosts the user already configured are never touched.
        let mine = "Host GitHub.com\n  IdentityFile ~/.ssh/gh\n";
        let next = ssh_config_with_trusted_hosts(mine).unwrap();
        assert!(next.contains("Host gitlab.com\n") && !next.contains("Host github.com"));
        assert_eq!(ssh_config_with_trusted_hosts("Host=github.com gitlab.com\n"), None);
    }

    #[test]
    fn parses_status() {
        let text = "@@name Kyo\n@@email ti@fleeting.com.br\n@@has git\n@@has keygen\n@@dir 700\n\
                    @@key id_ed25519 600\n@@pub ssh-ed25519 AAAAC3Nza kyo@pc\n@@fp 256 SHA256:abc123 kyo@pc (ED25519)\n\
                    @@key id_rsa 644\n";
        let s = parse_status(text);
        assert_eq!((s.name.as_deref(), s.email.as_deref()), (Some("Kyo"), Some("ti@fleeting.com.br")));
        assert!(s.git_installed && s.keygen_installed);
        assert_eq!(s.keys.len(), 2);
        assert_eq!(s.keys[0].kind.as_deref(), Some("ED25519"));
        assert_eq!(s.keys[0].fingerprint.as_deref(), Some("SHA256:abc123"));
        assert_eq!(s.keys[1].public_key, None, "private key without .pub");
        assert!(!s.permissions_ok, "id_rsa is 644");

        let empty = parse_status("@@name \n@@email \n@@has git\n");
        assert_eq!((empty.name, empty.email, empty.keys.len(), empty.permissions_ok), (None, None, 0, true));
    }

    #[test]
    fn parses_github_test() {
        let ok = parse_ssh_test("Hi consolelogkyo! You've successfully authenticated, but GitHub does not provide shell access.\n@@exit 1\n");
        assert!(ok.ok);
        assert_eq!(ok.username.as_deref(), Some("consolelogkyo"));
        assert_eq!(ok.message, "Conectado como @consolelogkyo");
        assert!(!ok.detail.contains("@@"));
        let denied = parse_ssh_test("git@github.com: Permission denied (publickey).\n@@exit 255\n");
        assert!(!denied.ok && denied.message.contains("recusou a chave"));
        assert!(parse_ssh_test("ssh: Could not resolve hostname github.com\n@@exit 255").message.contains("Sem conexão"));
        assert!(parse_ssh_test("@@no-ssh\n").message.contains("não está instalado"));
    }

    /// Real distro, throwaway HOME: generate a key, fix permissions, config block.
    /// `WSL_TEST_DISTRO=Ubuntu cargo test --lib live_ssh -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_ssh_keygen_in_scratch_home() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let home = format!("/tmp/wsls-ssh-{}", std::process::id());
        let with_home = |s: &str| format!("export HOME=\"{home}\"\n{s}");
        run(sh_in_distro(&distro, None, r#"rm -rf "$1"; mkdir -p "$1""#, &[&home]), READ_TIMEOUT).await.unwrap();

        sh(&distro, &with_home(KEYGEN_SCRIPT), &["teste@exemplo.com"], KEYGEN_TIMEOUT).await.expect("keygen");
        let err = sh(&distro, &with_home(KEYGEN_SCRIPT), &["teste@exemplo.com"], KEYGEN_TIMEOUT).await.unwrap_err();
        assert!(err.contains("não é substituída"), "{err}");

        // Loosen everything, then fix through the same listing/chmod scripts.
        run(sh_in_distro(&distro, None, r#"chmod 777 "$1/.ssh" "$1/.ssh/id_ed25519"; echo 'Host x' > "$1/.ssh/config""#, &[&home]), READ_TIMEOUT)
            .await
            .unwrap();
        let s = parse_status(&sh(&distro, &with_home(STATUS_SCRIPT), &[], READ_TIMEOUT).await.unwrap());
        assert!(!s.permissions_ok);
        let listing = sh(&distro, &with_home(LIST_SSH_FILES), &[], READ_TIMEOUT).await.unwrap();
        println!("{listing}");
        assert!(listing.contains("@@f id_ed25519\t-----BEGIN OPENSSH PRIVATE KEY-----"));
        sh(&distro, &with_home(CHMOD_SSH_FILES), &["1", "id_ed25519", "id_ed25519.pub", "config"], READ_TIMEOUT).await.unwrap();
        let s = parse_status(&sh(&distro, &with_home(STATUS_SCRIPT), &[], READ_TIMEOUT).await.unwrap());
        println!("{s:#?}");
        assert!(s.permissions_ok);
        assert_eq!(s.keys[0].kind.as_deref(), Some("ED25519"));
        assert!(s.keys[0].public_key.as_deref().unwrap().ends_with("teste@exemplo.com"));

        run(sh_in_distro(&distro, None, r#"rm -rf "$1""#, &[&home]), READ_TIMEOUT).await.unwrap();
    }
}
