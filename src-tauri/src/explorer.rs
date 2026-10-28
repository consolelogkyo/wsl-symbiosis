//! File explorer over a distro's filesystem, project detection and "open in" launchers.
//!
//! Every filesystem operation is one `sh -c` call (see `wsl::sh_in_distro`) running as the
//! distro's default user, so Linux permissions still apply. Paths and names reach the
//! scripts only as positional parameters ($1, $2...), never interpolated into the script.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::settings::SettingsStore;
use crate::wsl::{ensure_running, hidden_command, run, run_raw, sh_in_distro, validate_distro_name, wsl_command};

const FS_TIMEOUT: Duration = Duration::from_secs(20);
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_READ_LIMIT: usize = 512 * 1024;
const MAX_READ_LIMIT: usize = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;

/// Files/folders whose presence inside a subfolder marks it as a project (or as heavy,
/// cleanable build output: node_modules, target).
pub const PROJECT_MARKERS: [&str; 15] = [
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
    "Dockerfile",
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
];

// ---------------------------------------------------------------------------
// Types (mirrored in src/lib/explorer.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    /// Absolute Linux path.
    pub path: String,
    pub kind: EntryKind,
    /// For symlinks: whether the target is a directory (navigable).
    pub target_is_dir: bool,
    pub size: u64,
    /// Unix epoch seconds.
    pub modified: i64,
    /// `rwxr-xr-x` (file type letter stripped).
    pub permissions: String,
    pub project_markers: Vec<String>,
    /// Git repositories: the checked-out branch, or the short commit when detached.
    pub git_branch: Option<String>,
    pub git_detached: bool,
}

