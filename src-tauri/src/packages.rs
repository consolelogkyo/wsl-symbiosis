//! Development environment inventory (runtimes, version managers, npm globals) and
//! listening TCP ports of a distro.
//!
//! User tools live in the user's shell profile (nvm in ~/.zshrc, cargo in ~/.profile...), so
//! probes run through the user's *login shell* in interactive mode (`zsh -ic` / `bash -ic`),
//! falling back to `sh -c` on busybox distros. rc files may print anything, so every script
//! delimits its output with `@@section` markers. Windows drives (/mnt/<letter>/) are removed
//! from PATH first: they are slow to scan over 9P and their tools (e.g. Windows npm) belong to
//! Windows, not to the distro.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::wsl::{ensure_running, run, sh_in_distro};

const SCAN_TIMEOUT: Duration = Duration::from_secs(45);
const NPM_TIMEOUT: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Types (mirrored in src/lib/packages.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    Runtime,
    PackageManager,
    Tool,
    VersionManager,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedTool {
    pub id: String,
    pub label: String,
    pub category: ToolCategory,
    pub installed: bool,
    /// Clean version (`20.18.0`), when the tool reports one.
    pub version: Option<String>,
    /// First line of the version output, for tooltips.
    pub raw_version: Option<String>,
    /// `command -v` result: a path, or the name for shell functions (nvm).
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalPackage {
    pub name: String,
    pub version: String,
    /// Ships with Node itself (npm, corepack): uninstalling breaks the toolchain.
    pub bundled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeVersion {
    pub version: String,
    pub active: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NvmInfo {
    pub dir: String,
    pub versions: Vec<NodeVersion>,
    /// Raw content of `$NVM_DIR/alias/default` (`20`, `lts/*`, `v20.18.0`...).
    pub default_alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListeningPort {
    pub protocol: String,
    pub local_address: String,
    pub port: u16,
    /// `None` when the owner is in another distro or the WSL system itself (shared network,
    /// separate PID namespaces).
    pub pid: Option<u32>,
    pub process: Option<String>,
    /// Owner of the process (from /proc), when known.
    pub user: Option<String>,
    /// Working directory of the process: the project folder for dev servers.
    pub cwd: Option<String>,
    /// Command line (truncated), e.g. `node /home/kyo/app/node_modules/.bin/vite`.
    pub command: Option<String>,
}

/// A package installed globally in one of the nvm Node versions.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeGlobalPackage {
    /// `v20.18.0`
    pub node_version: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentScan {
    pub distro: String,
    /// How the probe ran, e.g. `/usr/bin/zsh -ic` or `sh -c`.
    pub shell: String,
    pub tools: Vec<DetectedTool>,
    pub nvm: Option<NvmInfo>,
    /// `None` when the distro has no (Linux) npm.
    pub npm_prefix: Option<String>,
    pub npm_globals: Vec<GlobalPackage>,
    /// Globals of every nvm Node version (read from disk, no npm call per version).
    pub nvm_globals: Vec<NodeGlobalPackage>,
    pub listening_ports: Vec<ListeningPort>,
    pub ports_error: Option<String>,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Tool catalog
// ---------------------------------------------------------------------------

struct ToolSpec {
    id: &'static str,
    label: &'static str,
    category: ToolCategory,
    /// Commands tried in order (python3 before python).
    commands: &'static [&'static str],
}

const TOOLS: &[ToolSpec] = &[
    ToolSpec { id: "node", label: "Node.js", category: ToolCategory::Runtime, commands: &["node"] },
    ToolSpec { id: "npm", label: "npm", category: ToolCategory::PackageManager, commands: &["npm"] },
    ToolSpec { id: "yarn", label: "Yarn", category: ToolCategory::PackageManager, commands: &["yarn"] },
    ToolSpec { id: "pnpm", label: "pnpm", category: ToolCategory::PackageManager, commands: &["pnpm"] },
    ToolSpec { id: "bun", label: "Bun", category: ToolCategory::Runtime, commands: &["bun"] },
    ToolSpec { id: "deno", label: "Deno", category: ToolCategory::Runtime, commands: &["deno"] },
    ToolSpec { id: "python", label: "Python", category: ToolCategory::Runtime, commands: &["python3", "python"] },
    ToolSpec { id: "pip", label: "pip", category: ToolCategory::PackageManager, commands: &["pip3", "pip"] },
    ToolSpec { id: "rustc", label: "Rust", category: ToolCategory::Runtime, commands: &["rustc"] },
    ToolSpec { id: "cargo", label: "Cargo", category: ToolCategory::PackageManager, commands: &["cargo"] },
    ToolSpec { id: "go", label: "Go", category: ToolCategory::Runtime, commands: &["go"] },
    ToolSpec { id: "java", label: "Java", category: ToolCategory::Runtime, commands: &["java"] },
    ToolSpec { id: "git", label: "Git", category: ToolCategory::Tool, commands: &["git"] },
    ToolSpec { id: "docker", label: "Docker", category: ToolCategory::Tool, commands: &["docker"] },
    ToolSpec { id: "gcc", label: "GCC", category: ToolCategory::Tool, commands: &["gcc"] },
    ToolSpec { id: "nvm", label: "nvm", category: ToolCategory::VersionManager, commands: &["nvm"] },
    ToolSpec { id: "fnm", label: "fnm", category: ToolCategory::VersionManager, commands: &["fnm"] },
    ToolSpec { id: "volta", label: "Volta", category: ToolCategory::VersionManager, commands: &["volta"] },
    ToolSpec { id: "pyenv", label: "pyenv", category: ToolCategory::VersionManager, commands: &["pyenv"] },
    ToolSpec { id: "rustup", label: "rustup", category: ToolCategory::VersionManager, commands: &["rustup"] },
    ToolSpec { id: "asdf", label: "asdf", category: ToolCategory::VersionManager, commands: &["asdf"] },
];

/// Pre-installed with Node; flagged so the UI does not offer to uninstall them casually.
const BUNDLED_NPM_PACKAGES: [&str; 2] = ["npm", "corepack"];

// ---------------------------------------------------------------------------
// Shell plumbing
// ---------------------------------------------------------------------------

/// Outer launcher (`sh -c`): `$1` is the real script, the rest are its arguments. Runs the
/// script in the user's login shell when it is zsh or bash (so ~/.zshrc / ~/.bashrc load
/// nvm & co.), else bash if present, else plain sh (busybox).
const LAUNCHER: &str = r#"s=$1; shift
u=$(id -un 2>/dev/null)
l=$(getent passwd "$u" 2>/dev/null | cut -d: -f7)
[ -n "$l" ] || l=$(grep "^$u:" /etc/passwd 2>/dev/null | cut -d: -f7)
case $l in
  */zsh|*/bash) [ -x "$l" ] && exec "$l" -ic "$s" "$l -ic" "$@" ;;
esac
[ -x /bin/bash ] && exec /bin/bash -ic "$s" "/bin/bash -ic" "$@"
exec /bin/sh -c "$s" "sh -c" "$@""#;

/// Prelude shared by user-shell scripts. Must work identically in zsh and sh: no reliance
/// on word splitting of unquoted variables (zsh does not split them), and `exit` inside a
/// pipeline only within `( )` (zsh runs the last pipeline element in the current shell).
///
/// Windows drives are found by mount type (9p with `aname=drvfs`), not by path: they are
/// `/mnt/c` in Ubuntu but `/mnt/host/c` in docker-desktop, and `automount.root` can move them.
const PRELUDE: &str = r#"unset HISTFILE
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0 COREPACK_ENABLE_NETWORK=0
wm=; while read -r dev mnt typ opts more; do case "$typ:$opts" in 9p:*aname=drvfs*|drvfs:*) wm="$wm$mnt
" ;; esac; done < /proc/mounts
on_windows() { printf '%s' "$wm" | ( while read -r m; do case "$1/" in "$m"/*) exit 0 ;; esac; done; exit 1 ); }
rest=$PATH:; P=
while [ -n "$rest" ]; do d=${rest%%:*}; rest=${rest#*:}; [ -n "$d" ] && ! on_windows "$d" && P=$P${P:+:}$d; done
PATH=$P; export PATH
"#;

pub(crate) fn user_shell_command(distro: &str, script: &str, args: &[&str]) -> tokio::process::Command {
    let full = format!("{PRELUDE}{script}");
    let mut all: Vec<&str> = vec![&full];
    all.extend_from_slice(args);
    sh_in_distro(distro, None, LAUNCHER, &all)
}

/// Runs an action wrapped as `@@out ... @@status N`, so rc-file noise printed before
/// `@@out` never leaks into results. Returns the command output (ANSI stripped).
async fn run_action(distro: &str, command: &str, args: &[&str]) -> Result<String, String> {
    let script = format!("printf '@@out\\n'\n{command} 2>&1\nprintf '\\n@@status %s\\n' \"$?\"\nexit 0");
    let text = run(user_shell_command(distro, &script, args), NPM_TIMEOUT).await?;
    parse_action_output(&text)
}

/// Like `run_action`, for multi-line scripts: the body runs in a subshell `( )`, so it may
/// `exit N` (that becomes the status) without killing the shell before `@@status`.
pub(crate) async fn run_user_script(distro: &str, body: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let script = format!("printf '@@out\\n'\n(\n{body}\n) 2>&1\nprintf '\\n@@status %s\\n' \"$?\"\nexit 0");
    let text = run(user_shell_command(distro, &script, args), timeout).await?;
    parse_action_output(&text)
}

pub fn parse_action_output(text: &str) -> Result<String, String> {
    let body = text.split_once("@@out\n").map(|(_, b)| b).ok_or("O shell da distro não executou o comando")?;
    let (output, status) = body.rsplit_once("@@status ").ok_or("Comando interrompido sem status")?;
    let output = strip_ansi(output).trim().to_string();
    match status.trim() {
        "0" => Ok(output),
        code => Err(if output.is_empty() { format!("Falhou com código {code}") } else { output }),
    }
}

pub(crate) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI: parameters until a final byte in @..~
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Environment scan
// ---------------------------------------------------------------------------

/// Version probes run in parallel (background jobs writing to a temp dir) under `timeout 5`;
/// yarn/pnpm corepack shims are kept offline by the prelude.
fn scan_script() -> String {
    let commands: Vec<&str> = TOOLS.iter().flat_map(|t| t.commands.iter().copied()).collect();
    format!(
        r#"printf '@@shell %s\n' "$0"
tmp=$(mktemp -d 2>/dev/null || echo /tmp/wsls-scan.$$); mkdir -p "$tmp"
probe() {{ if command -v timeout >/dev/null 2>&1; then timeout 5 "$@"; else "$@"; fi; }}
for t in {commands}; do
  p=$(command -v "$t" 2>/dev/null) || continue
  printf '%s' "$p" > "$tmp/$t.path"
  case $p in /*) ;; *) continue ;; esac
  case $t in
    java) probe java -version > "$tmp/$t.ver" 2>&1 < /dev/null & ;;
    go) probe go version > "$tmp/$t.ver" 2>&1 < /dev/null & ;;
    *) probe "$t" --version > "$tmp/$t.ver" 2>&1 < /dev/null & ;;
  esac
done
if [ -e "$tmp/npm.path" ]; then
  (probe npm prefix -g > "$tmp/npm.prefix" 2>/dev/null; probe npm list -g --depth=0 --json > "$tmp/npm.json" 2>/dev/null) < /dev/null &
fi
wait
for t in {commands}; do
  [ -e "$tmp/$t.path" ] || continue
  printf '@@tool %s\t%s\n' "$t" "$(cat "$tmp/$t.path")"
  [ -e "$tmp/$t.ver" ] && head -n 3 "$tmp/$t.ver"
done
if [ -e "$tmp/npm.path" ]; then
  printf '@@npmprefix\n'; cat "$tmp/npm.prefix" 2>/dev/null
  printf '\n@@npmjson\n'; cat "$tmp/npm.json" 2>/dev/null
fi
if [ -n "$NVM_DIR" ] && [ -d "$NVM_DIR/versions/node" ]; then
  printf '\n@@nvm %s\n' "$NVM_DIR"
  ls "$NVM_DIR/versions/node" 2>/dev/null
  printf '@@nvmalias\n'; cat "$NVM_DIR/alias/default" 2>/dev/null
  printf '\n@@nvmdefault\n'; command -v nvm >/dev/null 2>&1 && nvm version default 2>/dev/null
  # Globals of every version, straight from disk (find: no glob, so zsh can't fail on NOMATCH).
  printf '\n@@nvmglobals\n'
  find "$NVM_DIR/versions/node" -mindepth 5 -maxdepth 6 -path '*/lib/node_modules/*' -name package.json 2>/dev/null |
    while read -r f; do printf '%s\t%s\n' "$f" "$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' "$f" | head -n 1)"; done
fi
printf '\n@@end\n'
rm -rf "$tmp"
exit 0"#,
        commands = commands.join(" ")
    )
}

struct Section<'a> {
    arg: &'a str,
    lines: Vec<&'a str>,
}

/// Collects `@@name arg` sections in order (repeatable, e.g. one `@@tool` per tool).
/// Anything before the first marker (rc-file noise) is ignored.
fn sections(text: &str) -> Vec<(&str, Section<'_>)> {
    let mut out: Vec<(&str, Section)> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(header) = line.strip_prefix("@@") {
            let (name, arg) = header.split_once(' ').unwrap_or((header, ""));
            out.push((name, Section { arg, lines: Vec::new() }));
        } else if let Some((_, section)) = out.last_mut() {
            section.lines.push(line);
        }
    }
    out
}

/// First dotted number in `text` (`v20.18.0` → `20.18.0`, `go1.22.0` → `1.22.0`,
/// `gcc (Ubuntu 13.2.0-23ubuntu4) 13.2.0` → `13.2.0`, `pip 24.0 from ...` → `24.0`).
pub fn extract_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                if bytes[i] == b'.' {
                    if i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_digit() {
                        break;
                    }
                    dots += 1;
                }
                i += 1;
            }
            if dots > 0 {
                return Some(text[start..i].to_string());
            }
        }
        i += 1;
    }
    None
}

/// Builds the tool list from `@@tool <cmd>\t<path>` sections (+ version output lines).
pub fn parse_tools(found: &HashMap<String, (String, Vec<String>)>) -> Vec<DetectedTool> {
    TOOLS
        .iter()
        .map(|spec| {
            let hit = spec.commands.iter().find_map(|c| found.get(*c));
            let raw = hit.and_then(|(_, lines)| {
                lines.iter().map(|l| l.trim()).find(|l| !l.is_empty()).map(str::to_string)
            });
            // `java -version` prints `openjdk version "17.0.2"` on the first line already.
            let version = hit.and_then(|(_, lines)| lines.iter().find_map(|l| extract_version(l)));
            DetectedTool {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                category: spec.category,
                installed: hit.is_some(),
                version,
                raw_version: raw,
                path: hit.map(|(p, _)| p.clone()),
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct NpmList {
    #[serde(default)]
    dependencies: HashMap<String, NpmDep>,
}

#[derive(Deserialize)]
struct NpmDep {
    #[serde(default)]
    version: Option<String>,
}

/// Parses `npm list -g --depth=0 --json`. Tolerates leading noise before the JSON object.
pub fn parse_npm_globals(json: &str) -> Result<Vec<GlobalPackage>, String> {
    let start = json.find('{').ok_or("npm list não retornou JSON")?;
    let list: NpmList = serde_json::from_str(&json[start..]).map_err(|e| format!("JSON do npm inválido: {e}"))?;
    let mut packages: Vec<GlobalPackage> = list
        .dependencies
        .into_iter()
        .map(|(name, dep)| GlobalPackage {
            bundled: BUNDLED_NPM_PACKAGES.contains(&name.as_str()),
            version: dep.version.unwrap_or_default(),
            name,
        })
        .collect();
    packages.sort_by(|a, b| a.bundled.cmp(&b.bundled).then_with(|| a.name.cmp(&b.name)));
    Ok(packages)
}

/// nvm versions from `ls $NVM_DIR/versions/node`, the active node path and `nvm version default`.
pub fn parse_nvm(dir: &str, entries: &[&str], alias: Option<&str>, default: Option<&str>, node_path: Option<&str>) -> NvmInfo {
    let active = node_path.and_then(|p| {
        let rest = p.split("/versions/node/").nth(1)?;
        Some(rest.split('/').next()?.to_string())
    });
    let default = default.map(str::trim).filter(|d| d.starts_with('v'));
    let mut versions: Vec<NodeVersion> = entries
        .iter()
        .map(|e| e.trim())
        .filter(|e| e.starts_with('v'))
        .map(|v| NodeVersion {
            version: v.to_string(),
            active: active.as_deref() == Some(v),
            is_default: default == Some(v),
        })
        .collect();
    // Newest first, comparing numerically (v9 < v10).
    versions.sort_by(|a, b| version_key(&b.version).cmp(&version_key(&a.version)));
    NvmInfo {
        dir: dir.to_string(),
        versions,
        default_alias: alias.map(str::trim).filter(|a| !a.is_empty()).map(str::to_string),
    }
}

/// Lines `<.../versions/node/vX/lib/node_modules/[@scope/]name/package.json>\t<version>`.
/// Nested package.json files (inside a package) are skipped.
pub fn parse_nvm_globals(lines: &[&str]) -> Vec<NodeGlobalPackage> {
    let mut out: Vec<NodeGlobalPackage> = lines
        .iter()
        .filter_map(|line| {
            let (path, version) = line.split_once('\t')?;
            let rest = path.split("/versions/node/").nth(1)?;
            let (node_version, rest) = rest.split_once("/lib/node_modules/")?;
            let name = rest.strip_suffix("/package.json")?;
            let parts: Vec<&str> = name.split('/').collect();
            let valid = match parts.as_slice() {
                [n] => !n.starts_with('@') && !n.starts_with('.'),
                [scope, _] => scope.starts_with('@'),
                _ => false,
            };
            (valid && node_version.starts_with('v')).then(|| NodeGlobalPackage {
                node_version: node_version.to_string(),
                name: name.to_string(),
                version: version.trim().to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.name.cmp(&b.name).then_with(|| version_key(&b.node_version).cmp(&version_key(&a.node_version)))
    });
    out
}

fn version_key(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

struct UserScan {
    shell: String,
    tools: Vec<DetectedTool>,
    nvm: Option<NvmInfo>,
    npm_prefix: Option<String>,
    npm_globals: Vec<GlobalPackage>,
    nvm_globals: Vec<NodeGlobalPackage>,
}

fn parse_user_scan(text: &str) -> Result<UserScan, String> {
    let secs = sections(text);
    if !secs.iter().any(|(n, _)| *n == "end") {
        return Err("A sonda do ambiente não terminou (verifique o ~/.zshrc / ~/.bashrc)".into());
    }
    let one = |name: &str| secs.iter().find(|(n, _)| *n == name).map(|(_, s)| s);
    let joined = |name: &str| one(name).map(|s| s.lines.join("\n").trim().to_string()).filter(|s| !s.is_empty());

    let mut found: HashMap<String, (String, Vec<String>)> = HashMap::new();
    for (_, s) in secs.iter().filter(|(n, _)| *n == "tool") {
        if let Some((cmd, path)) = s.arg.split_once('\t') {
            let lines = s.lines.iter().map(|l| strip_ansi(l)).collect();
            found.insert(cmd.to_string(), (path.to_string(), lines));
        }
    }
    let tools = parse_tools(&found);

    let node_path = tools.iter().find(|t| t.id == "node").and_then(|t| t.path.clone());
    let nvm = one("nvm").map(|s| {
        let alias = joined("nvmalias");
        let default = joined("nvmdefault");
        parse_nvm(s.arg, &s.lines, alias.as_deref(), default.as_deref(), node_path.as_deref())
    });

    let has_npm = found.contains_key("npm");
    let npm_globals = match joined("npmjson") {
        Some(json) if has_npm => parse_npm_globals(&json).unwrap_or_default(),
        _ => Vec::new(),
    };

    Ok(UserScan {
        shell: one("shell").map(|s| s.arg.to_string()).unwrap_or_else(|| "sh -c".into()),
        tools,
        nvm,
        npm_prefix: if has_npm { joined("npmprefix") } else { None },
        npm_globals,
        nvm_globals: one("nvmglobals").map(|s| parse_nvm_globals(&s.lines)).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// Listening ports
// ---------------------------------------------------------------------------

/// Runs as root so `ss`/`netstat` can name processes of every user in this distro. Then,
/// per owning PID: user, working directory (the project folder for dev servers) and command.
const PORTS_SCRIPT: &str = r#"if command -v ss >/dev/null 2>&1; then echo @@ss; out=$(ss -tlnp)
elif netstat -tlnp >/dev/null 2>&1; then echo @@netstat; out=$(netstat -tlnp)
else echo @@proc; out=$(cat /proc/net/tcp /proc/net/tcp6 2>/dev/null)
fi
printf '%s\n' "$out"
echo @@owners
for p in $(printf '%s\n' "$out" | grep -oE 'pid=[0-9]+|[[:space:]][0-9]+/' | tr -dc '0-9\n' | sort -u); do
  [ -d "/proc/$p" ] || continue
  printf '%s\t%s\t%s\t%s\n' "$p" "$(stat -c %U "/proc/$p" 2>/dev/null)" "$(readlink "/proc/$p/cwd" 2>/dev/null)" \
    "$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null | cut -c1-240)"
done"#;

impl ListeningPort {
    fn new(local_address: String, port: u16, pid: Option<u32>, process: Option<String>) -> Self {
        Self { protocol: "tcp".into(), local_address, port, pid, process, user: None, cwd: None, command: None }
    }
}

/// `@@owners` lines: `pid\tuser\tcwd\tcmdline` → fills user/cwd/command of matching ports.
fn apply_owners(ports: &mut [ListeningPort], lines: &[&str]) {
    let non_empty = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(str::to_string);
    let owners: HashMap<u32, (Option<String>, Option<String>, Option<String>)> = lines
        .iter()
        .filter_map(|line| {
            let mut f = line.splitn(4, '\t');
            let pid = f.next()?.trim().parse().ok()?;
            let user = f.next().and_then(non_empty);
            let cwd = f.next().and_then(non_empty);
            let cmd = f.next().and_then(non_empty);
            Some((pid, (user, cwd, cmd)))
        })
        .collect();
    for port in ports {
        if let Some((user, cwd, cmd)) = port.pid.and_then(|pid| owners.get(&pid)) {
            port.user = user.clone();
            port.cwd = cwd.clone();
            port.command = cmd.clone();
        }
    }
}

/// `127.0.0.53%lo:53`, `[::]:22`, `*:80`, `:::80`, `[::ffff:127.0.0.1]:8080`.
pub fn split_address(text: &str) -> Option<(String, u16)> {
    let (addr, port) = text.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let addr = addr.trim_start_matches('[').trim_end_matches(']');
    let addr = addr.split('%').next().unwrap_or(addr);
    Some((if addr.is_empty() { "*".into() } else { addr.to_string() }, port))
}

/// `users:(("node",pid=3115,fd=20),("node",pid=3116,fd=20))` → first process.
fn parse_ss_users(text: &str) -> (Option<String>, Option<u32>) {
    let Some(start) = text.find("((\"") else { return (None, None) };
    let rest = &text[start + 3..];
    let name = rest.split('"').next().map(str::to_string);
    let pid = rest
        .split_once("pid=")
        .and_then(|(_, r)| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|p| p.parse().ok());
    (name, pid)
}

pub fn parse_ss(lines: &[&str]) -> Vec<ListeningPort> {
    lines
        .iter()
        .filter(|l| l.starts_with("LISTEN"))
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let (local_address, port) = split_address(tokens.get(3)?)?;
            let process_text = tokens.get(5..).map(|t| t.join(" ")).unwrap_or_default();
            let (process, pid) = parse_ss_users(&process_text);
            Some(ListeningPort::new(local_address, port, pid, process))
        })
        .collect()
}

/// net-tools / busybox: `tcp 0 0 0.0.0.0:22 0.0.0.0:* LISTEN 123/sshd` (or `-`).
pub fn parse_netstat(lines: &[&str]) -> Vec<ListeningPort> {
    lines
        .iter()
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if !tokens.first()?.starts_with("tcp") || tokens.get(5) != Some(&"LISTEN") {
                return None;
            }
            let (local_address, port) = split_address(tokens.get(3)?)?;
            let owner = tokens.get(6..).map(|t| t.join(" ")).unwrap_or_default();
            let (pid, process) = match owner.split_once('/') {
                Some((pid, name)) => (pid.parse().ok(), Some(name.to_string())),
                None => (None, None),
            };
            Some(ListeningPort::new(local_address, port, pid, process))
        })
        .collect()
}

/// Last resort: `/proc/net/tcp{,6}` (state 0A = LISTEN). No process information.
pub fn parse_proc_net_tcp(lines: &[&str]) -> Vec<ListeningPort> {
    lines
        .iter()
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.get(3) != Some(&"0A") {
                return None;
            }
            let (addr_hex, port_hex) = tokens.get(1)?.split_once(':')?;
            let port = u16::from_str_radix(port_hex, 16).ok()?;
            let local_address = match addr_hex.len() {
                8 => {
                    let n = u32::from_str_radix(addr_hex, 16).ok()?;
                    std::net::Ipv4Addr::from(n.swap_bytes()).to_string()
                }
                32 if addr_hex.chars().all(|c| c == '0') => "::".into(),
                32 if addr_hex == "00000000000000000000000001000000" => "::1".into(),
                _ => "ipv6".into(),
            };
            Some(ListeningPort::new(local_address, port, None, None))
        })
        .collect()
}

pub fn parse_ports(text: &str) -> Vec<ListeningPort> {
    let secs = sections(text);
    let mut ports = match secs.first() {
        Some(("ss", s)) => parse_ss(&s.lines),
        Some(("netstat", s)) => parse_netstat(&s.lines),
        Some(("proc", s)) => parse_proc_net_tcp(&s.lines),
        _ => Vec::new(),
    };
    if let Some((_, owners)) = secs.iter().find(|(n, _)| *n == "owners") {
        apply_owners(&mut ports, &owners.lines);
    }
    ports.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.local_address.cmp(&b.local_address)));
    ports.dedup_by(|a, b| a.port == b.port && a.local_address == b.local_address);
    ports
}

pub(crate) async fn listening_ports(distro: &str) -> Result<Vec<ListeningPort>, String> {
    ensure_running(distro).await?;
    let text = run(sh_in_distro(distro, Some("root"), PORTS_SCRIPT, &[]), SCAN_TIMEOUT).await?;
    Ok(parse_ports(&text))
}

pub(crate) async fn scan(distro: &str) -> Result<EnvironmentScan, String> {
    ensure_running(distro).await?;
    let started = Instant::now();
    let user = async {
        let text = run(user_shell_command(distro, &scan_script(), &[]), SCAN_TIMEOUT).await?;
        parse_user_scan(&text)
    };
    let ports = async { run(sh_in_distro(distro, Some("root"), PORTS_SCRIPT, &[]), SCAN_TIMEOUT).await };
    let (user, ports) = tokio::join!(user, ports);
    let user = user?;
    let (listening_ports, ports_error) = match ports {
        Ok(text) => (parse_ports(&text), None),
        Err(e) => (Vec::new(), Some(e)),
    };
    Ok(EnvironmentScan {
        distro: distro.to_string(),
        shell: user.shell,
        tools: user.tools,
        nvm: user.nvm,
        npm_prefix: user.npm_prefix,
        npm_globals: user.npm_globals,
        nvm_globals: user.nvm_globals,
        listening_ports,
        ports_error,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

// ---------------------------------------------------------------------------
// npm globals & nvm
// ---------------------------------------------------------------------------

fn valid_name_part(part: &str) -> bool {
    !part.is_empty()
        && !part.starts_with(['.', '_'])
        && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
}

/// `name`, `@scope/name`, optionally followed by `@<version|range|tag>` when
/// `allow_version`. Rejects flags (`-g`, `--prefix=...`), paths, URLs, git specs and spaces.
pub fn validate_package_spec(spec: &str, allow_version: bool) -> Result<(), String> {
    let invalid = || Err(format!("Nome de pacote inválido: {spec:?}"));
    if spec.is_empty() || spec.len() > 256 || spec.starts_with('-') {
        return invalid();
    }
    let (scope, rest) = match spec.strip_prefix('@') {
        Some(scoped) => match scoped.split_once('/') {
            Some((scope, rest)) => (Some(scope), rest),
            None => return invalid(),
        },
        None => (None, spec),
    };
    let (name, version) = match rest.split_once('@') {
        Some((name, version)) => (name, Some(version)),
        None => (rest, None),
    };
    if scope.is_some_and(|s| !valid_name_part(s)) || !valid_name_part(name) || name.len() > 214 {
        return invalid();
    }
    match version {
        None => Ok(()),
        Some(_) if !allow_version => Err("Informe só o nome do pacote, sem versão".into()),
        Some(v) if !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || ".^~<>=*+-".contains(c)) => Ok(()),
        Some(_) => invalid(),
    }
}

/// nvm version selectors offered by the UI: `v20.18.0`, `20`, `20.18`.
pub fn validate_node_version(version: &str) -> Result<(), String> {
    let digits = version.strip_prefix('v').unwrap_or(version);
    let ok = !digits.is_empty()
        && digits.split('.').count() <= 3
        && digits.split('.').all(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()));
    if ok { Ok(()) } else { Err(format!("Versão inválida: {version:?}")) }
}

/// Explains the most common global-install failure instead of a raw npm stack.
fn explain_npm_error(err: String) -> String {
    if err.contains("EACCES") {
        format!(
            "Sem permissão no prefixo global do npm (provavelmente o Node do sistema, em /usr). \
             Use o Node do nvm ou configure `npm config set prefix ~/.npm-global`.\n\n{err}"
        )
    } else {
        err
    }
}

const NPM_GUARD: &str =
    r#"command -v npm >/dev/null 2>&1 || { echo "npm nao encontrado na distro (o npm do Windows e ignorado)"; exit 127; }"#;

pub(crate) async fn npm_install(distro: &str, spec: &str) -> Result<String, String> {
    validate_package_spec(spec, true)?;
    ensure_running(distro).await?;
    let command = format!("({NPM_GUARD}; npm install -g -- \"$1\")");
    run_action(distro, &command, &[spec]).await.map_err(explain_npm_error)
}

pub(crate) async fn npm_uninstall(distro: &str, name: &str) -> Result<String, String> {
    validate_package_spec(name, false)?;
    ensure_running(distro).await?;
    let command = format!("({NPM_GUARD}; npm uninstall -g -- \"$1\")");
    run_action(distro, &command, &[name]).await.map_err(explain_npm_error)
}

pub(crate) async fn nvm_default(distro: &str, version: &str) -> Result<String, String> {
    validate_node_version(version)?;
    ensure_running(distro).await?;
    let command = r#"(command -v nvm >/dev/null 2>&1 || { echo "nvm nao carregado pelo shell do usuario"; exit 127; }
  v=$(nvm version "$1" 2>/dev/null); [ -n "$v" ] && [ "$v" != N/A ] || { echo "versao nao instalada: $1"; exit 1; }
  nvm alias default "$1")"#;
    run_action(distro, command, &[version]).await
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn scan_environment(distro: String) -> Result<EnvironmentScan, String> {
    scan(&distro).await
}

#[tauri::command]
pub async fn list_listening_ports(distro: String) -> Result<Vec<ListeningPort>, String> {
    listening_ports(&distro).await
}

#[tauri::command]
pub async fn install_npm_global(distro: String, package_spec: String) -> Result<String, String> {
    npm_install(&distro, &package_spec).await
}

#[tauri::command]
pub async fn uninstall_npm_global(distro: String, package_name: String) -> Result<String, String> {
    npm_uninstall(&distro, &package_name).await
}

#[tauri::command]
pub async fn switch_nvm_default(distro: String, version: String) -> Result<String, String> {
    nvm_default(&distro, &version).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_versions() {
        let cases = [
            ("v20.18.0", "20.18.0"),
            ("Python 3.12.3", "3.12.3"),
            ("pip 24.0 from /usr/lib/python3/dist-packages/pip (python 3.12)", "24.0"),
            ("rustc 1.75.0 (82e1608df 2023-12-21)", "1.75.0"),
            ("go version go1.22.0 linux/amd64", "1.22.0"),
            ("git version 2.43.0", "2.43.0"),
            ("Docker version 24.0.7, build afdd53b", "24.0.7"),
            ("openjdk version \"17.0.20.1\" 2026-08-18", "17.0.20.1"),
            ("gcc (Ubuntu 13.2.0-23ubuntu4) 13.2.0", "13.2.0"),
            ("deno 1.40.0 (release, x86_64-unknown-linux-gnu)", "1.40.0"),
            ("1.22.19", "1.22.19"),
        ];
        for (input, want) in cases {
            assert_eq!(extract_version(input).as_deref(), Some(want), "{input}");
        }
        assert_eq!(extract_version("no digits here"), None);
        assert_eq!(extract_version("build 2024"), None, "a plain number is not a version");
    }

    #[test]
    fn parses_scan_output_with_rc_noise() {
        let text = "Welcome to oh-my-zsh!\n\x1b[1mpowerline: not found\x1b[0m\n\
@@shell /usr/bin/zsh -ic\n\
@@tool node\t/home/kyo/.nvm/versions/node/v22.13.0/bin/node\nv22.13.0\n\
@@tool npm\t/home/kyo/.nvm/versions/node/v22.13.0/bin/npm\n10.9.2\n\
@@tool python3\t/usr/bin/python3\nPython 3.12.3\n\
@@tool java\t/usr/bin/java\nopenjdk version \"17.0.20.1\" 2026-08-18\nOpenJDK Runtime Environment\n\
@@tool nvm\tnvm\n\
@@npmprefix\n/home/kyo/.nvm/versions/node/v22.13.0\n\
@@npmjson\n{\"name\":\"lib\",\"dependencies\":{\"typescript\":{\"version\":\"5.7.2\"},\"npm\":{\"version\":\"10.9.2\"},\"@nestjs/cli\":{\"version\":\"10.4.9\"}}}\n\
@@nvm /home/kyo/.nvm\nv12.18.2\nv20.18.0\nv22.13.0\nv24.20.0\n\
@@nvmalias\n22\n@@nvmdefault\nv22.13.0\n@@end\n";
        let scan = parse_user_scan(text).unwrap();
        assert_eq!(scan.shell, "/usr/bin/zsh -ic");

        let tool = |id: &str| scan.tools.iter().find(|t| t.id == id).unwrap();
        assert_eq!(tool("node").version.as_deref(), Some("22.13.0"));
        assert_eq!(tool("python").version.as_deref(), Some("3.12.3"));
        assert_eq!(tool("python").path.as_deref(), Some("/usr/bin/python3"));
        assert_eq!(tool("java").version.as_deref(), Some("17.0.20.1"));
        assert!(tool("nvm").installed && tool("nvm").version.is_none());
        assert!(!tool("go").installed);

        assert_eq!(scan.npm_prefix.as_deref(), Some("/home/kyo/.nvm/versions/node/v22.13.0"));
        let names: Vec<&str> = scan.npm_globals.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["@nestjs/cli", "typescript", "npm"], "bundled last");
        assert!(scan.npm_globals[2].bundled);

        let nvm = scan.nvm.unwrap();
        let versions: Vec<&str> = nvm.versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(versions, ["v24.20.0", "v22.13.0", "v20.18.0", "v12.18.2"]);
        assert!(nvm.versions[1].active && nvm.versions[1].is_default);
        assert_eq!(nvm.default_alias.as_deref(), Some("22"));
    }

    #[test]
    fn scan_requires_end_marker() {
        assert!(parse_user_scan("@@shell sh -c\n@@tool git\t/usr/bin/git\n").is_err());
    }

    #[test]
    fn parses_npm_json() {
        let pkgs = parse_npm_globals("npm WARN config\n{\"dependencies\":{\"tsx\":{\"version\":\"4.19.2\"},\"corepack\":{\"version\":\"0.32.0\"}}}").unwrap();
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "tsx");
        assert!(pkgs[1].bundled);
        assert!(parse_npm_globals("{}").unwrap().is_empty());
        assert!(parse_npm_globals("not json").is_err());
    }

    #[test]
    fn parses_ss() {
        let lines = [
            "State  Recv-Q Send-Q  Local Address:Port  Peer Address:PortProcess",
            "LISTEN 0      4096    127.0.0.53%lo:53         0.0.0.0:*    users:((\"systemd-resolve\",pid=138,fd=14))",
            "LISTEN 0      1000   10.255.255.254:53         0.0.0.0:*",
            "LISTEN 0      511         127.0.0.1:44864      0.0.0.0:*    users:((\"node\",pid=3115,fd=20),(\"node\",pid=3116,fd=20))",
            "LISTEN 0      511              [::]:3000          [::]:*    users:((\"next-server (v1\",pid=42,fd=3))",
            "LISTEN 0      128                 *:8080             *:*",
        ];
        let ports = parse_ss(&lines);
        assert_eq!(ports.len(), 5);
        assert_eq!((ports[0].local_address.as_str(), ports[0].port), ("127.0.0.53", 53));
        assert_eq!(ports[0].process.as_deref(), Some("systemd-resolve"));
        assert_eq!(ports[0].pid, Some(138));
        assert_eq!(ports[1].pid, None);
        assert_eq!(ports[2].pid, Some(3115), "first owner wins");
        assert_eq!((ports[3].local_address.as_str(), ports[3].port), ("::", 3000));
        assert_eq!(ports[3].process.as_deref(), Some("next-server (v1"));
        assert_eq!(ports[4].local_address, "*");
    }

    #[test]
    fn parses_netstat_and_proc() {
        let lines = [
            "Active Internet connections (only servers)",
            "Proto Recv-Q Send-Q Local Address           Foreign Address         State       PID/Program name",
            "tcp        0      0 127.0.0.53:53           0.0.0.0:*               LISTEN      -",
            "tcp        0      0 0.0.0.0:22              0.0.0.0:*               LISTEN      812/sshd: /usr/sbin",
            "tcp6       0      0 :::80                   :::*                    LISTEN      99/nginx",
            "tcp        0      0 10.0.0.2:22             10.0.0.9:51234          ESTABLISHED 812/sshd",
        ];
        let ports = parse_netstat(&lines);
        assert_eq!(ports.len(), 3);
        assert_eq!(ports[0].pid, None);
        assert_eq!(ports[1].pid, Some(812));
        assert_eq!(ports[1].process.as_deref(), Some("sshd: /usr/sbin"));
        assert_eq!((ports[2].local_address.as_str(), ports[2].port), ("::", 80));

        let proc = [
            "  sl  local_address rem_address   st tx_queue rx_queue",
            "   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000",
            "   1: 00000000:0016 00000000:0000 0A 00000000:00000000",
            "   2: 0100007F:1F90 0100007F:D431 01 00000000:00000000",
            "   0: 00000000000000000000000000000000:0BB8 00000000000000000000000000000000:0000 0A 0",
        ];
        let ports = parse_proc_net_tcp(&proc);
        assert_eq!(ports.len(), 3);
        assert_eq!((ports[0].local_address.as_str(), ports[0].port), ("127.0.0.1", 8080));
        assert_eq!((ports[1].local_address.as_str(), ports[1].port), ("0.0.0.0", 22));
        assert_eq!((ports[2].local_address.as_str(), ports[2].port), ("::", 3000));
    }

    #[test]
    fn ports_are_sorted_and_deduped() {
        let text = "@@netstat\ntcp 0 0 0.0.0.0:8080 0.0.0.0:* LISTEN -\ntcp 0 0 0.0.0.0:22 0.0.0.0:* LISTEN -\ntcp 0 0 0.0.0.0:8080 0.0.0.0:* LISTEN -\n";
        let ports = parse_ports(text);
        assert_eq!(ports.iter().map(|p| p.port).collect::<Vec<_>>(), [22, 8080]);
    }

    #[test]
    fn ports_get_owner_cwd_and_command() {
        let text = "@@ss\n\
State Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
LISTEN 0 511 0.0.0.0:5173 0.0.0.0:* users:((\"node\",pid=4100,fd=22))\n\
LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:* users:((\"systemd-resolve\",pid=180,fd=14))\n\
@@owners\n\
180\tsystemd-resolve\t/\t/usr/lib/systemd/systemd-resolved \n\
4100\tkyo\t/home/kyo/dev/web\tnode /home/kyo/dev/web/node_modules/.bin/vite \n";
        let ports = parse_ports(text);
        assert_eq!(ports.len(), 2);
        let vite = ports.iter().find(|p| p.port == 5173).unwrap();
        assert_eq!(vite.user.as_deref(), Some("kyo"));
        assert_eq!(vite.cwd.as_deref(), Some("/home/kyo/dev/web"));
        assert_eq!(vite.command.as_deref(), Some("node /home/kyo/dev/web/node_modules/.bin/vite"));
        let dns = ports.iter().find(|p| p.port == 53).unwrap();
        assert_eq!(dns.cwd.as_deref(), Some("/"));
    }

    #[test]
    fn parses_globals_of_every_nvm_version() {
        let lines = [
            "/home/kyo/.nvm/versions/node/v18.20.4/lib/node_modules/eas-cli/package.json\t16.3.1",
            "/home/kyo/.nvm/versions/node/v22.11.0/lib/node_modules/npm/package.json\t10.9.0",
            "/home/kyo/.nvm/versions/node/v22.11.0/lib/node_modules/@nestjs/cli/package.json\t10.4.9",
            "/home/kyo/.nvm/versions/node/v18.20.4/lib/node_modules/npm/package.json\t10.7.0",
            // Nested package.json inside a package, and a stray scope-less path: ignored.
            "/home/kyo/.nvm/versions/node/v22.11.0/lib/node_modules/npm/docs/package.json\t1.0.0",
            "/home/kyo/.nvm/versions/node/v22.11.0/lib/node_modules/.bin/package.json\t1.0.0",
        ];
        let globals = parse_nvm_globals(&lines);
        let view: Vec<(&str, &str, &str)> =
            globals.iter().map(|g| (g.name.as_str(), g.node_version.as_str(), g.version.as_str())).collect();
        assert_eq!(
            view,
            [
                ("@nestjs/cli", "v22.11.0", "10.4.9"),
                ("eas-cli", "v18.20.4", "16.3.1"),
                ("npm", "v22.11.0", "10.9.0"),
                ("npm", "v18.20.4", "10.7.0"),
            ]
        );
    }

    #[test]
    fn validates_package_specs() {
        for ok in ["typescript", "vercel", "@nestjs/cli", "tsx@4.19.2", "@angular/cli@^17.0.0", "pnpm@latest", "lodash.merge", "JSONStream", "npm@>=10"] {
            assert!(validate_package_spec(ok, true).is_ok(), "{ok} should be valid");
        }
        for bad in [
            "-g", "--prefix=/etc", "--unsafe-perm", "", "@scope", "@/cli", "../evil", "./local",
            "git+https://x/y.git", "github:user/repo", "file:/tmp/x", "https://evil.sh/pkg.tgz",
            "a b", "pkg;rm -rf /", "pkg$(id)", "pkg@1.0 || 2", "_private", ".hidden", "a/b",
        ] {
            assert!(validate_package_spec(bad, true).is_err(), "{bad:?} should be rejected");
        }
        assert!(validate_package_spec("typescript@5", false).is_err(), "uninstall takes a bare name");
        assert!(validate_package_spec("@nestjs/cli", false).is_ok());
    }

    #[test]
    fn validates_node_versions() {
        for ok in ["v20.18.0", "20", "20.18", "v24.20.0"] {
            assert!(validate_node_version(ok).is_ok(), "{ok}");
        }
        for bad in ["", "v", "lts/*", "node", "20.1.2.3", "-v20", "v20; rm -rf ~", "v20.x"] {
            assert!(validate_node_version(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn parses_action_output() {
        assert_eq!(parse_action_output("rc noise\n@@out\nadded 1 package in 2s\n\n@@status 0\n"), Ok("added 1 package in 2s".into()));
        let err = parse_action_output("@@out\n\x1b[31mnpm error\x1b[0m 404 Not Found\n@@status 1\n").unwrap_err();
        assert_eq!(err, "npm error 404 Not Found");
        assert!(parse_action_output("nothing").is_err());
        assert!(explain_npm_error("npm error code EACCES".into()).contains("nvm"));
    }

    // Live tests. Run with:
    //   WSL_TEST_DISTRO=Ubuntu cargo test --lib -- --ignored --nocapture --test-threads=1 packages::tests::live_

    #[tokio::test]
    #[ignore]
    async fn live_scan() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let s = scan(&distro).await.expect("scan");
        println!("shell = {} · {} ms", s.shell, s.duration_ms);
        for t in s.tools.iter().filter(|t| t.installed) {
            println!("  {:<8} {:<12} {}", t.id, t.version.as_deref().unwrap_or("-"), t.path.as_deref().unwrap_or(""));
        }
        println!("nvm = {:#?}", s.nvm);
        println!("npm prefix = {:?}; globals = {:?}", s.npm_prefix, s.npm_globals);
        for p in &s.listening_ports {
            println!("  :{:<6} {:<16} {:?} {:?}", p.port, p.local_address, p.pid, p.process);
        }
        assert!(s.ports_error.is_none(), "{:?}", s.ports_error);
        let windows = |p: &str| p.starts_with("/mnt/c/") || p.starts_with("/mnt/host/");
        assert!(s.tools.iter().all(|t| t.path.as_deref().is_none_or(|p| !windows(p))), "Windows tools leaked");
    }

    /// Starts a real HTTP server in the distro, finds it by port and frees the port with the
    /// same kill command the UI uses.
    #[tokio::test]
    #[ignore]
    async fn live_port_killer() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let mut server = crate::wsl::wsl_command();
        server.args(["-d", &distro, "--exec", "python3", "-m", "http.server", "8765", "--bind", "127.0.0.1"]);
        let _child = server.spawn().expect("spawn server");
        tokio::time::sleep(Duration::from_secs(2)).await;

        let ports = listening_ports(&distro).await.expect("ports");
        let p = ports.iter().find(|p| p.port == 8765).expect("server listening on 8765");
        println!("found {p:?}");
        assert_eq!(p.local_address, "127.0.0.1");
        assert!(p.process.as_deref().is_some_and(|n| n.starts_with("python")));

        crate::metrics::kill_process(&distro, p.pid.expect("pid"), Some("TERM")).await.expect("kill");
        tokio::time::sleep(Duration::from_millis(800)).await;
        let after = listening_ports(&distro).await.unwrap();
        assert!(!after.iter().any(|p| p.port == 8765), "port still busy");
    }

    /// Opt-in: installs and removes a tiny real package (`is-number`) globally.
    #[tokio::test]
    #[ignore]
    async fn live_npm_roundtrip() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        assert!(npm_install(&distro, "--prefix=/tmp").await.is_err());
        let out = npm_install(&distro, "is-number@7.0.0").await.expect("install");
        println!("install: {out}");
        let s = scan(&distro).await.unwrap();
        assert!(s.npm_globals.iter().any(|p| p.name == "is-number" && p.version == "7.0.0"));
        let out = npm_uninstall(&distro, "is-number").await.expect("uninstall");
        println!("uninstall: {out}");
        let s = scan(&distro).await.unwrap();
        assert!(!s.npm_globals.iter().any(|p| p.name == "is-number"));
    }

    /// Opt-in: switches the nvm default to another installed version and restores it.
    #[tokio::test]
    #[ignore]
    async fn live_nvm_switch() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let nvm = scan(&distro).await.unwrap().nvm.expect("nvm present");
        let original = nvm.default_alias.clone().expect("has default alias");
        let other = nvm.versions.iter().find(|v| !v.is_default).expect("2+ versions").version.clone();
        println!("{}", nvm_default(&distro, &other).await.expect("switch"));
        let after = scan(&distro).await.unwrap().nvm.unwrap();
        assert!(after.versions.iter().any(|v| v.version == other && v.is_default));
        assert!(nvm_default(&distro, "v99.0.0").await.is_err(), "not installed must fail");
        // Restore the exact alias the user had (e.g. "22" or "lts/*" is restored via its
        // resolved version when it is not a plain version number).
        let restore = if validate_node_version(&original).is_ok() {
            original.clone()
        } else {
            nvm.versions.iter().find(|v| v.is_default).unwrap().version.clone()
        };
        nvm_default(&distro, &restore).await.expect("restore");
        let restored = scan(&distro).await.unwrap().nvm.unwrap();
        println!("restored alias: {:?} (original {:?})", restored.default_alias, original);
    }
}
