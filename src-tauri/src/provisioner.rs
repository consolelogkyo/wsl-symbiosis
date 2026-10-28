//! First-run setup ("Zero-to-Hero"): inspects the machine and provisions WSL2, an Ubuntu
//! distro, Windows Terminal (Ubuntu as default + Dracula) and zsh (Oh-My-Zsh, Spaceship,
//! plugins, nvm).
//!
//! Rules:
//! - inspection never boots a stopped distro (CLAUDE.md 3.1); provisioning may, the user asked;
//! - every module is idempotent and backs up a config file before its first edit
//!   (`<file>.symbiosis.bak`, kept from the first run so it always holds the original);
//! - config files (`~/.zshrc`, `/etc/wsl.conf`, Windows Terminal `settings.json`) are patched by
//!   the pure functions below (unit-tested), never by sed;
//! - the Linux password only travels through stdin (`chpasswd`), never as an argument or log line.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::packages::strip_ansi;
use crate::settings::SettingsStore;
use crate::wsl::{self, Distro, DistroState, hidden_command, run, run_raw, sh_in_distro, validate_distro_name, wsl_command};

pub const MODULES: [&str; 5] = ["windows_terminal_dracula", "zsh_ohmyzsh", "spaceship_theme", "zsh_plugins", "nvm_node_lts"];

const INSPECT_TIMEOUT: Duration = Duration::from_secs(20);
const SHORT_TIMEOUT: Duration = Duration::from_secs(30);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const LONG_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const DISTRO_INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Linux tools only: Windows paths in $PATH would expose git.exe & co.
const LINUX_PATH: &str = "export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n";
const BACKUP_SUFFIX: &str = ".symbiosis.bak";
const NVM_INSTALLER: &str = "https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh";
const OMZ_INSTALLER: &str = "https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh";
const COMPLETIONS_FPATH: &str = "fpath+=${ZSH_CUSTOM:-${ZSH:-~/.oh-my-zsh}/custom}/plugins/zsh-completions/src";

/// Only one provisioning (or distro install) at a time.
#[derive(Default)]
pub struct ProvisionLock(Mutex<()>);