impl DirEntry {
    fn is_navigable(&self) -> bool {
        self.kind == EntryKind::Dir || (self.kind == EntryKind::Symlink && self.target_is_dir)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    /// `\\wsl.localhost\<distro>\...`
    pub windows_path: String,
    pub entries: Vec<DirEntry>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LineEnding {
    Lf,
    Crlf,
    Mixed,
    None,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFileContent {
    pub path: String,
    /// Empty when `binary`.
    pub content: String,
    pub size: u64,
    /// Only the first `max_bytes` were read; the UI must not save it back.
    pub truncated: bool,
    /// Contains NUL bytes or is not valid UTF-8.
    pub binary: bool,
    pub line_ending: LineEnding,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub available: bool,
    /// Launcher actually used (custom first, then auto-detected), or `None` when only the
    /// in-distro fallback may work.
    pub path: Option<String>,
    /// What auto-detection finds on its own (shown in the diagnostic screen).
    pub auto_path: Option<String>,
    /// Path from settings.json, even when it no longer exists (`source` tells).
    pub custom_path: Option<String>,
    pub source: ToolSource,
    /// Accepts a custom path in the settings (everything but Explorer).
    pub customizable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    Custom,
    /// A custom path is configured but the file is gone: auto-detection took over.
    CustomMissing,
    Auto,
    None,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Validates an absolute Linux path and normalizes it lexically (`//`, `.`, `..`,
/// trailing `/`). Never touches the filesystem.
pub fn normalize_linux_path(path: &str) -> Result<String, String> {
    if !path.starts_with('/') {
        return Err(format!("O caminho precisa ser absoluto: {path:?}"));
    }
    if path.chars().any(char::is_control) {
        return Err("O caminho contém caracteres de controle".into());
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

pub fn parent_of(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let idx = path.rfind('/')?;
    Some(if idx == 0 { "/".to_string() } else { path[..idx].to_string() })
}

fn join(dir: &str, name: &str) -> String {
    if dir == "/" { format!("/{name}") } else { format!("{dir}/{name}") }
}

/// Validates the last component of a path the user is creating or renaming to.
fn validate_new_path(path: &str) -> Result<(), String> {
    validate_entry_name(path.rsplit('/').next().unwrap_or_default())
}

/// A single path component chosen by the user (new file/folder, rename).
pub fn validate_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("Nome inválido".into());
    }
    if name.contains('/') || name.chars().any(char::is_control) {
        return Err("O nome não pode conter '/' nem caracteres de controle".into());
    }
    if name.len() > 255 {
        return Err("Nome longo demais (máx. 255 bytes)".into());
    }
    Ok(())
}

/// Paths that must never be deleted (or renamed away) from the UI: the root, every
/// top-level directory (/home, /root, /etc, /usr, /var, /bin, ...), each user's home
/// and each Windows drive mount (/mnt/c). Expects a normalized path.
pub fn is_protected(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(parts.as_slice(), [] | [_] | ["home" | "mnt", _])
}

/// `/home/kyo` → `\\wsl.localhost\Ubuntu\home\kyo`.
pub fn to_windows_path(distro: &str, linux_path: &str) -> String {
    format!(r"\\wsl.localhost\{distro}{}", linux_path.replace('/', "\\"))
}

/// Windows Terminal treats `;` as a command separator; escape it inside arguments.
fn escape_wt_arg(arg: &str) -> String {
    arg.replace(';', "\\;")
}

pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

fn detect_line_ending(content: &str) -> LineEnding {
    let crlf = content.matches("\r\n").count();
    let lf = content.matches('\n').count();
    match (crlf, lf) {
        (0, 0) => LineEnding::None,
        (0, _) => LineEnding::Lf,
        (c, l) if c == l => LineEnding::Crlf,
        _ => LineEnding::Mixed,
    }
}

// ---------------------------------------------------------------------------
// Directory listing
// ---------------------------------------------------------------------------

/// `$1` = directory, `$2` = 1 to include dotfiles. Sections:
/// - `@@entries`: `stat -c '%F/%s/%Y/%A/%n'` per entry ('/' cannot appear in a name);
/// - `@@links`: `d/<name>` or `f/<name>` for symlinks (target kind);
/// - `@@markers`: `<name>/<marker>/<marker>...` for subfolders with project markers;
/// - `@@heads`: `<name>/<first line of .git/HEAD>` for git repositories (`.git` may be a
///   `gitdir:` file in worktrees and submodules).
///
/// Only builtins run per entry, so big folders (node_modules) stay fast. The branch comes
/// from reading HEAD, not from running git (which may not even be installed).
fn list_script() -> String {
    format!(
        r#"cd -- "$1" || exit 3
if [ "$2" = 1 ]; then set -- * .[!.]* ..?*; else set -- *; fi
echo @@entries
stat -c '%F/%s/%Y/%A/%n' -- "$@" 2>/dev/null
echo @@links
for f in "$@"; do if [ -L "$f" ]; then if [ -d "$f" ]; then echo "d/$f"; else echo "f/$f"; fi; fi; done
echo @@markers
for d in "$@"; do
  [ -d "$d" ] || continue
  m=
  for x in {markers}; do [ -e "$d/$x" ] && m="$m/$x"; done
  [ -n "$m" ] && echo "$d$m"
done
echo @@heads
for d in "$@"; do
  [ -e "$d/.git" ] || continue
  g="$d/.git"
  if [ -f "$g" ]; then
    {{ read -r _ g < "$g"; }} 2>/dev/null
    case $g in /*) ;; *) g="$d/$g" ;; esac
  fi
  {{ read -r h < "$g/HEAD"; }} 2>/dev/null && echo "$d/$h"
done
exit 0"#,
        markers = PROJECT_MARKERS.join(" ")
    )
}

/// Parses the output of [`list_script`] for directory `dir`; sorts folders first,
/// then files, case-insensitively.
pub fn parse_listing(dir: &str, text: &str, show_hidden: bool) -> Vec<DirEntry> {
    let mut section = "";
    let mut entries: Vec<DirEntry> = Vec::new();
    let mut links: Vec<(String, bool)> = Vec::new();
    let mut markers: Vec<(String, Vec<String>)> = Vec::new();
    let mut heads: Vec<(String, String)> = Vec::new();

    for line in text.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            section = name.trim();
            continue;
        }
        match section {
            "entries" => {
                let mut f = line.splitn(5, '/');
                let (Some(kind), Some(size), Some(mtime), Some(mode), Some(name)) =
                    (f.next(), f.next(), f.next(), f.next(), f.next())
                else {
                    continue;
                };
                if name.is_empty() || name == "." || name == ".." || (!show_hidden && name.starts_with('.')) {
                    continue;
                }
                entries.push(DirEntry {
                    name: name.to_string(),
                    path: join(dir, name),
                    kind: match kind {
                        "directory" => EntryKind::Dir,
                        "symbolic link" => EntryKind::Symlink,
                        _ => EntryKind::File,
                    },
                    target_is_dir: false,
                    size: size.parse().unwrap_or(0),
                    modified: mtime.parse().unwrap_or(0),
                    permissions: mode.chars().skip(1).collect(),
                    project_markers: Vec::new(),
                    git_branch: None,
                    git_detached: false,
                });
            }
            "links" => {
                if let Some((t, name)) = line.split_once('/') {
                    links.push((name.to_string(), t == "d"));
                }
            }
            "markers" => {
                let mut parts = line.split('/');
                if let Some(name) = parts.next() {
                    markers.push((name.to_string(), parts.map(str::to_string).collect()));
                }
            }
            "heads" => {
                if let Some((name, head)) = line.split_once('/') {
                    heads.push((name.to_string(), head.trim().to_string()));
                }
            }
            _ => {}
        }
    }

    for entry in &mut entries {
        if let Some((_, is_dir)) = links.iter().find(|(n, _)| *n == entry.name) {
            entry.target_is_dir = *is_dir;
        }
        if let Some((_, m)) = markers.iter().find(|(n, _)| *n == entry.name) {
            entry.project_markers = m.clone();
        }
        if let Some((_, head)) = heads.iter().find(|(n, _)| *n == entry.name) {
            (entry.git_branch, entry.git_detached) = parse_head(head);
        }
    }
    entries.sort_by(|a, b| {
        b.is_navigable()
            .cmp(&a.is_navigable())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

/// First line of `.git/HEAD`: `ref: refs/heads/<branch>` or a detached commit hash.
fn parse_head(head: &str) -> (Option<String>, bool) {
    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        return (Some(branch.to_string()), false);
    }
    if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) {
        return (Some(head[..7].to_string()), true);
    }
    (None, false)
}

// ---------------------------------------------------------------------------
// Text files
// ---------------------------------------------------------------------------

const READ_SCRIPT: &str = r#"[ -f "$1" ] || { echo "nao e um arquivo regular: $1" >&2; exit 1; }
wc -c < "$1"
head -c "$2" -- "$1""#;

/// Output of [`READ_SCRIPT`]: first line is the full size, the rest is up to N raw bytes.
pub fn parse_read_output(path: &str, raw: &[u8]) -> Result<TextFileContent, String> {
    let newline = raw.iter().position(|&b| b == b'\n').ok_or("Resposta inválida ao ler o arquivo")?;
    let size: u64 = String::from_utf8_lossy(&raw[..newline])
        .trim()
        .parse()
        .map_err(|_| "Tamanho do arquivo inválido".to_string())?;
    let bytes = &raw[newline + 1..];
    let truncated = (bytes.len() as u64) < size;

    let text = (!bytes.contains(&0)).then(|| std::str::from_utf8(bytes).ok()).flatten();
    Ok(match text {
        Some(text) => TextFileContent {
            path: path.to_string(),
            line_ending: detect_line_ending(text),
            content: text.to_string(),
            size,
            truncated,
            binary: false,
        },
        // A truncated read may cut a multi-byte char at the end; that alone is not binary.
        None if truncated && std::str::from_utf8(&bytes[..bytes.len().saturating_sub(3)]).is_ok() => {
            let text = String::from_utf8_lossy(bytes).trim_end_matches('\u{FFFD}').to_string();
            TextFileContent {
                path: path.to_string(),
                line_ending: detect_line_ending(&text),
                content: text,
                size,
                truncated,
                binary: false,
            }
        }
        None => TextFileContent {
            path: path.to_string(),
            content: String::new(),
            size,
            truncated,
            binary: true,
            line_ending: LineEnding::None,
        },
    })
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

async fn sh(distro: &str, script: &str, args: &[&str]) -> Result<String, String> {
    run(sh_in_distro(distro, None, script, args), FS_TIMEOUT).await
}

pub(crate) async fn distro_home(distro: &str) -> Result<String, String> {
    ensure_running(distro).await?;
    let script = r#"h=$HOME; if [ -z "$h" ] || [ ! -d "$h" ]; then h=/; fi; printf '%s' "$h""#;
    let home = sh(distro, script, &[]).await?;
    normalize_linux_path(home.trim()).or_else(|_| Ok("/".into()))
}

pub(crate) async fn list(distro: &str, path: &str, show_hidden: bool) -> Result<DirectoryListing, String> {
    let path = normalize_linux_path(path)?;
    ensure_running(distro).await?;
    let flag = if show_hidden { "1" } else { "0" };
    let text = sh(distro, &list_script(), &[&path, flag]).await?;
    Ok(DirectoryListing {
        entries: parse_listing(&path, &text, show_hidden),
        parent: parent_of(&path),
        windows_path: to_windows_path(distro, &path),
        path,
    })
}

pub(crate) async fn mkdir(distro: &str, path: &str) -> Result<(), String> {
    let path = normalize_linux_path(path)?;
    ensure_running(distro).await?;
    sh(distro, r#"mkdir -p -- "$1""#, &[&path]).await.map(|_| ())
}

pub(crate) async fn new_file(distro: &str, path: &str) -> Result<(), String> {
    let path = normalize_linux_path(path)?;
    validate_new_path(&path)?;
    ensure_running(distro).await?;
    let script = r#"if [ -e "$1" ]; then echo "ja existe: $1" >&2; exit 1; fi; touch -- "$1""#;
    sh(distro, script, &[&path]).await.map(|_| ())
}

pub(crate) async fn rename(distro: &str, old_path: &str, new_path: &str) -> Result<(), String> {
    let (old_path, new_path) = (normalize_linux_path(old_path)?, normalize_linux_path(new_path)?);
    if is_protected(&old_path) {
        return Err(format!("{old_path} é protegido e não pode ser renomeado"));
    }
    validate_new_path(&new_path)?;
    ensure_running(distro).await?;
    let script = r#"if [ -e "$2" ] || [ -L "$2" ]; then echo "destino ja existe: $2" >&2; exit 1; fi; mv -- "$1" "$2""#;
    sh(distro, script, &[&old_path, &new_path]).await.map(|_| ())
}

pub(crate) async fn delete(distro: &str, path: &str) -> Result<(), String> {
    let path = normalize_linux_path(path)?;
    if is_protected(&path) {
        return Err(format!("{path} é protegido e não pode ser excluído"));
    }
    ensure_running(distro).await?;
    sh(distro, r#"rm -rf -- "$1""#, &[&path]).await.map(|_| ())
}

pub(crate) async fn read_text(distro: &str, path: &str, max_bytes: Option<usize>) -> Result<TextFileContent, String> {
    let path = normalize_linux_path(path)?;
    ensure_running(distro).await?;
    let limit = max_bytes.unwrap_or(DEFAULT_READ_LIMIT).clamp(1, MAX_READ_LIMIT).to_string();
    let cmd = sh_in_distro(distro, None, READ_SCRIPT, &[&path, &limit]);
    let raw = run_raw(cmd, FS_TIMEOUT, None).await?;
    parse_read_output(&path, &raw)
}

pub(crate) async fn write_text(distro: &str, path: &str, content: &str) -> Result<(), String> {
    let path = normalize_linux_path(path)?;
    let content = normalize_line_endings(content);
    if content.len() > MAX_WRITE_BYTES {
        return Err("Conteúdo grande demais para o editor rápido".into());
    }
    ensure_running(distro).await?;
    // `cat >` keeps the file's inode, owner and permissions; bytes go through stdin
    // untouched (UTF-8, LF).
    let script = r#"if [ -d "$1" ]; then echo "e um diretorio: $1" >&2; exit 1; fi; cat > "$1""#;
    let cmd = sh_in_distro(distro, None, script, &[&path]);
    run_raw(cmd, FS_TIMEOUT, Some(content.as_bytes())).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// Open in external tools
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Antigravity,
    VsCode,
    Cursor,
    Explorer,
    WindowsTerminal,
}

impl Tool {
    const ALL: [Tool; 5] = [Tool::Antigravity, Tool::VsCode, Tool::Cursor, Tool::Explorer, Tool::WindowsTerminal];

    pub fn parse(id: &str) -> Result<Tool, String> {
        Tool::ALL
            .into_iter()
            .find(|t| t.id() == id)
            .ok_or_else(|| format!("Ferramenta desconhecida: {id}"))
    }

    pub fn id(self) -> &'static str {
        match self {
            Tool::Antigravity => "antigravity",
            Tool::VsCode => "vscode",
            Tool::Cursor => "cursor",
            Tool::Explorer => "explorer",
            Tool::WindowsTerminal => "windows_terminal",
        }
    }

    /// explorer.exe always exists; the others may live anywhere (e.g. `D:\`).
    pub fn customizable(self) -> bool {
        self != Tool::Explorer
    }

    /// Editors that can be the user's default IDE (the labeled "Abrir em" button).
    pub fn is_ide(self) -> bool {
        matches!(self, Tool::Antigravity | Tool::VsCode | Tool::Cursor)
    }

    /// The custom path from the settings when it still exists, else auto-detection.
    fn launcher(self, custom: &HashMap<String, String>) -> Option<PathBuf> {
        self.custom_launcher(custom)
            .filter(|p| p.is_file())
            .or_else(|| self.auto_launcher())
    }

    fn custom_launcher(self, custom: &HashMap<String, String>) -> Option<PathBuf> {
        if !self.customizable() {
            return None;
        }
        custom.get(self.id()).map(PathBuf::from)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Antigravity => "Antigravity",
            Tool::VsCode => "VS Code",
            Tool::Cursor => "Cursor",
            Tool::Explorer => "Explorer",
            Tool::WindowsTerminal => "Windows Terminal",
        }
    }

    /// Known install locations (relative to %LOCALAPPDATA% / %ProgramFiles%), then PATH.
    fn auto_launcher(self) -> Option<PathBuf> {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        let at = |base: &Option<PathBuf>, rel: &str| base.as_ref().map(|b| b.join(rel));
        let (candidates, path_names): (Vec<Option<PathBuf>>, &[&str]) = match self {
            Tool::Antigravity => (
                vec![
                    at(&local, r"Programs\Antigravity IDE\bin\antigravity-ide.cmd"),
                    at(&local, r"Programs\Antigravity\bin\antigravity.cmd"),
                ],
                &["antigravity-ide.cmd", "antigravity.cmd"],
            ),
            // `code` on PATH may be Cursor's shim (cursor\resources\app\codeBin\code.cmd),
            // so VS Code's own install dirs come first and PATH hits inside cursor are skipped.
            Tool::VsCode => (
                vec![
                    at(&local, r"Programs\Microsoft VS Code\bin\code.cmd"),
                    at(&program_files, r"Microsoft VS Code\bin\code.cmd"),
                ],
                &["code.cmd"],
            ),
            Tool::Cursor => (vec![at(&local, r"Programs\cursor\resources\app\bin\cursor.cmd")], &["cursor.cmd"]),
            Tool::Explorer => (vec![], &["explorer.exe"]),
            Tool::WindowsTerminal => (vec![at(&local, r"Microsoft\WindowsApps\wt.exe")], &["wt.exe"]),
        };
        candidates
            .into_iter()
            .flatten()
            .find(|p| p.is_file())
            .or_else(|| {
                path_names
                    .iter()
                    .filter_map(|name| which(name))
                    .find(|p| self != Tool::VsCode || !p.to_string_lossy().to_lowercase().contains("cursor"))
            })
    }

    /// CLI names to try inside the distro when no Windows launcher is found.
    fn distro_fallback(self) -> &'static [&'static str] {
        match self {
            Tool::Antigravity => &["antigravity-ide", "antigravity"],
            Tool::VsCode => &["code"],
            Tool::Cursor => &["cursor"],
            Tool::Explorer | Tool::WindowsTerminal => &[],
        }
    }
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

pub(crate) fn detect(custom: &HashMap<String, String>) -> Vec<ToolInfo> {
    Tool::ALL
        .into_iter()
        .map(|tool| {
            let auto = tool.auto_launcher();
            let custom_path = tool.custom_launcher(custom);
            let custom_ok = custom_path.as_ref().is_some_and(|p| p.is_file());
            let (path, source) = match (&custom_path, custom_ok, &auto) {
                (Some(p), true, _) => (Some(p.clone()), ToolSource::Custom),
                (Some(_), false, a) => (a.clone(), ToolSource::CustomMissing),
                (None, _, Some(a)) => (Some(a.clone()), ToolSource::Auto),
                (None, _, None) => (None, ToolSource::None),
            };
            let show = |p: PathBuf| p.display().to_string();
            ToolInfo {
                id: tool.id(),
                label: tool.label(),
                available: path.is_some() || tool == Tool::Explorer,
                path: path.map(show),
                auto_path: auto.map(show),
                custom_path: custom_path.map(show),
                source,
                customizable: tool.customizable(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Official app icons, extracted from the user's own installs (works offline)
// ---------------------------------------------------------------------------

const ICON_TIMEOUT: Duration = Duration::from_secs(20);

/// Extracted icons (PNG data URLs) by executable path. Extraction spawns PowerShell, so
/// each path is extracted once per app run; `None` = the file has no usable icon.
#[derive(Default)]
pub struct ToolIconCache(Mutex<HashMap<PathBuf, Option<String>>>);

/// The GUI executable whose icon represents a launcher: the launcher itself when it is a
/// real .exe, else the largest .exe in the folders above a .cmd shim (Electron IDEs keep
/// `bin\code.cmd` a few levels below `Code.exe`; `code-tunnel.exe` is much smaller).
fn icon_source(launcher: &Path) -> Option<PathBuf> {
    let ext = launcher.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "exe" {
        // App execution aliases (WindowsApps\wt.exe) are 0-byte reparse points: no icon.
        let len = std::fs::metadata(launcher).ok()?.len();
        return (len > 0).then(|| launcher.to_path_buf());
    }
    launcher
        .ancestors()
        .skip(1)
        .take(4)
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_ascii_lowercase();
            if !name.ends_with(".exe") || name.starts_with("unins") {
                return None;
            }
            Some((entry.metadata().ok()?.len(), path))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, path)| path)
}

/// One PowerShell run for all paths (`System.Drawing.Icon.ExtractAssociatedIcon`, 32px PNG).
/// Paths go in an env var, one per line; output lines are `<index>:<base64 or empty>`.
async fn extract_icons(paths: &[PathBuf]) -> Result<Vec<Option<String>>, String> {
    // No double quotes in the script: it travels as a single -Command argument.
    const SCRIPT: &str = "Add-Type -AssemblyName System.Drawing; $i = 0; \
        foreach ($p in ($env:WSLSYM_ICON_PATHS -split [char]10)) { $b = ''; \
        try { $icon = [System.Drawing.Icon]::ExtractAssociatedIcon($p); $ms = New-Object System.IO.MemoryStream; \
        $icon.ToBitmap().Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $b = [Convert]::ToBase64String($ms.ToArray()) } catch {}; \
        Write-Output ($i.ToString() + ':' + $b); $i++ }";
    let joined = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
    let mut cmd = hidden_command("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("WSLSYM_ICON_PATHS", joined);
    let out = run(cmd, ICON_TIMEOUT).await?;
    Ok(parse_icon_output(&out, paths.len()))
}

fn parse_icon_output(out: &str, count: usize) -> Vec<Option<String>> {
    let mut icons = vec![None; count];
    for line in out.lines() {
        let Some((index, b64)) = line.trim().split_once(':') else { continue };
        let Ok(index) = index.parse::<usize>() else { continue };
        if index < count && !b64.is_empty() && b64.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)) {
            icons[index] = Some(format!("data:image/png;base64,{b64}"));
        }
    }
    icons
}

/// Waits briefly for a launcher (IDE CLIs hand off to the GUI and exit). A launcher still
/// running after the timeout is assumed to have started fine and is left alone.
async fn launch(mut cmd: tokio::process::Command, check_status: bool) -> Result<(), String> {
    let child = cmd.spawn().map_err(|e| format!("Falha ao iniciar: {e}"))?;
    match tokio::time::timeout(LAUNCH_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) if check_status && !out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if stderr.is_empty() { format!("Saiu com {}", out.status) } else { stderr })
        }
        Ok(Err(e)) => Err(format!("Falha ao aguardar: {e}")),
        _ => Ok(()),
    }
}

pub(crate) async fn open(
    distro: &str,
    linux_path: &str,
    tool: &str,
    custom: &HashMap<String, String>,
) -> Result<(), String> {
    validate_distro_name(distro)?;
    let tool = Tool::parse(tool)?;
    let path = normalize_linux_path(linux_path)?;
    ensure_running(distro).await?;

    let win_path = to_windows_path(distro, &path);
    let is_dir = Path::new(&win_path).is_dir();

    match tool {
        Tool::Explorer => {
            if win_path.contains('"') {
                return Err("O Explorer não abre caminhos com aspas".into());
            }
            let mut cmd = hidden_command("explorer.exe");
            #[cfg(windows)]
            {
                // explorer.exe has its own argument syntax; quote by hand.
                let arg = if is_dir { format!("\"{win_path}\"") } else { format!("/select,\"{win_path}\"") };
                cmd.raw_arg(arg);
            }
            // explorer.exe returns 1 even on success.
            launch(cmd, false).await
        }
        Tool::WindowsTerminal => {
            let wt = tool.launcher(custom).ok_or("Windows Terminal (wt.exe) não encontrado")?;
            let dir = if is_dir { path.clone() } else { parent_of(&path).unwrap_or_else(|| "/".into()) };
            let mut cmd = hidden_command(wt);
            cmd.args(["-w", "0", "nt", "wsl.exe", "-d", distro, "--cd"]).arg(escape_wt_arg(&dir));
            launch(cmd, true).await
        }
        Tool::Antigravity | Tool::VsCode | Tool::Cursor => {
            if let Some(launcher) = tool.launcher(custom) {
                let mut cmd = hidden_command(launcher);
                cmd.args(["--remote", &format!("wsl+{distro}"), &path]);
                return launch(cmd, true).await;
            }
            // No Windows install found: try the CLI from inside the distro (interactive
            // shell so ~/.bashrc PATH tweaks apply). Exit 127 = not found, try the next.
            for name in tool.distro_fallback() {
                let mut cmd = wsl_command();
                cmd.args(["-d", distro, "--", "bash", "-ic"])
                    .arg(r#"command -v "$2" >/dev/null 2>&1 || exit 127; "$2" "$1""#)
                    .args(["_", &path, name]);
                match run(cmd, LAUNCH_TIMEOUT).await {
                    Ok(_) => return Ok(()),
                    Err(_) => continue,
                }
            }
            Err(format!("{} não encontrado no Windows nem na distro", tool.label()))
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_distro_home(distro: String) -> Result<String, String> {
    distro_home(&distro).await
}

#[tauri::command]
pub async fn list_directory(distro: String, path: String, show_hidden: bool) -> Result<DirectoryListing, String> {
    list(&distro, &path, show_hidden).await
}

#[tauri::command]
pub async fn create_directory(distro: String, path: String) -> Result<(), String> {
    mkdir(&distro, &path).await
}

#[tauri::command]
pub async fn create_file(distro: String, path: String) -> Result<(), String> {
    new_file(&distro, &path).await
}

#[tauri::command]
pub async fn rename_entry(distro: String, old_path: String, new_path: String) -> Result<(), String> {
    rename(&distro, &old_path, &new_path).await
}

#[tauri::command]
pub async fn delete_entry(distro: String, path: String) -> Result<(), String> {
    delete(&distro, &path).await
}

#[tauri::command]
pub async fn read_text_file(distro: String, path: String, max_bytes: Option<usize>) -> Result<TextFileContent, String> {
    read_text(&distro, &path, max_bytes).await
}

#[tauri::command]
pub async fn write_text_file(distro: String, path: String, content: String) -> Result<(), String> {
    write_text(&distro, &path, &content).await
}

#[tauri::command]
pub async fn open_in_tool(
    settings: State<'_, SettingsStore>,
    distro: String,
    linux_path: String,
    tool: String,
) -> Result<(), String> {
    let custom = settings.custom_tool_paths();
    open(&distro, &linux_path, &tool, &custom).await
}

#[tauri::command]
pub async fn detect_tools(settings: State<'_, SettingsStore>) -> Result<Vec<ToolInfo>, String> {
    let custom = settings.custom_tool_paths();
    tauri::async_runtime::spawn_blocking(move || detect(&custom))
        .await
        .map_err(|e| e.to_string())
}

/// Tool id → official icon (PNG data URL) taken from the installed executable. Tools whose
/// icon can't be read are simply absent (the UI keeps its generic icon).
#[tauri::command]
pub async fn get_tool_icons(
    settings: State<'_, SettingsStore>,
    cache: State<'_, ToolIconCache>,
) -> Result<HashMap<String, String>, String> {
    let custom = settings.custom_tool_paths();
    let sources: Vec<(&'static str, PathBuf)> = tauri::async_runtime::spawn_blocking(move || {
        // Explorer keeps the plain white folder glyph: less visual noise in every row.
        Tool::ALL
            .into_iter()
            .filter(|&tool| tool != Tool::Explorer)
            .filter_map(|tool| Some((tool.id(), icon_source(&tool.launcher(&custom)?)?)))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?;

    let mut missing: Vec<PathBuf> = {
        let known = cache.0.lock().unwrap();
        sources.iter().map(|(_, p)| p.clone()).filter(|p| !known.contains_key(p)).collect()
    };
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        // A failed run (PowerShell blocked, timeout) isn't cached: the next call retries.
        if let Ok(icons) = extract_icons(&missing).await {
            let mut known = cache.0.lock().unwrap();
            known.extend(missing.into_iter().zip(icons));
        }
    }

    let known = cache.0.lock().unwrap();
    Ok(sources
        .into_iter()
        .filter_map(|(id, path)| Some((id.to_string(), known.get(&path)?.clone()?)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_linux_path("/").unwrap(), "/");
        assert_eq!(normalize_linux_path("//home//kyo/").unwrap(), "/home/kyo");
        assert_eq!(normalize_linux_path("/home/kyo/./a/../b").unwrap(), "/home/kyo/b");
        assert_eq!(normalize_linux_path("/../../etc").unwrap(), "/etc");
        assert!(normalize_linux_path("home/kyo").is_err());
        assert!(normalize_linux_path("/tmp/a\nb").is_err());
    }

    #[test]
    fn protects_critical_paths() {
        for p in ["/", "/home", "/root", "/etc", "/usr", "/var", "/bin", "/home/kyo", "/mnt/c"] {
            assert!(is_protected(&normalize_linux_path(p).unwrap()), "{p} must be protected");
        }
        // Traversal tricks normalize onto a protected path.
        assert!(is_protected(&normalize_linux_path("/home/kyo/proj/../../..").unwrap()));
        assert!(is_protected(&normalize_linux_path("/tmp/../etc/").unwrap()));
        for p in ["/home/kyo/proj/node_modules", "/tmp/x", "/mnt/c/Users", "/usr/local/x"] {
            assert!(!is_protected(p), "{p} should be deletable");
        }
    }

    #[test]
    fn validates_names() {
        assert!(validate_entry_name("novo arquivo (1).txt").is_ok());
        assert!(validate_entry_name("..").is_err());
        assert!(validate_entry_name("a/b").is_err());
        assert!(validate_entry_name("").is_err());
    }

    #[test]
    fn converts_paths() {
        assert_eq!(to_windows_path("Ubuntu", "/home/kyo"), r"\\wsl.localhost\Ubuntu\home\kyo");
        assert_eq!(to_windows_path("Ubuntu", "/"), r"\\wsl.localhost\Ubuntu\");
        assert_eq!(parent_of("/home/kyo"), Some("/home".into()));
        assert_eq!(parent_of("/home"), Some("/".into()));
        assert_eq!(parent_of("/"), None);
        assert_eq!(escape_wt_arg("/tmp/a;b"), "/tmp/a\\;b");
        assert_eq!(Tool::parse("windows_terminal"), Ok(Tool::WindowsTerminal));
        assert!(Tool::parse("rm -rf").is_err());
    }

    #[test]
    fn writes_with_lf() {
        assert_eq!(normalize_line_endings("a\r\nb\r\n\r\nc"), "a\nb\n\nc");
        assert_eq!(normalize_line_endings("já\nLF\n"), "já\nLF\n");
        assert_eq!(detect_line_ending("a\r\nb\r\n"), LineEnding::Crlf);
        assert_eq!(detect_line_ending("a\nb\r\n"), LineEnding::Mixed);
        assert_eq!(detect_line_ending("a\nb"), LineEnding::Lf);
        assert_eq!(detect_line_ending("a"), LineEnding::None);
    }

    const LISTING: &str = "@@entries
regular file/220/1700000000/-rw-r--r--/.bashrc
directory/4096/1700000001/drwxr-xr-x/zeta
directory/4096/1700000002/drwxr-xr-x/Alpha proj
regular file/10/1700000003/-rw-r--r--/b.txt
symbolic link/9/1700000004/lrwxrwxrwx/link-dir
symbolic link/9/1700000005/lrwxrwxrwx/link-file
regular empty file/0/1700000006/-rw-------/a.md
@@links
d/link-dir
f/link-file
@@markers
Alpha proj/package.json/.git/node_modules
zeta/Cargo.toml/target
@@heads
Alpha proj/ref: refs/heads/feature/FLEET-2098
zeta/3f9c2a1b8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a
";

    #[test]
    fn parses_listing() {
        let entries = parse_listing("/home/kyo", LISTING, true);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha proj", "link-dir", "zeta", ".bashrc", "a.md", "b.txt", "link-file"]);

        let alpha = &entries[0];
        assert_eq!(alpha.path, "/home/kyo/Alpha proj");
        assert_eq!(alpha.kind, EntryKind::Dir);
        assert_eq!(alpha.permissions, "rwxr-xr-x");
        assert_eq!(alpha.project_markers, ["package.json", ".git", "node_modules"]);
        assert_eq!(entries[1].kind, EntryKind::Symlink);
        assert!(entries[1].target_is_dir);
        assert_eq!(entries[2].project_markers, ["Cargo.toml", "target"]);
        assert_eq!(alpha.git_branch.as_deref(), Some("feature/FLEET-2098"), "branch names keep their slashes");
        assert!(!alpha.git_detached);
        assert_eq!(entries[2].git_branch.as_deref(), Some("3f9c2a1"));
        assert!(entries[2].git_detached);
        assert_eq!(entries[3].git_branch, None);
        assert_eq!(entries[4].size, 0);
        assert_eq!(entries[4].modified, 1_700_000_006);
        assert!(!entries[6].target_is_dir);
    }

    #[test]
    fn listing_hides_dotfiles_and_handles_root() {
        let entries = parse_listing("/", LISTING, false);
        assert!(entries.iter().all(|e| !e.name.starts_with('.')));
        assert_eq!(entries.iter().find(|e| e.name == "zeta").unwrap().path, "/zeta");
        assert!(parse_listing("/", "@@entries\ngarbage\n", true).is_empty());
    }

    #[test]
    fn parses_read_output() {
        let text = parse_read_output("/a", b"12\nol\xc3\xa1\r\nmundo\n").unwrap();
        assert!(!text.binary && !text.truncated);
        assert_eq!(text.content, "olá\r\nmundo\n");
        assert_eq!(text.line_ending, LineEnding::Mixed);

        let bin = parse_read_output("/b", b"4\n\x7fELF").unwrap();
        assert!(!bin.binary, "no NUL and valid UTF-8 is text");
        let bin = parse_read_output("/b", b"5\nA\x00B\x01C").unwrap();
        assert!(bin.binary && bin.content.is_empty());

        // Truncated in the middle of a multi-byte char: still text.
        let cut = parse_read_output("/c", b"100\nabc\xc3").unwrap();
        assert!(cut.truncated && !cut.binary);
        assert_eq!(cut.content, "abc");
        assert!(parse_read_output("/d", b"").is_err());
    }

    #[test]
    fn custom_tool_paths_take_precedence() {
        let dir = std::env::temp_dir().join(format!("wsl-symbiosis-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("cursor.cmd");
        std::fs::write(&fake, "@echo off").unwrap();

        let mut custom = HashMap::new();
        custom.insert("cursor".to_string(), fake.display().to_string());
        custom.insert("vscode".to_string(), dir.join("gone.cmd").display().to_string());
        custom.insert("explorer".to_string(), fake.display().to_string());
        let tools = detect(&custom);
        let get = |id: &str| tools.iter().find(|t| t.id == id).unwrap();

        let cursor = get("cursor");
        assert_eq!(cursor.source, ToolSource::Custom);
        assert!(cursor.available);
        assert_eq!(cursor.path.as_deref(), Some(fake.to_str().unwrap()));
        assert_eq!(Tool::Cursor.launcher(&custom), Some(fake.clone()));

        // Missing custom file: falls back to whatever auto-detection finds.
        let vscode = get("vscode");
        assert_eq!(vscode.source, ToolSource::CustomMissing);
        assert_eq!(vscode.path, vscode.auto_path);

        // Explorer ignores custom paths.
        let explorer = get("explorer");
        assert!(!explorer.customizable && explorer.custom_path.is_none() && explorer.available);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_gui_exe_behind_cmd_shims() {
        let root = std::env::temp_dir().join(format!("wsl-symbiosis-icons-{}", std::process::id()));
        let bin = root.join("Fake IDE").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let shim = bin.join("fake.cmd");
        std::fs::write(&shim, "@echo off").unwrap();
        std::fs::write(bin.join("fake-tunnel.exe"), vec![0u8; 10]).unwrap();
        std::fs::write(root.join("Fake IDE").join("Fake IDE.exe"), vec![0u8; 100]).unwrap();
        std::fs::write(root.join("Fake IDE").join("unins000.exe"), vec![0u8; 500]).unwrap();
        let empty_alias = root.join("alias.exe");
        std::fs::write(&empty_alias, b"").unwrap();

        assert_eq!(icon_source(&shim), Some(root.join("Fake IDE").join("Fake IDE.exe")));
        assert_eq!(icon_source(&empty_alias), None, "0-byte app execution aliases have no icon");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_icon_output() {
        let out = "noise from profile\r\n0:iVBORw0KGgo=\r\n1:\r\n2:not base64!\r\n9:AAAA\r\n";
        let icons = parse_icon_output(out, 3);
        assert_eq!(icons[0].as_deref(), Some("data:image/png;base64,iVBORw0KGgo="));
        assert_eq!(icons[1], None);
        assert_eq!(icons[2], None);
    }

    /// Extracts real icons from this machine's installs. Run with:
    ///   cargo test --lib -- --ignored --nocapture explorer::tests::live_tool_icons
    #[tokio::test]
    #[ignore]
    async fn live_tool_icons() {
        let sources: Vec<_> = Tool::ALL
            .into_iter()
            .filter_map(|t| Some((t.id(), icon_source(&t.launcher(&HashMap::new())?)?)))
            .collect();
        println!("{sources:#?}");
        let paths: Vec<_> = sources.iter().map(|(_, p)| p.clone()).collect();
        let icons = extract_icons(&paths).await.expect("powershell");
        for ((id, path), icon) in sources.iter().zip(&icons) {
            println!("{id}: {} -> {} bytes", path.display(), icon.as_ref().map_or(0, |i| i.len()));
        }
        assert!(icons.iter().any(Option::is_some), "at least explorer.exe has an icon");
    }

    // Live tests against a running distro. Run with:
    //   WSL_TEST_DISTRO=Ubuntu cargo test --lib -- --ignored --nocapture --test-threads=1 explorer::tests::live_
    // Everything happens under /tmp/wsl-symbiosis-test-<pid> and is removed afterwards.

    #[tokio::test]
    #[ignore]
    async fn live_file_operations() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let root = format!("/tmp/wsl-symbiosis-test-{}", std::process::id());
        // Spaces, quotes, $, ;, backslash, unicode: must survive Windows → wsl.exe → sh.
        let weird = format!(r#"{root}/proj 'a' "b" $HOME; ç\x"#);

        mkdir(&distro, &format!("{weird}/node_modules/dep")).await.expect("mkdir");
        new_file(&distro, &format!("{weird}/package.json")).await.expect("touch");
        assert!(new_file(&distro, &format!("{weird}/package.json")).await.is_err(), "touch must refuse existing");

        let listing = list(&distro, &root, true).await.expect("list root");
        println!("{:#?}", listing.entries);
        let proj = &listing.entries[0];
        assert_eq!(proj.path, weird);
        assert_eq!(proj.project_markers, ["package.json", "node_modules"]);

        // Editor round trip: CRLF in, LF on disk, UTF-8 preserved.
        let file = format!("{weird}/notas.md");
        write_text(&distro, &file, "olá\r\nmundo ✓\r\n").await.expect("write");
        let read = read_text(&distro, &file, None).await.expect("read");
        assert_eq!(read.content, "olá\nmundo ✓\n");
        assert_eq!(read.line_ending, LineEnding::Lf);
        assert_eq!(read.size, "olá\nmundo ✓\n".len() as u64);
        let partial = read_text(&distro, &file, Some(4)).await.expect("read partial");
        assert!(partial.truncated);
        let bin = read_text(&distro, "/bin/sh", Some(4096)).await.expect("read binary");
        assert!(bin.binary);

        let renamed = format!("{weird}/README.md");
        rename(&distro, &file, &renamed).await.expect("rename");
        assert!(rename(&distro, &renamed, &format!("{weird}/package.json")).await.is_err(), "no overwrite");

        delete(&distro, &format!("{weird}/node_modules")).await.expect("delete node_modules");
        let after = list(&distro, &root, true).await.unwrap();
        assert_eq!(after.entries[0].project_markers, ["package.json"]);

        assert!(delete(&distro, "/home").await.is_err());
        assert!(delete(&distro, "/tmp/../etc").await.is_err());
        delete(&distro, &root).await.expect("cleanup");
        assert!(list(&distro, &root, false).await.is_err(), "root must be gone");

        let home = distro_home(&distro).await.expect("home");
        println!("home = {home}");
        assert!(home.starts_with('/'));
        println!("{:#?}", detect(&HashMap::new()));
    }
}