// ---------------------------------------------------------------------------
// Inspection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStateInspection {
    pub wsl_installed: bool,
    pub wsl_version: Option<String>,
    /// Windows features were enabled and a restart is still pending.
    pub needs_reboot: bool,
    pub distros: Vec<Distro>,
    /// A real Linux distro (docker-desktop & co. don't count).
    pub has_linux_distro: bool,
    pub windows_terminal: WindowsTerminalState,
    /// The distro the Linux side refers to.
    pub distro: Option<String>,
    pub distro_running: bool,
    /// `None` when there is no distro or it is stopped (inspection never boots it).
    pub linux: Option<LinuxState>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsTerminalState {
    pub installed: bool,
    pub settings_path: Option<String>,
    /// The selected distro's profile is the default one.
    pub ubuntu_is_default: bool,
    pub has_dracula_scheme: bool,
    /// The selected distro's profile uses the Dracula scheme.
    pub ubuntu_uses_dracula: bool,
    /// Windows Terminal already has a profile for the selected distro (it creates them itself).
    pub profile_found: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxState {
    pub user: String,
    pub has_apt: bool,
    pub zsh_installed: bool,
    pub zsh_is_default_shell: bool,
    pub oh_my_zsh_installed: bool,
    pub spaceship_installed: bool,
    /// Active `ZSH_THEME` (the UI warns before replacing another theme, e.g. powerlevel10k).
    pub zsh_theme: Option<String>,
    pub zsh_plugins: ZshPluginsState,
    pub nvm_installed: bool,
    pub node_lts_installed: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZshPluginsState {
    pub autosuggestions: bool,
    pub syntax_highlighting: bool,
    pub completions: bool,
}

const INSPECT_SCRIPT: &str = r#"u=$(id -un 2>/dev/null)
echo "@@user"; echo "$u"
echo "@@shell"; getent passwd "$u" 2>/dev/null | cut -d: -f7
echo "@@flags"
command -v zsh >/dev/null 2>&1 && echo zsh
command -v apt-get >/dev/null 2>&1 && echo apt
c=$HOME/.oh-my-zsh/custom
[ -d "$HOME/.oh-my-zsh" ] && echo omz
[ -d "$c/themes/spaceship-prompt" ] && echo spaceship_dir
[ -d "$c/plugins/zsh-autosuggestions" ] && echo autosuggestions_dir
[ -d "$c/plugins/zsh-syntax-highlighting" ] && echo syntax_dir
[ -d "$c/plugins/zsh-completions" ] && echo completions_dir
n=$HOME/.nvm
[ -s "$n/nvm.sh" ] && echo nvm
[ -s "$n/alias/default" ] && [ -n "$(ls "$n/versions/node" 2>/dev/null)" ] && echo node
echo "@@zshrc"
cat "$HOME/.zshrc" 2>/dev/null
exit 0
"#;

/// Internal VMs of desktop tools (Docker Desktop, Rancher Desktop, Podman). They are not a
/// development environment: never selected, inspected or provisioned. Mirrored by
/// `isSystemDistro` in src/lib/wsl.ts.
pub fn is_system_distro(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("docker-desktop") || n.starts_with("rancher-desktop") || n.starts_with("podman-machine")
}

/// Whether the app opens on the full-window setup:
/// - first run, or coming from a version without it (`setup_wizard_seen` is false after updating
///   from 0.1.2, whose settings already say `onboarding_completed`);
/// - WSL was just enabled and Windows restarted;
/// - no development distro (none, or only docker-desktop & co.). `distros` = None when wsl.exe
///   didn't answer: an unknown list never forces the setup.
pub fn setup_required(settings: &crate::settings::Settings, distros: Option<&[Distro]>) -> bool {
    let no_dev_distro = distros.is_some_and(|list| list.iter().all(|d| is_system_distro(&d.name)));
    !settings.onboarding_completed || !settings.setup_wizard_seen || settings.wsl_reboot_pending || no_dev_distro
}

/// Splits `@@name` sections of the inspection output.
fn sections(text: &str) -> Vec<(&str, String)> {
    let mut out: Vec<(&str, String)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            out.push((name.trim(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out
}

pub fn parse_linux_state(text: &str) -> LinuxState {
    let secs = sections(text);
    let get = |name: &str| secs.iter().find(|(n, _)| *n == name).map(|(_, b)| b.as_str()).unwrap_or("");
    let flags: Vec<&str> = get("flags").lines().map(str::trim).collect();
    let has = |f: &str| flags.contains(&f);
    let zshrc = get("zshrc");
    let shell = get("shell").trim().to_string();
    let plugins = zsh_plugins(zshrc).unwrap_or_default();
    let in_plugins = |p: &str| plugins.iter().any(|x| x == p);
    LinuxState {
        user: get("user").trim().to_string(),
        has_apt: has("apt"),
        zsh_installed: has("zsh"),
        zsh_is_default_shell: shell.ends_with("/zsh"),
        oh_my_zsh_installed: has("omz"),
        spaceship_installed: has("spaceship_dir") && zsh_theme(zshrc).as_deref() == Some("spaceship"),
        zsh_theme: zsh_theme(zshrc),
        zsh_plugins: ZshPluginsState {
            autosuggestions: has("autosuggestions_dir") && in_plugins("zsh-autosuggestions"),
            syntax_highlighting: has("syntax_dir") && in_plugins("zsh-syntax-highlighting"),
            completions: has("completions_dir") && (has_completions_fpath(zshrc) || in_plugins("zsh-completions")),
        },
        nvm_installed: has("nvm"),
        node_lts_installed: has("node"),
    }
}

async fn inspect_linux(distro: &str) -> Result<LinuxState, String> {
    let script = format!("{LINUX_PATH}{INSPECT_SCRIPT}");
    let bytes = run_raw(sh_in_distro(distro, None, &script, &[]), INSPECT_TIMEOUT, None).await?;
    Ok(parse_linux_state(&String::from_utf8_lossy(&bytes)))
}

/// Windows asks for a restart after enabling features (DISM leaves this key behind).
async fn windows_reboot_pending() -> bool {
    let mut cmd = hidden_command("reg.exe");
    cmd.args(["query", r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"]);
    matches!(tokio::time::timeout(SHORT_TIMEOUT, cmd.status()).await, Ok(Ok(s)) if s.success())
}

pub(crate) async fn inspect(app: &AppHandle, distro: Option<String>) -> Result<SetupStateInspection, String> {
    let info = wsl::get_wsl_info().await?;
    // Without WSL, wsl.exe is only a stub that fails: that means "no distros".
    let distros = wsl::list_distros().await.unwrap_or_default();
    let wsl_installed = info.wsl_version.is_some() || !distros.is_empty();

    let store = app.state::<SettingsStore>();
    let mut needs_reboot = store.get().wsl_reboot_pending;
    if needs_reboot && !windows_reboot_pending().await && wsl_installed {
        // The restart happened and WSL answers: the pending flag is done.
        store.set_wsl_reboot_pending(false)?;
        needs_reboot = false;
    }

    let linux_distros: Vec<&Distro> = distros.iter().filter(|d| !is_system_distro(&d.name)).collect();
    let selected = match distro {
        // docker-desktop selected in the Titlebar is never the setup's target.
        Some(name) => {
            validate_distro_name(&name)?;
            linux_distros.iter().any(|d| d.name == name).then_some(name)
        }
        None => None,
    }
    .or_else(|| {
        linux_distros
            .iter()
            .find(|d| d.is_default)
            .or_else(|| linux_distros.iter().find(|d| d.name.starts_with("Ubuntu")))
            .or_else(|| linux_distros.first())
            .map(|d| d.name.clone())
    });
    let distro_running = selected
        .as_deref()
        .is_some_and(|name| distros.iter().any(|d| d.name == name && d.state == DistroState::Running));
    let linux = match (&selected, distro_running) {
        (Some(name), true) => Some(inspect_linux(name).await?),
        _ => None,
    };

    Ok(SetupStateInspection {
        wsl_installed,
        wsl_version: info.wsl_version,
        needs_reboot,
        has_linux_distro: !linux_distros.is_empty(),
        windows_terminal: inspect_windows_terminal(selected.as_deref()),
        distros,
        distro: selected,
        distro_running,
        linux,
    })
}

// ---------------------------------------------------------------------------
// Windows Terminal
// ---------------------------------------------------------------------------

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// Store (stable, preview) and unpackaged installs, in this order.
fn wt_settings_candidates() -> Vec<PathBuf> {
    let Some(base) = local_app_data() else { return Vec::new() };
    vec![
        base.join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"),
        base.join(r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json"),
        base.join(r"Microsoft\Windows Terminal\settings.json"),
    ]
}

fn wt_settings_path() -> Option<PathBuf> {
    wt_settings_candidates().into_iter().find(|p| p.is_file())
}

fn wt_installed() -> bool {
    let Some(base) = local_app_data() else { return false };
    wt_settings_path().is_some()
        || base.join(r"Microsoft\WindowsApps\wt.exe").exists()
        || base.join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe").is_dir()
}

fn read_wt_settings(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Falha ao ler {}: {e}", path.display()))?;
    serde_json::from_str(&strip_jsonc(&text))
        .map_err(|e| format!("O settings.json do Windows Terminal não é um JSON válido ({e}); nada foi alterado."))
}

fn inspect_windows_terminal(distro: Option<&str>) -> WindowsTerminalState {
    let path = wt_settings_path();
    let mut state = WindowsTerminalState {
        installed: wt_installed(),
        settings_path: path.as_ref().map(|p| p.display().to_string()),
        ..Default::default()
    };
    if let (Some(path), Some(distro)) = (path, distro) {
        if let Ok(value) = read_wt_settings(&path) {
            let wt = wt_status(&value, distro);
            state.has_dracula_scheme = wt.has_dracula;
            state.profile_found = wt.profile_found;
            state.ubuntu_is_default = wt.is_default;
            state.ubuntu_uses_dracula = wt.uses_dracula;
        }
    } else if let Some(path) = wt_settings_path() {
        if let Ok(value) = read_wt_settings(&path) {
            state.has_dracula_scheme = has_scheme(&value, "Dracula");
        }
    }
    state
}

/// Windows Terminal writes JSON with comments (and tolerates trailing commas).
pub fn strip_jsonc(text: &str) -> String {
    let text = text.trim_start_matches('\u{FEFF}');
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == ',' {
            // Trailing comma: next significant char closes the object/array.
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}' | ']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

pub fn dracula_scheme() -> Value {
    json!({
        "name": "Dracula",
        "background": "#282A36",
        "foreground": "#F8F8F2",
        "cursorColor": "#F8F8F2",
        "selectionBackground": "#44475A",
        "black": "#21222C",
        "red": "#FF5555",
        "green": "#50FA7B",
        "yellow": "#F1FA8C",
        "blue": "#BD93F9",
        "purple": "#FF79C6",
        "cyan": "#8BE9FD",
        "white": "#F8F8F2",
        "brightBlack": "#6272A4",
        "brightRed": "#FF6E6E",
        "brightGreen": "#69FF94",
        "brightYellow": "#FFFFA5",
        "brightBlue": "#D6ACFF",
        "brightPurple": "#FF92DF",
        "brightCyan": "#A4FFFF",
        "brightWhite": "#FFFFFF"
    })
}

fn has_scheme(root: &Value, name: &str) -> bool {
    root.get("schemes")
        .and_then(Value::as_array)
        .is_some_and(|s| s.iter().any(|x| x.get("name").and_then(Value::as_str) == Some(name)))
}

/// `profiles.list`, or the legacy top-level `profiles` array.
fn profile_list(root: &Value) -> Option<&Vec<Value>> {
    match root.get("profiles")? {
        Value::Array(list) => Some(list),
        other => other.get("list")?.as_array(),
    }
}

fn profile_list_mut(root: &mut Value) -> Option<&mut Vec<Value>> {
    match root.get_mut("profiles")? {
        Value::Array(list) => Some(list),
        other => other.get_mut("list")?.as_array_mut(),
    }
}

/// The profile the user sees for `distro`. Both WSL's dynamic profile and the Store package's
/// one are named after the distro and one of them is usually hidden: visible ones win, then
/// WSL-sourced ones.
pub fn find_distro_profile(list: &[Value], distro: &str) -> Option<usize> {
    let score = |p: &Value| {
        let name = p.get("name").and_then(Value::as_str).unwrap_or("");
        let cmd = p.get("commandline").and_then(Value::as_str).unwrap_or("");
        let matches_name = name.eq_ignore_ascii_case(distro);
        let matches_cmd = cmd.contains("wsl") && cmd.split_whitespace().any(|t| t.eq_ignore_ascii_case(distro));
        if !matches_name && !matches_cmd {
            return None;
        }
        let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        let source = p.get("source").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        let wsl_source = source.contains("wsl") || source.contains("canonical") || matches_cmd;
        Some(((!hidden) as u8) * 2 + wsl_source as u8)
    };
    list.iter()
        .enumerate()
        .filter_map(|(i, p)| score(p).map(|s| (s, i)))
        // Highest score; on ties, the first one in the file.
        .max_by(|(sa, ia), (sb, ib)| sa.cmp(sb).then(ib.cmp(ia)))
        .map(|(_, i)| i)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WtStatus {
    pub has_dracula: bool,
    pub profile_found: bool,
    pub is_default: bool,
    pub uses_dracula: bool,
}

pub fn wt_status(root: &Value, distro: &str) -> WtStatus {
    let mut status = WtStatus { has_dracula: has_scheme(root, "Dracula"), ..Default::default() };
    if let Some(list) = profile_list(root) {
        if let Some(i) = find_distro_profile(list, distro) {
            let profile = &list[i];
            status.profile_found = true;
            let guid = profile.get("guid").and_then(Value::as_str);
            status.is_default = guid.is_some() && root.get("defaultProfile").and_then(Value::as_str) == guid;
            status.uses_dracula = profile.get("colorScheme").and_then(Value::as_str) == Some("Dracula");
        }
    }
    status
}

#[derive(Debug, PartialEq, Eq)]
pub struct WtPatch {
    pub changed: bool,
    pub profile_found: bool,
}

/// Adds the Dracula scheme (if missing), makes the distro's profile the default one and gives it
/// the Dracula scheme. Every other field the user has is kept as is.
pub fn apply_dracula(root: &mut Value, distro: &str) -> Result<WtPatch, String> {
    let obj = root.as_object_mut().ok_or("settings.json não é um objeto JSON")?;
    let mut changed = false;

    let schemes = obj.entry("schemes").or_insert_with(|| Value::Array(Vec::new()));
    let schemes = schemes.as_array_mut().ok_or("\"schemes\" não é uma lista")?;
    if !schemes.iter().any(|s| s.get("name").and_then(Value::as_str) == Some("Dracula")) {
        schemes.push(dracula_scheme());
        changed = true;
    }

    let Some(list) = profile_list_mut(root) else {
        return Ok(WtPatch { changed, profile_found: false });
    };
    let Some(i) = find_distro_profile(list, distro) else {
        return Ok(WtPatch { changed, profile_found: false });
    };
    let profile = list[i].as_object_mut().ok_or("perfil inválido no settings.json")?;
    if profile.get("colorScheme").and_then(Value::as_str) != Some("Dracula") {
        profile.insert("colorScheme".into(), Value::String("Dracula".into()));
        changed = true;
    }
    let guid = profile.get("guid").and_then(Value::as_str).map(str::to_string);
    if let Some(guid) = guid {
        let obj = root.as_object_mut().expect("checked above");
        if obj.get("defaultProfile").and_then(Value::as_str) != Some(guid.as_str()) {
            obj.insert("defaultProfile".into(), Value::String(guid));
            changed = true;
        }
    }
    Ok(WtPatch { changed, profile_found: true })
}

/// Windows Terminal's own formatting: 4 spaces.
fn to_wt_json(value: &Value) -> Result<String, String> {
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    value.serialize(&mut ser).map_err(|e| e.to_string())?;
    String::from_utf8(buf).map_err(|e| e.to_string())
}

fn backup_once(path: &Path) -> Result<Option<PathBuf>, String> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(BACKUP_SUFFIX);
    let backup = path.with_file_name(name);
    if backup.exists() {
        return Ok(None);
    }
    std::fs::copy(path, &backup).map_err(|e| format!("Falha ao criar o backup {}: {e}", backup.display()))?;
    Ok(Some(backup))
}

// ---------------------------------------------------------------------------
// ~/.zshrc and /etc/wsl.conf (pure functions)
// ---------------------------------------------------------------------------

fn is_active(line: &str) -> bool {
    let t = line.trim_start();
    !t.is_empty() && !t.starts_with('#')
}

fn unquote(value: &str) -> &str {
    let v = value.trim();
    let v = v.split(" #").next().unwrap_or(v).trim();
    v.trim_matches(|c| c == '"' || c == '\'')
}

fn join_lines(lines: &[String], trailing_newline: bool) -> String {
    let mut out = lines.join("\n");
    if trailing_newline || out.is_empty() {
        out.push('\n');
    }
    out
}

/// Last active `ZSH_THEME=` value (zsh keeps the last assignment).
pub fn zsh_theme(zshrc: &str) -> Option<String> {
    zshrc
        .lines()
        .filter(|l| is_active(l))
        .filter_map(|l| l.trim_start().strip_prefix("ZSH_THEME="))
        .last()
        .map(|v| unquote(v).to_string())
}

/// Index of the line that loads Oh-My-Zsh (`source $ZSH/oh-my-zsh.sh`).
fn omz_source_line(lines: &[String]) -> Option<usize> {
    lines.iter().position(|l| {
        let t = l.trim_start();
        is_active(l) && (t.starts_with("source ") || t.starts_with(". ")) && t.contains("oh-my-zsh.sh")
    })
}

/// Sets `ZSH_THEME="<theme>"`: replaces the active assignments, or inserts one before Oh-My-Zsh
/// is loaded. Everything else (nvm, aliases, exports) is kept.
pub fn set_zsh_theme(zshrc: &str, theme: &str) -> String {
    let trailing = zshrc.ends_with('\n');
    let mut lines: Vec<String> = zshrc.lines().map(str::to_string).collect();
    let wanted = format!("ZSH_THEME=\"{theme}\"");
    let mut found = false;
    for line in lines.iter_mut() {
        if is_active(line) && line.trim_start().starts_with("ZSH_THEME=") {
            let indent = &line[..line.len() - line.trim_start().len()];
            *line = format!("{indent}{wanted}");
            found = true;
        }
    }
    if !found {
        let at = omz_source_line(&lines).unwrap_or(lines.len());
        lines.insert(at, wanted);
    }
    join_lines(&lines, trailing)
}

/// Line range `[start, end]` of the active `plugins=( ... )` assignment.
fn plugins_block(lines: &[String]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| is_active(l) && l.trim_start().starts_with("plugins=("))?;
    let end = (start..lines.len()).find(|&i| {
        let text = if i == start { lines[i].split_once('(').map(|x| x.1).unwrap_or("") } else { &lines[i] };
        text.split('#').next().unwrap_or("").contains(')')
    })?;
    Some((start, end))
}

pub fn zsh_plugins(zshrc: &str) -> Option<Vec<String>> {
    let lines: Vec<String> = zshrc.lines().map(str::to_string).collect();
    let (start, end) = plugins_block(&lines)?;
    let mut joined = String::new();
    for line in &lines[start..=end] {
        joined.push_str(line.split('#').next().unwrap_or(""));
        joined.push(' ');
    }
    let inner = joined.split_once('(')?.1;
    let inner = inner.split(')').next().unwrap_or("");
    Some(inner.split_whitespace().map(str::to_string).collect())
}

/// Adds `wanted` to `plugins=(...)` without dropping the user's plugins.
/// zsh-syntax-highlighting must be the last one loaded (its README), so it is moved to the end.
pub fn ensure_plugins(zshrc: &str, wanted: &[&str]) -> String {
    let trailing = zshrc.ends_with('\n');
    let mut lines: Vec<String> = zshrc.lines().map(str::to_string).collect();
    let current = zsh_plugins(zshrc);
    let mut list: Vec<String> = current.clone().unwrap_or_else(|| vec!["git".into()]);
    for p in wanted {
        if !list.iter().any(|x| x == p) {
            list.push((*p).to_string());
        }
    }
    const LAST: &str = "zsh-syntax-highlighting";
    if let Some(i) = list.iter().position(|x| x == LAST) {
        let item = list.remove(i);
        list.push(item);
    }
    if current.as_ref() == Some(&list) {
        return zshrc.to_string();
    }

    match plugins_block(&lines) {
        Some((start, end)) if start == end => {
            let line = &lines[start];
            let indent = &line[..line.len() - line.trim_start().len()];
            let after = line.split_once('(').map(|x| x.1).unwrap_or("");
            let tail = after.split_once(')').map(|x| x.1).unwrap_or("");
            lines[start] = format!("{indent}plugins=({}){tail}", list.join(" "));
        }
        Some((start, end)) => {
            let indent = lines[start + 1..end]
                .iter()
                .find(|l| !l.trim().is_empty())
                .map(|l| l[..l.len() - l.trim_start().len()].to_string())
                .unwrap_or_else(|| "  ".into());
            let lead = &lines[start][..lines[start].len() - lines[start].trim_start().len()];
            let mut block = vec![format!("{lead}plugins=(")];
            block.extend(list.iter().map(|p| format!("{indent}{p}")));
            block.push(format!("{lead})"));
            lines.splice(start..=end, block);
        }
        None => {
            let at = omz_source_line(&lines).unwrap_or(lines.len());
            lines.insert(at, format!("plugins=({})", list.join(" ")));
        }
    }
    join_lines(&lines, trailing)
}

pub fn has_completions_fpath(zshrc: &str) -> bool {
    zshrc.lines().any(|l| is_active(l) && l.contains("fpath") && l.contains("zsh-completions/src"))
}

/// zsh-completions goes in `fpath` before Oh-My-Zsh runs compinit (its README: as a regular
/// plugin it breaks the completion cache).
pub fn ensure_completions_fpath(zshrc: &str) -> String {
    if has_completions_fpath(zshrc) {
        return zshrc.to_string();
    }
    let trailing = zshrc.ends_with('\n');
    let mut lines: Vec<String> = zshrc.lines().map(str::to_string).collect();
    let at = omz_source_line(&lines).unwrap_or(lines.len());
    lines.insert(at, COMPLETIONS_FPATH.to_string());
    join_lines(&lines, trailing)
}

/// An existing ~/.zshrc kept by the installer (KEEP_ZSHRC=yes) does not load Oh-My-Zsh yet:
/// prepend the standard header, keeping the user's lines below it.
pub fn ensure_omz_loaded(zshrc: &str) -> String {
    let lines: Vec<String> = zshrc.lines().map(str::to_string).collect();
    if omz_source_line(&lines).is_some() {
        return zshrc.to_string();
    }
    let mut out = String::from(
        "# Oh-My-Zsh (adicionado pelo WSL Symbiosis)\nexport ZSH=\"$HOME/.oh-my-zsh\"\nZSH_THEME=\"robbyrussell\"\nplugins=(git)\nsource $ZSH/oh-my-zsh.sh\n",
    );
    if !zshrc.trim().is_empty() {
        out.push('\n');
        out.push_str(zshrc);
        if !zshrc.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// `[user]\ndefault=<user>` in /etc/wsl.conf, keeping every other section (e.g. `[boot] systemd`).
pub fn set_wsl_conf_default_user(conf: &str, user: &str) -> String {
    let trailing = conf.ends_with('\n') || conf.is_empty();
    let mut lines: Vec<String> = conf.lines().map(str::to_string).collect();
    let wanted = format!("default={user}");
    let header = lines.iter().position(|l| l.trim().eq_ignore_ascii_case("[user]"));
    match header {
        Some(h) => {
            let end = (h + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('[')).unwrap_or(lines.len());
            let existing = (h + 1..end).find(|&i| {
                let t = lines[i].trim_start();
                !t.starts_with('#') && t.split('=').next().is_some_and(|k| k.trim().eq_ignore_ascii_case("default"))
            });
            match existing {
                Some(i) => lines[i] = wanted,
                None => lines.insert(h + 1, wanted),
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[user]".into());
            lines.push(wanted);
        }
    }
    join_lines(&lines, trailing)
}

/// Linux user name for the new distro (Debian's NAME_REGEX, plus a length limit).
pub fn validate_linux_username(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if name.len() > 32 || !first_ok || !rest_ok {
        return Err("Usuário inválido: use letras minúsculas, números, _ ou -, começando por letra (até 32).".into());
    }
    if name == "root" {
        return Err("Escolha um usuário diferente de root.".into());
    }
    Ok(())
}

pub fn validate_linux_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("Informe uma senha para o usuário Linux.".into());
    }
    if password.chars().count() > 128 || password.chars().any(char::is_control) {
        return Err("A senha não pode ter quebras de linha nem passar de 128 caracteres.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Progress events and process streaming
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionProgress {
    pub step_id: String,
    /// running | done | warning | skipped | error
    pub status: String,
    pub message: Option<String>,
    pub log_line: Option<String>,
}

/// Where progress goes: the `provision-progress` event in the app, stdout in live tests.
pub(crate) type Sink<'a> = &'a (dyn Fn(ProvisionProgress) + Send + Sync);

struct Reporter<'a> {
    sink: Sink<'a>,
    step: &'a str,
}

impl Reporter<'_> {
    fn emit(&self, status: &str, message: Option<String>, log_line: Option<String>) {
        (self.sink)(ProvisionProgress { step_id: self.step.into(), status: status.into(), message, log_line });
    }
    fn running(&self, message: impl Into<String>) {
        self.emit("running", Some(message.into()), None);
    }
    fn log(&self, line: impl Into<String>) {
        self.emit("running", None, Some(line.into()));
    }
}

async fn pump(reader: impl AsyncRead + Unpin, tx: tokio::sync::mpsc::UnboundedSender<String>) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf).replace('\0', "");
                // Progress bars redraw with \r: keep the last state of the line.
                let text = strip_ansi(text.trim_end().rsplit('\r').next().unwrap_or(""));
                if !text.trim().is_empty() {
                    let _ = tx.send(text.trim_end().to_string());
                }
            }
        }
    }
}

/// Runs a process streaming stdout/stderr lines as log events. Fails with the last lines.
async fn stream(rep: &Reporter<'_>, mut cmd: Command, timeout: Duration, stdin: Option<Vec<u8>>) -> Result<(), String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| format!("Falha ao iniciar o processo: {e}"))?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        use tokio::io::AsyncWriteExt;
        let _ = pipe.write_all(&data).await;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let out = child.stdout.take().map(|r| tokio::spawn(pump(r, tx.clone())));
    let err = child.stderr.take().map(|r| tokio::spawn(pump(r, tx.clone())));
    drop(tx);

    let mut tail: Vec<String> = Vec::new();
    let work = async {
        while let Some(line) = rx.recv().await {
            rep.log(line.clone());
            tail.push(line);
            if tail.len() > 6 {
                tail.remove(0);
            }
        }
        child.wait().await
    };
    let status = tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| "Tempo esgotado: a etapa demorou demais e foi interrompida.".to_string())?
        .map_err(|e| e.to_string())?;
    for task in [out, err].into_iter().flatten() {
        let _ = task.await;
    }
    if status.success() {
        Ok(())
    } else {
        let detail = tail.join("\n");
        Err(if detail.is_empty() { format!("Falhou (código {:?})", status.code()) } else { detail })
    }
}

/// `sh -c` in the distro as `user` (None = default user), Linux-only PATH, stderr merged.
fn linux_script(distro: &str, user: Option<&str>, body: &str, args: &[&str]) -> Command {
    let script = format!("{LINUX_PATH}exec 2>&1\n{body}");
    sh_in_distro(distro, user, &script, args)
}

async fn read_user_file(distro: &str, path_expr: &str) -> Result<String, String> {
    let script = format!("{LINUX_PATH}f={path_expr}; [ -f \"$f\" ] && cat \"$f\"; exit 0");
    let bytes = run_raw(sh_in_distro(distro, None, &script, &[]), SHORT_TIMEOUT, None).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Writes through stdin, backing up the original once (`<file>.symbiosis.bak`).
async fn write_user_file(distro: &str, user: Option<&str>, path_expr: &str, content: &str) -> Result<(), String> {
    let script = format!(
        "{LINUX_PATH}f={path_expr}; [ -f \"$f\" ] && [ ! -e \"$f{BACKUP_SUFFIX}\" ] && cp -p \"$f\" \"$f{BACKUP_SUFFIX}\"; cat > \"$f\""
    );
    run_raw(sh_in_distro(distro, user, &script, &[]), SHORT_TIMEOUT, Some(content.as_bytes())).await.map(|_| ())
}

/// Reads ~/.zshrc, applies `patch`, writes only if something changed.
async fn patch_zshrc(rep: &Reporter<'_>, distro: &str, what: &str, patch: impl Fn(&str) -> String) -> Result<(), String> {
    let current = read_user_file(distro, "\"$HOME/.zshrc\"").await?;
    let next = patch(&current);
    if next == current {
        rep.log(format!("~/.zshrc: {what} já estava configurado"));
        return Ok(());
    }
    write_user_file(distro, None, "\"$HOME/.zshrc\"", &next).await?;
    rep.log(format!("~/.zshrc: {what} (backup em ~/.zshrc{BACKUP_SUFFIX})"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------

/// Root. `$@` = commands that must exist (package name = command name: zsh, git, curl).
/// A failing `apt-get update` doesn't stop the install: images ship with repositories that went
/// offline (Debian's bullseye-backports returns 404) while the main mirrors still work.
const APT_SCRIPT: &str = r#"export DEBIAN_FRONTEND=noninteractive
missing=
for p in "$@"; do command -v "$p" >/dev/null 2>&1 || missing="$missing $p"; done
if [ -z "$missing" ]; then echo "$* já instalado(s)"; exit 0; fi
command -v apt-get >/dev/null 2>&1 || { echo "Faltam:$missing, e esta distro não usa apt-get (o setup é para Ubuntu/Debian)."; exit 3; }
apt-get -o DPkg::Lock::Timeout=120 update || echo "aviso: o apt-get update teve erros (algum repositório fora do ar); instalando mesmo assim"
apt-get -o DPkg::Lock::Timeout=120 install -y $missing"#;

/// Installs the missing commands (as root) before a module that needs them.
async fn ensure_packages(rep: &Reporter<'_>, distro: &str, packages: &[&str]) -> Result<(), String> {
    stream(rep, linux_script(distro, Some("root"), APT_SCRIPT, packages), LONG_TIMEOUT, None).await
}

const OMZ_SCRIPT: &str = r#"if [ -d "$HOME/.oh-my-zsh" ]; then echo "Oh-My-Zsh já instalado em ~/.oh-my-zsh"; exit 0; fi
t=$(mktemp) || exit 1
curl -fsSL "$1" -o "$t" || { rm -f "$t"; echo "Não foi possível baixar o instalador do Oh-My-Zsh"; exit 4; }
RUNZSH=no CHSH=no KEEP_ZSHRC=yes sh "$t" --unattended
s=$?; rm -f "$t"; exit $s"#;

const CHSH_SCRIPT: &str = r#"z=$(command -v zsh) || { echo "zsh não encontrado"; exit 3; }
cur=$(getent passwd "$1" | cut -d: -f7)
if [ "$cur" = "$z" ]; then echo "Shell padrão de $1 já é $z"; exit 0; fi
usermod -s "$z" "$1" && echo "Shell padrão de $1: $z""#;

/// `$1` = destination folder under $ZSH_CUSTOM, `$2` = repository.
const CLONE_SCRIPT: &str = r#"[ -d "$HOME/.oh-my-zsh" ] || { echo "Oh-My-Zsh não está instalado: marque também o módulo Zsh + Oh-My-Zsh."; exit 3; }
d=$HOME/.oh-my-zsh/custom/$1
if [ -d "$d" ]; then echo "$1 já existe"; exit 0; fi
mkdir -p "$(dirname "$d")"
GIT_TERMINAL_PROMPT=0 git clone --depth=1 "$2" "$d""#;

const SPACESHIP_LINK: &str = r#"c=$HOME/.oh-my-zsh/custom/themes
ln -sf "$c/spaceship-prompt/spaceship.zsh-theme" "$c/spaceship.zsh-theme" && echo "link spaceship.zsh-theme ok""#;

const NVM_SCRIPT: &str = r#"export NVM_DIR="$HOME/.nvm"
if [ ! -s "$NVM_DIR/nvm.sh" ]; then
  t=$(mktemp) || exit 1
  curl -fsSL "$1" -o "$t" || { rm -f "$t"; echo "Não foi possível baixar o instalador do nvm"; exit 4; }
  PROFILE=/dev/null bash "$t"; s=$?; rm -f "$t"; [ $s -eq 0 ] || exit $s
fi
for f in "$HOME/.zshrc" "$HOME/.bashrc"; do
  if [ -f "$f" ] && ! grep -q 'NVM_DIR' "$f"; then
    [ -e "$f.symbiosis.bak" ] || cp -p "$f" "$f.symbiosis.bak"
    printf '\n# nvm (adicionado pelo WSL Symbiosis)\nexport NVM_DIR="$HOME/.nvm"\n[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"\n[ -s "$NVM_DIR/bash_completion" ] && . "$NVM_DIR/bash_completion"\n' >> "$f"
    echo "nvm carregado em $f"
  fi
done
. "$NVM_DIR/nvm.sh" || exit 1
nvm install --lts && nvm alias default 'lts/*'"#;

fn bash_in_distro(distro: &str, body: &str, args: &[&str]) -> Command {
    let script = format!("{LINUX_PATH}exec 2>&1\n{body}");
    let mut cmd = wsl_command();
    cmd.args(["-d", distro, "--exec", "bash", "-c", &script, "bash"]).args(args);
    cmd
}

async fn default_user(distro: &str) -> Result<String, String> {
    let user = run(sh_in_distro(distro, None, "id -un", &[]), SHORT_TIMEOUT).await?.trim().to_string();
    if user.is_empty() { Err("Não foi possível descobrir o usuário padrão da distro".into()) } else { Ok(user) }
}

async fn module_windows_terminal(rep: &Reporter<'_>, distro: &str) -> Result<String, String> {
    if !wt_installed() {
        rep.running("Instalando o Windows Terminal (winget)…");
        let mut cmd = hidden_command("winget");
        cmd.args([
            "install", "--id", "Microsoft.WindowsTerminal", "-e",
            "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity",
        ]);
        stream(rep, cmd, LONG_TIMEOUT, None).await.map_err(|e| format!("winget não conseguiu instalar o Windows Terminal:\n{e}"))?;
    }
    let Some(path) = wt_settings_path() else {
        return Err("O Windows Terminal ainda não criou o settings.json: abra-o uma vez e rode este módulo de novo.".into());
    };
    apply_wt_file(rep, &path, distro)
}

/// Patches one Windows Terminal settings.json (backup once, atomic write, untouched if already set).
fn apply_wt_file(rep: &Reporter<'_>, path: &Path, distro: &str) -> Result<String, String> {
    rep.running("Aplicando o tema Dracula e o perfil padrão…");
    let path = path.to_path_buf();
    let mut value = read_wt_settings(&path)?;
    let patch = apply_dracula(&mut value, distro)?;
    if patch.changed {
        if let Some(backup) = backup_once(&path)? {
            rep.log(format!("backup: {}", backup.display()));
        }
        let text = to_wt_json(&value)?;
        let tmp = path.with_extension("json.symbiosis.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("Falha ao gravar {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("Falha ao gravar {}: {e}", path.display()))?;
        rep.log(format!("gravado: {}", path.display()));
    } else {
        rep.log("settings.json já estava configurado");
    }
    if patch.profile_found {
        Ok(format!("{distro} é o perfil padrão, com o tema Dracula"))
    } else {
        Err(format!(
            "Tema Dracula adicionado, mas o Windows Terminal ainda não tem um perfil para {distro}: abra o Terminal uma vez e rode de novo."
        ))
    }
}

async fn module_zsh(rep: &Reporter<'_>, distro: &str) -> Result<String, String> {
    rep.running("Instalando zsh, git e curl (apt)…");
    ensure_packages(rep, distro, &["zsh", "git", "curl"]).await?;
    rep.running("Instalando o Oh-My-Zsh…");
    stream(rep, linux_script(distro, None, OMZ_SCRIPT, &[OMZ_INSTALLER]), NETWORK_TIMEOUT, None).await?;
    patch_zshrc(rep, distro, "Oh-My-Zsh carregado", ensure_omz_loaded).await?;
    rep.running("Definindo o zsh como shell padrão…");
    let user = default_user(distro).await?;
    stream(rep, linux_script(distro, Some("root"), CHSH_SCRIPT, &[&user]), SHORT_TIMEOUT, None).await?;
    Ok("zsh + Oh-My-Zsh prontos (abra um terminal novo)".into())
}

async fn module_spaceship(rep: &Reporter<'_>, distro: &str) -> Result<String, String> {
    ensure_packages(rep, distro, &["git"]).await?;
    rep.running("Baixando o Spaceship Prompt…");
    let args = ["themes/spaceship-prompt", "https://github.com/spaceship-prompt/spaceship-prompt.git"];
    stream(rep, linux_script(distro, None, CLONE_SCRIPT, &args), NETWORK_TIMEOUT, None).await?;
    stream(rep, linux_script(distro, None, SPACESHIP_LINK, &[]), SHORT_TIMEOUT, None).await?;
    patch_zshrc(rep, distro, "ZSH_THEME=\"spaceship\"", |z| set_zsh_theme(z, "spaceship")).await?;
    Ok("Tema Spaceship ativo".into())
}

async fn module_plugins(rep: &Reporter<'_>, distro: &str) -> Result<String, String> {
    ensure_packages(rep, distro, &["git"]).await?;
    for (name, repo) in [
        ("zsh-autosuggestions", "https://github.com/zsh-users/zsh-autosuggestions"),
        ("zsh-syntax-highlighting", "https://github.com/zsh-users/zsh-syntax-highlighting"),
        ("zsh-completions", "https://github.com/zsh-users/zsh-completions"),
    ] {
        rep.running(format!("Baixando {name}…"));
        let dest = format!("plugins/{name}");
        stream(rep, linux_script(distro, None, CLONE_SCRIPT, &[&dest, repo]), NETWORK_TIMEOUT, None).await?;
    }
    rep.running("Atualizando plugins=(…) no ~/.zshrc…");
    patch_zshrc(rep, distro, "plugins autosuggestions + syntax-highlighting", |z| {
        ensure_plugins(z, &["zsh-autosuggestions", "zsh-syntax-highlighting"])
    })
    .await?;
    patch_zshrc(rep, distro, "fpath do zsh-completions", ensure_completions_fpath).await?;
    Ok("Plugins ativos (autosuggestions, syntax highlighting, completions)".into())
}

async fn module_nvm(rep: &Reporter<'_>, distro: &str) -> Result<String, String> {
    ensure_packages(rep, distro, &["curl"]).await?;
    rep.running("Instalando o nvm e o Node.js LTS…");
    stream(rep, bash_in_distro(distro, NVM_SCRIPT, &[NVM_INSTALLER]), LONG_TIMEOUT, None).await?;
    Ok("nvm + Node.js LTS (padrão) instalados".into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub step_id: String,
    pub status: String,
    pub message: String,
}

pub(crate) async fn run_modules(sink: Sink<'_>, distro: &str, modules: &[String]) -> Result<Vec<StepResult>, String> {
    validate_distro_name(distro)?;
    if is_system_distro(distro) {
        return Err(format!("{distro} é uma distro interna de ferramenta (Docker/Rancher/Podman): o ambiente não é configurado nela."));
    }
    if let Some(bad) = modules.iter().find(|m| !MODULES.contains(&m.as_str())) {
        return Err(format!("Módulo desconhecido: {bad}"));
    }

    let mut results = Vec::new();
    // Fixed order: zsh before its theme/plugins, whatever order the UI sent.
    for id in MODULES.iter().filter(|id| modules.iter().any(|m| m == *id)) {
        let rep = Reporter { sink, step: id };
        rep.running("Iniciando…");
        let outcome = match *id {
            "windows_terminal_dracula" => module_windows_terminal(&rep, distro).await,
            "zsh_ohmyzsh" => module_zsh(&rep, distro).await,
            "spaceship_theme" => module_spaceship(&rep, distro).await,
            "zsh_plugins" => module_plugins(&rep, distro).await,
            _ => module_nvm(&rep, distro).await,
        };
        let (status, message) = match outcome {
            Ok(msg) => ("done", msg),
            // Partial success (scheme added, profile still missing) is a warning, not a failure.
            Err(msg) if *id == "windows_terminal_dracula" && msg.starts_with("Tema Dracula adicionado") => ("warning", msg),
            Err(msg) => ("error", msg),
        };
        rep.emit(status, Some(message.clone()), None);
        results.push(StepResult { step_id: id.to_string(), status: status.into(), message });
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// WSL engine and distro install
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslEnableResult {
    pub needs_reboot: bool,
    pub message: String,
}

/// Runs elevated (one UAC prompt). Writes the exit codes to `$out`: DISM 3010 = restart needed.
const ENABLE_WSL_PS1: &str = r#"param($out)
$codes = @()
& dism.exe /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart | Out-Null
$codes += $LASTEXITCODE
& dism.exe /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart | Out-Null
$codes += $LASTEXITCODE
& wsl.exe --install --no-distribution | Out-Null
$codes += $LASTEXITCODE
& wsl.exe --set-default-version 2 | Out-Null
$codes += $LASTEXITCODE
($codes -join ' ') | Set-Content -Path $out -Encoding ascii
"#;

pub(crate) async fn enable_wsl(app: &AppHandle) -> Result<WslEnableResult, String> {
    let dir = std::env::temp_dir().join(format!("wsl-symbiosis-setup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ps1 = dir.join("enable-wsl.ps1");
    let out = dir.join("enable-wsl.txt");
    let _ = std::fs::remove_file(&out);
    std::fs::write(&ps1, ENABLE_WSL_PS1).map_err(|e| e.to_string())?;

    // Windows paths can't contain `"`; `'` is doubled for the PowerShell single-quoted string.
    let inner = format!("-NoProfile -ExecutionPolicy Bypass -File \"{}\" \"{}\"", ps1.display(), out.display());
    let command = format!(
        "Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '{}'",
        inner.replace('\'', "''")
    );
    let mut cmd = hidden_command("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &command]);
    let output = tokio::time::timeout(LONG_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "Tempo esgotado habilitando o WSL".to_string())?
        .map_err(|e| format!("Falha ao executar o PowerShell: {e}"))?;
    let codes = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    if !output.status.success() || codes.trim().is_empty() {
        let err = wsl::decode_output(&output.stderr);
        if err.contains("cancel") || codes.trim().is_empty() {
            return Err("A permissão de administrador foi negada (ou a janela do UAC foi fechada). Nada foi alterado.".into());
        }
        return Err(err.trim().to_string());
    }

    let codes: Vec<i64> = codes.split_whitespace().filter_map(|c| c.parse().ok()).collect();
    let dism_failed = codes.iter().take(2).any(|&c| c != 0 && c != 3010);
    if dism_failed {
        return Err(format!("O DISM não conseguiu habilitar os recursos do Windows (códigos {codes:?})."));
    }
    let needs_reboot = codes.iter().take(2).any(|&c| c == 3010) || windows_reboot_pending().await;
    if needs_reboot {
        app.state::<SettingsStore>().set_wsl_reboot_pending(true)?;
    }
    Ok(WslEnableResult {
        needs_reboot,
        message: if needs_reboot {
            "WSL habilitado. Reinicie o Windows e abra o WSL Symbiosis de novo para continuar.".into()
        } else {
            "WSL2 habilitado e pronto.".into()
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineDistro {
    pub name: String,
    pub friendly_name: String,
}

/// `wsl --list --online`: the intro text is localized, the table header (`NAME  FRIENDLY NAME`)
/// is not. The catalog changes with the WSL version (e.g. no `Ubuntu-24.04` on some builds),
/// so the installer offers what this machine actually has.
pub fn parse_online_distros(text: &str) -> Vec<OnlineDistro> {
    let mut lines = text.lines().skip_while(|l| !l.trim_start().starts_with("NAME"));
    lines.next();
    lines
        .filter_map(|l| {
            let l = l.trim();
            let (name, friendly) = l.split_once(char::is_whitespace)?;
            validate_distro_name(name).ok()?;
            Some(OnlineDistro { name: name.into(), friendly_name: friendly.trim().into() })
        })
        .collect()
}

/// What the installer offers: Ubuntu only. The modules also work on an up-to-date Debian, but
/// the catalog's Debian image can be an EOL release (bullseye, on WSL 2.3) whose mirrors 404,
/// so a fresh install would fail at the first `apt-get install`.
pub fn is_installable(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("ubuntu")
}

pub(crate) async fn online_distros() -> Result<Vec<OnlineDistro>, String> {
    let mut cmd = wsl_command();
    cmd.args(["--list", "--online"]);
    let text = run(cmd, SHORT_TIMEOUT).await?;
    let list: Vec<OnlineDistro> = parse_online_distros(&text).into_iter().filter(|d| is_installable(&d.name)).collect();
    if list.is_empty() { Err("O catálogo do WSL não trouxe nenhum Ubuntu".into()) } else { Ok(list) }
}

/// Store-era distros register only on first launch: `ubuntu.exe install --root` does it
/// without the interactive user prompt.
fn appx_launcher(distro: &str) -> String {
    format!("{}.exe", distro.to_ascii_lowercase().replace(['-', '.'], ""))
}

async fn is_registered(distro: &str) -> bool {
    wsl::list_distros().await.unwrap_or_default().iter().any(|d| d.name.eq_ignore_ascii_case(distro))
}

pub(crate) async fn install_distro(sink: Sink<'_>, distro: &str, username: &str, password: &str) -> Result<(), String> {
    validate_distro_name(distro)?;
    validate_linux_username(username)?;
    validate_linux_password(password)?;
    if is_system_distro(distro) {
        return Err(format!("{distro} não é uma distro de desenvolvimento"));
    }
    let rep = Reporter { sink, step: "install_distro" };

    if is_registered(distro).await {
        rep.log(format!("{distro} já está instalada"));
    } else {
        rep.running(format!("Baixando e instalando {distro} (pode levar alguns minutos)…"));
        let mut cmd = wsl_command();
        cmd.args(["--install", "-d", distro, "--no-launch", "--web-download"]);
        let result = stream(&rep, cmd, DISTRO_INSTALL_TIMEOUT, None).await;
        if !is_registered(distro).await {
            // Older Store packages: installed but not registered until first launch.
            rep.log("registrando a distro…");
            let mut launcher = hidden_command(appx_launcher(distro));
            launcher.args(["install", "--root"]);
            let registered = stream(&rep, launcher, DISTRO_INSTALL_TIMEOUT, None).await;
            if !is_registered(distro).await {
                let detail = result.err().or(registered.err()).unwrap_or_default();
                let msg = format!("Não foi possível instalar {distro}.\n{detail}");
                rep.emit("error", Some(msg.clone()), None);
                return Err(msg);
            }
        }
    }

    rep.running(format!("Criando o usuário {username}…"));
    let create = r#"id "$1" >/dev/null 2>&1 && echo "usuário $1 já existe" || useradd -m -G sudo -s /bin/bash "$1" || exit $?
chpasswd && echo "senha definida""#;
    // stdin = "user:password\n" for chpasswd; the password never shows in args or logs.
    let secret = format!("{username}:{password}\n").into_bytes();
    let step = stream(&rep, linux_script(distro, Some("root"), create, &[username]), SHORT_TIMEOUT, Some(secret)).await;
    if let Err(e) = step {
        rep.emit("error", Some(e.clone()), None);
        return Err(e);
    }

    let conf_bytes = run_raw(
        sh_in_distro(distro, Some("root"), "[ -f /etc/wsl.conf ] && cat /etc/wsl.conf; exit 0", &[]),
        SHORT_TIMEOUT,
        None,
    )
    .await?;
    let conf = set_wsl_conf_default_user(&String::from_utf8_lossy(&conf_bytes), username);
    run_raw(sh_in_distro(distro, Some("root"), "cat > /etc/wsl.conf", &[]), SHORT_TIMEOUT, Some(conf.as_bytes())).await?;
    rep.log(format!("/etc/wsl.conf: [user] default={username}"));

    // wsl.conf is read when the distro boots: restart it and check the user really is the default.
    let mut terminate = wsl_command();
    terminate.args(["--terminate", distro]);
    run(terminate, SHORT_TIMEOUT).await?;
    let user = default_user(distro).await?;
    if user != username {
        let msg = format!("A distro abriu com o usuário {user}, não {username}: confira o /etc/wsl.conf.");
        rep.emit("error", Some(msg.clone()), None);
        return Err(msg);
    }
    rep.log(format!("usuário padrão confirmado: {user}"));

    // The new distro becomes WSL's default unless the user already has another real one there
    // (Docker Desktop often leaves docker-desktop as the default).
    let default = wsl::list_distros().await.unwrap_or_default().into_iter().find(|d| d.is_default);
    if default.as_ref().is_none_or(|d| is_system_distro(&d.name)) {
        let mut set_default = wsl_command();
        set_default.args(["--set-default", distro]);
        match run(set_default, SHORT_TIMEOUT).await {
            Ok(_) => rep.log(format!("{distro} agora é a distro padrão do WSL")),
            Err(e) => rep.log(format!("não foi possível torná-la padrão do WSL: {e}")),
        }
    }
    rep.emit("done", Some(format!("{distro} instalada; {username} é o usuário padrão")), None);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn inspect_setup_state(app: AppHandle, distro: Option<String>) -> Result<SetupStateInspection, String> {
    inspect(&app, distro).await
}

#[tauri::command]
pub async fn enable_wsl_engine(app: AppHandle) -> Result<WslEnableResult, String> {
    enable_wsl(&app).await
}

/// Startup decision for the full-window setup (see `setup_required`).
#[tauri::command]
pub async fn should_open_setup(app: AppHandle) -> Result<bool, String> {
    let settings = app.state::<SettingsStore>().get();
    // The app waits for this answer before drawing: a WSL service slow at boot must not hold
    // the window blank (unknown list = the distro rule doesn't apply).
    let distros = tokio::time::timeout(Duration::from_secs(4), wsl::list_distros()).await.ok().and_then(Result::ok);
    Ok(setup_required(&settings, distros.as_deref()))
}

#[tauri::command]
pub async fn list_online_distros() -> Result<Vec<OnlineDistro>, String> {
    online_distros().await
}

/// Progress → `provision-progress` events for the setup screen.
fn event_sink(app: &AppHandle) -> impl Fn(ProvisionProgress) + Send + Sync + '_ {
    move |p| {
        let _ = app.emit("provision-progress", p);
    }
}

#[tauri::command]
pub async fn install_ubuntu_distro(app: AppHandle, distro_name: String, username: String, password: String) -> Result<(), String> {
    let lock = app.state::<ProvisionLock>();
    let _guard = lock.0.try_lock().map_err(|_| "Já existe uma configuração em andamento".to_string())?;
    install_distro(&event_sink(&app), &distro_name, &username, &password).await
}

#[tauri::command]
pub async fn run_provisioning_tasks(app: AppHandle, distro: String, modules: Vec<String>) -> Result<Vec<StepResult>, String> {
    let lock = app.state::<ProvisionLock>();
    let _guard = lock.0.try_lock().map_err(|_| "Já existe uma configuração em andamento".to_string())?;
    run_modules(&event_sink(&app), &distro, &modules).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const WT_SETTINGS: &str = r##"// This file was initially generated by Windows Terminal
{
    "$help": "https://aka.ms/terminal-documentation",
    "defaultProfile": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}",
    "copyOnSelect": true, // user setting
    "profiles": {
        "defaults": { "font": { "face": "CaskaydiaCove Nerd Font" } },
        "list": [
            { "guid": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}", "name": "Windows PowerShell", "hidden": false },
            { "guid": "{2c4de342-38b7-51cf-b940-2309a097f518}", "name": "Ubuntu", "source": "Windows.Terminal.Wsl", "hidden": true },
            { "guid": "{51855cb2-8cce-5362-8f54-464b92b32386}", "name": "Ubuntu", "source": "CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc", "hidden": false, "fontSize": 13 },
        ]
    },
    "schemes": [ { "name": "Campbell", "background": "#0C0C0C" } ],
    "actions": [ { "command": "paste", "keys": "ctrl+v" } ]
}"##;

    fn wt() -> Value {
        serde_json::from_str(&strip_jsonc(WT_SETTINGS)).expect("jsonc parses")
    }

    #[test]
    fn strips_jsonc_but_not_strings() {
        let v: Value = serde_json::from_str(&strip_jsonc(
            "{ /* c */ \"a\": \"http://x//y\", // c\n \"b\": [1, 2,], \"c\": \"/* not */\", }",
        ))
        .unwrap();
        assert_eq!(v["a"], "http://x//y");
        assert_eq!(v["b"], json!([1, 2]));
        assert_eq!(v["c"], "/* not */");
    }

    #[test]
    fn dracula_patch_is_non_destructive() {
        let mut v = wt();
        assert_eq!(wt_status(&v, "Ubuntu"), WtStatus { profile_found: true, ..Default::default() });
        let patch = apply_dracula(&mut v, "Ubuntu").unwrap();
        assert_eq!(patch, WtPatch { changed: true, profile_found: true });

        // The visible Ubuntu profile (Store package) is picked over WSL's hidden one.
        assert_eq!(v["defaultProfile"], "{51855cb2-8cce-5362-8f54-464b92b32386}");
        let list = v["profiles"]["list"].as_array().unwrap();
        assert_eq!(list[2]["colorScheme"], "Dracula");
        assert_eq!(list[2]["fontSize"], 13, "existing profile fields are kept");
        assert!(list[1].get("colorScheme").is_none());
        assert!(list[0].get("colorScheme").is_none());
        // Everything else untouched.
        assert_eq!(v["copyOnSelect"], true);
        assert_eq!(v["profiles"]["defaults"]["font"]["face"], "CaskaydiaCove Nerd Font");
        assert_eq!(v["actions"][0]["keys"], "ctrl+v");
        let schemes = v["schemes"].as_array().unwrap();
        assert_eq!(schemes.len(), 2);
        assert_eq!(schemes[0]["name"], "Campbell");
        assert_eq!(schemes[1], dracula_scheme());
        // Key order is preserved (serde_json preserve_order).
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["$help", "defaultProfile", "copyOnSelect", "profiles", "schemes", "actions"]);

        assert_eq!(
            wt_status(&v, "Ubuntu"),
            WtStatus { has_dracula: true, profile_found: true, is_default: true, uses_dracula: true }
        );
        // Idempotent: a second run changes nothing and doesn't duplicate the scheme.
        let again = apply_dracula(&mut v, "Ubuntu").unwrap();
        assert_eq!(again, WtPatch { changed: false, profile_found: true });
        assert_eq!(v["schemes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn dracula_patch_without_profile_or_schemes() {
        let mut v = json!({ "profiles": { "list": [ { "guid": "{x}", "name": "PowerShell" } ] } });
        let patch = apply_dracula(&mut v, "Ubuntu-24.04").unwrap();
        assert_eq!(patch, WtPatch { changed: true, profile_found: false });
        assert_eq!(v["schemes"][0]["name"], "Dracula");
        assert!(v.get("defaultProfile").is_none());

        // Legacy layout: "profiles" is the list itself; match by commandline.
        let mut v = json!({ "profiles": [ { "guid": "{u}", "name": "Dev", "commandline": "wsl.exe -d Ubuntu-24.04" } ] });
        assert!(apply_dracula(&mut v, "Ubuntu-24.04").unwrap().profile_found);
        assert_eq!(v["defaultProfile"], "{u}");
        assert!(apply_dracula(&mut json!([1]), "Ubuntu").is_err());
    }

    #[test]
    fn writes_wt_json_with_four_spaces() {
        let text = to_wt_json(&json!({ "a": { "b": 1 } })).unwrap();
        assert_eq!(text, "{\n    \"a\": {\n        \"b\": 1\n    }\n}");
    }

    const ZSHRC: &str = r#"# Path to your oh-my-zsh installation.
export ZSH="$HOME/.oh-my-zsh"

ZSH_THEME="robbyrussell"
# ZSH_THEME="agnoster"

plugins=(git)

source $ZSH/oh-my-zsh.sh

export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"  # This loads nvm
alias ll='ls -la'
"#;

    #[test]
    fn zshrc_theme_patch() {
        assert_eq!(zsh_theme(ZSHRC).as_deref(), Some("robbyrussell"));
        let out = set_zsh_theme(ZSHRC, "spaceship");
        assert_eq!(zsh_theme(&out).as_deref(), Some("spaceship"));
        assert!(out.contains("# ZSH_THEME=\"agnoster\""), "comments untouched");
        assert!(out.contains("export NVM_DIR=\"$HOME/.nvm\""));
        assert!(out.contains("alias ll='ls -la'"));
        assert_eq!(out.lines().count(), ZSHRC.lines().count());
        assert_eq!(set_zsh_theme(&out, "spaceship"), out, "idempotent");

        // No ZSH_THEME at all: inserted right before Oh-My-Zsh is loaded.
        let bare = "export ZSH=\"$HOME/.oh-my-zsh\"\nsource $ZSH/oh-my-zsh.sh\n";
        assert_eq!(set_zsh_theme(bare, "spaceship"), "export ZSH=\"$HOME/.oh-my-zsh\"\nZSH_THEME=\"spaceship\"\nsource $ZSH/oh-my-zsh.sh\n");
    }

    #[test]
    fn zshrc_plugins_patch() {
        let wanted = ["zsh-autosuggestions", "zsh-syntax-highlighting"];
        let out = ensure_plugins(ZSHRC, &wanted);
        assert!(out.contains("plugins=(git zsh-autosuggestions zsh-syntax-highlighting)\n"));
        let out = ensure_completions_fpath(&out);
        let lines: Vec<&str> = out.lines().collect();
        let fpath = lines.iter().position(|l| l.contains("zsh-completions/src")).unwrap();
        let source = lines.iter().position(|l| l.starts_with("source $ZSH/oh-my-zsh.sh")).unwrap();
        assert_eq!(fpath + 1, source, "fpath goes right before oh-my-zsh.sh");
        assert!(out.contains("[ -s \"$NVM_DIR/nvm.sh\" ] && \\. \"$NVM_DIR/nvm.sh\"  # This loads nvm"));
        assert!(out.contains("alias ll='ls -la'"));
        assert_eq!(out.lines().count(), ZSHRC.lines().count() + 1);
        assert_eq!(ensure_completions_fpath(&ensure_plugins(&out, &wanted)), out, "idempotent");

        let parsed = parse_linux_state(&format!(
            "@@user\nkyo\n@@shell\n/usr/bin/zsh\n@@flags\nzsh\nomz\nautosuggestions_dir\nsyntax_dir\ncompletions_dir\n@@zshrc\n{out}"
        ));
        assert!(parsed.zsh_is_default_shell && parsed.oh_my_zsh_installed);
        assert!(parsed.zsh_plugins.autosuggestions && parsed.zsh_plugins.syntax_highlighting && parsed.zsh_plugins.completions);
        assert!(!parsed.spaceship_installed);
    }

    #[test]
    fn zshrc_plugins_keep_user_plugins_and_syntax_last() {
        let z = "plugins=(git zsh-syntax-highlighting docker) # mine\nsource $ZSH/oh-my-zsh.sh\n";
        let out = ensure_plugins(z, &["zsh-autosuggestions", "zsh-syntax-highlighting"]);
        assert_eq!(out, "plugins=(git docker zsh-autosuggestions zsh-syntax-highlighting) # mine\nsource $ZSH/oh-my-zsh.sh\n");

        let multi = "plugins=(\n    git\n    docker\n)\nsource $ZSH/oh-my-zsh.sh\n";
        assert_eq!(zsh_plugins(multi).unwrap(), ["git", "docker"]);
        let out = ensure_plugins(multi, &["zsh-autosuggestions"]);
        assert_eq!(out, "plugins=(\n    git\n    docker\n    zsh-autosuggestions\n)\nsource $ZSH/oh-my-zsh.sh\n");

        let none = "source $ZSH/oh-my-zsh.sh\n";
        assert_eq!(ensure_plugins(none, &["zsh-autosuggestions"]), "plugins=(git zsh-autosuggestions)\nsource $ZSH/oh-my-zsh.sh\n");
    }

    #[test]
    fn zshrc_without_oh_my_zsh_gets_header() {
        let user = "export NVM_DIR=\"$HOME/.nvm\"\n";
        let out = ensure_omz_loaded(user);
        assert!(out.starts_with("# Oh-My-Zsh"));
        assert!(out.ends_with(user));
        assert_eq!(ensure_omz_loaded(&out), out);
        assert_eq!(ensure_omz_loaded(ZSHRC), ZSHRC);
    }

    #[test]
    fn wsl_conf_default_user() {
        assert_eq!(set_wsl_conf_default_user("", "kyo"), "[user]\ndefault=kyo\n");
        assert_eq!(
            set_wsl_conf_default_user("[boot]\nsystemd=true\n", "kyo"),
            "[boot]\nsystemd=true\n\n[user]\ndefault=kyo\n"
        );
        assert_eq!(
            set_wsl_conf_default_user("[user]\ndefault=old\n[boot]\nsystemd=true\n", "kyo"),
            "[user]\ndefault=kyo\n[boot]\nsystemd=true\n"
        );
        assert_eq!(set_wsl_conf_default_user("[User]\n# note\n", "kyo"), "[User]\ndefault=kyo\n# note\n");
    }

    #[test]
    fn validates_linux_users_and_passwords() {
        for ok in ["kyo", "_svc", "dev-01", "a_b"] {
            assert!(validate_linux_username(ok).is_ok(), "{ok}");
        }
        assert!(validate_linux_username(&"a".repeat(33)).is_err());
        for bad in ["", "root", "Kyo", "1abc", "-x", "a b", "a;rm", "joão"] {
            assert!(validate_linux_username(bad).is_err(), "{bad}");
        }
        assert!(validate_linux_password("s3nh@: com espaço").is_ok());
        assert!(validate_linux_password("").is_err());
        assert!(validate_linux_password("a\nroot:x").is_err());
    }

    #[test]
    fn parses_online_catalog() {
        let text = "Veja a seguir uma lista de distribuições válidas que podem ser instaladas.\n\
                    Instale usando 'wsl.exe --install <Distro>'.\n\n\
                    NAME                            FRIENDLY NAME\n\
                    Ubuntu                          Ubuntu\n\
                    Debian                          Debian GNU/Linux\n\
                    kali-linux                      Kali Linux Rolling\n\
                    Ubuntu-24.04                    Ubuntu 24.04 LTS\n";
        let all = parse_online_distros(text);
        assert_eq!(all.len(), 4);
        assert_eq!(all[1], OnlineDistro { name: "Debian".into(), friendly_name: "Debian GNU/Linux".into() });
        let offered: Vec<&str> = all.iter().filter(|d| is_installable(&d.name)).map(|d| d.name.as_str()).collect();
        assert_eq!(offered, ["Ubuntu", "Ubuntu-24.04"]);
        assert!(parse_online_distros("erro sem tabela").is_empty());
    }

    #[test]
    fn appx_launcher_names() {
        assert_eq!(appx_launcher("Ubuntu"), "ubuntu.exe");
        assert_eq!(appx_launcher("Ubuntu-24.04"), "ubuntu2404.exe");
    }

    fn distro(name: &str, is_default: bool) -> Distro {
        Distro { name: name.into(), state: DistroState::Stopped, state_label: "Stopped".into(), version: 2, is_default }
    }

    #[test]
    fn system_distros() {
        for system in ["docker-desktop", "docker-desktop-data", "rancher-desktop", "rancher-desktop-data", "podman-machine-default", "Docker-Desktop"] {
            assert!(is_system_distro(system), "{system}");
        }
        for dev in ["Ubuntu", "Ubuntu-24.04", "Debian", "kali-linux", "SymbiosisE2E", "docker-dev"] {
            assert!(!is_system_distro(dev), "{dev}");
        }
    }

    #[test]
    fn setup_opening_rule() {
        use crate::settings::Settings;
        let seen = Settings { onboarding_completed: true, setup_wizard_seen: true, ..Default::default() };
        let ubuntu = [distro("Ubuntu", false), distro("docker-desktop", true)];
        let only_docker = [distro("docker-desktop", true), distro("docker-desktop-data", false)];

        // Coming from 0.1.2: onboarding done, setup never seen → opens once.
        let from_012 = Settings { onboarding_completed: true, ..Default::default() };
        assert!(setup_required(&from_012, Some(&ubuntu)));
        // First run.
        assert!(setup_required(&Settings::default(), Some(&ubuntu)));
        // Set up, with Ubuntu (even if docker-desktop is the WSL default) → doesn't open.
        assert!(!setup_required(&seen, Some(&ubuntu)));
        // Docker Desktop only, or nothing at all → opens (there is no Linux to work in).
        assert!(setup_required(&seen, Some(&only_docker)));
        assert!(setup_required(&seen, Some(&[])));
        // wsl.exe didn't answer: an unknown list never forces the setup.
        assert!(!setup_required(&seen, None));
        // Windows restart pending after enabling WSL.
        assert!(setup_required(&Settings { wsl_reboot_pending: true, ..seen.clone() }, Some(&ubuntu)));
    }

    // Live, read-only: WSL_TEST_DISTRO=Ubuntu cargo test --lib live_setup -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_setup_inspect_linux() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let state = inspect_linux(&distro).await.expect("inspect");
        println!("{state:#?}");
        assert!(!state.user.is_empty());
        println!("{:#?}", inspect_windows_terminal(Some(&distro)));
    }

    /// End-to-end in a THROWAWAY distro: install (user + password + wsl.conf), the 4 Linux modules,
    /// checks, idempotent re-run, Windows Terminal on a COPY of settings.json, then unregister.
    /// Refuses to run if the distro already exists, and only removes the one it created.
    ///   WSL_E2E_DISTRO=Ubuntu-24.04 cargo test --lib live_setup_e2e -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_setup_e2e_throwaway_distro() {
        let distro = std::env::var("WSL_E2E_DISTRO").expect("set WSL_E2E_DISTRO (ex.: Ubuntu-24.04)");
        assert!(!is_registered(&distro).await, "{distro} já existe: o teste só roda numa distro que ele mesmo cria");
        // Optional: an official rootfs imported under a test name (lets the test run even when
        // the catalog's distros are already installed). install_distro then skips the download.
        let import_dir = std::env::temp_dir().join(format!("wsls-e2e-{distro}"));
        if let Ok(rootfs) = std::env::var("WSL_E2E_ROOTFS") {
            let _ = std::fs::remove_dir_all(&import_dir);
            std::fs::create_dir_all(&import_dir).unwrap();
            let mut import = wsl_command();
            import.args(["--import", &distro]).arg(&import_dir).arg(&rootfs).args(["--version", "2"]);
            run(import, DISTRO_INSTALL_TIMEOUT).await.expect("wsl --import");
            println!("== importada: {distro} de {rootfs}");
        }

        let name = distro.clone();
        let outcome = tokio::spawn(async move {
            let distro = name;
            let sink = |p: ProvisionProgress| {
                println!("[{}] {:<7} {}", p.step_id, p.status, p.message.or(p.log_line).unwrap_or_default());
            };
            let started = std::time::Instant::now();
            install_distro(&sink, &distro, "e2e", "symbiosis-e2e").await?;
            println!("== instalação: {:?}", started.elapsed());

            assert_eq!(default_user(&distro).await?, "e2e", "usuário padrão via /etc/wsl.conf");
            let conf = run(sh_in_distro(&distro, Some("root"), "cat /etc/wsl.conf", &[]), SHORT_TIMEOUT).await?;
            println!("== /etc/wsl.conf:\n{conf}");
            assert!(conf.contains("[user]") && conf.contains("default=e2e"));
            let groups = run(sh_in_distro(&distro, None, "id -nG", &[]), SHORT_TIMEOUT).await?;
            assert!(groups.split_whitespace().any(|g| g == "sudo"), "grupo sudo: {groups}");
            let hash = run(sh_in_distro(&distro, Some("root"), "getent shadow e2e | cut -d: -f2", &[]), SHORT_TIMEOUT).await?;
            assert!(hash.trim().starts_with('$'), "senha definida (hash no shadow)");

            let modules: Vec<String> =
                ["zsh_ohmyzsh", "spaceship_theme", "zsh_plugins", "nvm_node_lts"].iter().map(|s| s.to_string()).collect();
            let started = std::time::Instant::now();
            for r in run_modules(&sink, &distro, &modules).await? {
                assert_eq!(r.status, "done", "{}: {}", r.step_id, r.message);
            }
            println!("== módulos: {:?}", started.elapsed());

            let state = inspect_linux(&distro).await?;
            println!("{state:#?}");
            assert!(state.zsh_installed && state.zsh_is_default_shell && state.oh_my_zsh_installed);
            assert!(state.spaceship_installed);
            let p = &state.zsh_plugins;
            assert!(p.autosuggestions && p.syntax_highlighting && p.completions);
            assert!(state.nvm_installed && state.node_lts_installed);

            let zshrc = read_user_file(&distro, "\"$HOME/.zshrc\"").await?;
            println!("== ~/.zshrc (linhas ativas):");
            zshrc.lines().filter(|l| is_active(l)).for_each(|l| println!("   {l}"));

            // A real interactive zsh loads everything without errors.
            let check = run(
                sh_in_distro(&distro, None, "zsh -ic 'echo theme=$ZSH_THEME; command -v node; node --version' 2>&1", &[]),
                SHORT_TIMEOUT,
            )
            .await?;
            println!("== zsh -ic:\n{check}");
            assert!(check.contains("theme=spaceship") && check.contains("/.nvm/versions/node/"));

            // Idempotent: a second run changes nothing.
            let started = std::time::Instant::now();
            for r in run_modules(&sink, &distro, &modules).await? {
                assert_eq!(r.status, "done", "2ª execução {}: {}", r.step_id, r.message);
            }
            println!("== 2ª execução: {:?}", started.elapsed());
            assert_eq!(read_user_file(&distro, "\"$HOME/.zshrc\"").await?, zshrc, "~/.zshrc idêntico na 2ª execução");

            // Windows Terminal: on a copy, never the real file.
            if let Some(real) = wt_settings_path() {
                let before = std::fs::read(&real).unwrap();
                let dir = std::env::temp_dir().join(format!("wsls-e2e-wt-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let copy = dir.join("settings.json");
                // Without Dracula and pointing elsewhere, so the patch has work to do.
                let mut v = read_wt_settings(&real)?;
                v.as_object_mut().unwrap().remove("schemes");
                v["defaultProfile"] = Value::String("{00000000-0000-0000-0000-000000000000}".into());
                std::fs::write(&copy, to_wt_json(&v)?).unwrap();
                let rep = Reporter { sink: &sink, step: "windows_terminal_dracula" };
                println!("== WT (cópia): {}", apply_wt_file(&rep, &copy, "Ubuntu")?);
                let patched = read_wt_settings(&copy)?;
                assert!(wt_status(&patched, "Ubuntu").is_default && wt_status(&patched, "Ubuntu").uses_dracula);
                assert!(dir.join(format!("settings.json{BACKUP_SUFFIX}")).exists());
                assert_eq!(std::fs::read(&real).unwrap(), before, "o settings.json real não foi tocado");
                let _ = std::fs::remove_dir_all(&dir);
            }
            Ok::<(), String>(())
        })
        .await;

        // Always remove the distro this test created, even after a failed assertion.
        let mut unregister = wsl_command();
        unregister.args(["--unregister", &distro]);
        let removed = run(unregister, Duration::from_secs(180)).await;
        println!("== unregister {distro}: {removed:?}");
        let _ = std::fs::remove_dir_all(&import_dir);
        assert!(!is_registered(&distro).await, "{distro} removida");
        outcome.expect("o teste entrou em pânico").expect("falhou");
    }

    /// Runs the zshrc patches against a scratch HOME (not the user's).
    #[tokio::test]
    #[ignore]
    async fn live_setup_zshrc_roundtrip_in_scratch_home() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let home = "/tmp/wsls-setup-test";
        let prep = format!("rm -rf {home}; mkdir -p {home}");
        run(sh_in_distro(&distro, None, &prep, &[]), SHORT_TIMEOUT).await.unwrap();
        let path = format!("\"{home}/.zshrc\"");
        write_user_file(&distro, None, &path, ZSHRC).await.unwrap();
        // Second write: the backup must still hold the original.
        let patched = set_zsh_theme(&read_user_file(&distro, &path).await.unwrap(), "spaceship");
        write_user_file(&distro, None, &path, &patched).await.unwrap();
        write_user_file(&distro, None, &path, &ensure_plugins(&patched, &["zsh-autosuggestions"])).await.unwrap();
        let backup = read_user_file(&distro, &format!("\"{home}/.zshrc{BACKUP_SUFFIX}\"")).await.unwrap();
        assert_eq!(backup, ZSHRC);
        let now = read_user_file(&distro, &path).await.unwrap();
        assert_eq!(zsh_theme(&now).as_deref(), Some("spaceship"));
        assert!(zsh_plugins(&now).unwrap().contains(&"zsh-autosuggestions".to_string()));
        run(sh_in_distro(&distro, None, &format!("rm -rf {home}"), &[]), SHORT_TIMEOUT).await.unwrap();
    }
}
