//! Visual editor for `%USERPROFILE%\.wslconfig` (global limits of the WSL2 VM).
//!
//! The file is the user's: edits are non-destructive. Only the keys whose value changed in the
//! UI are touched, in place (original key spelling and inline comments kept); comments, unknown
//! keys and other sections stay byte for byte. A backup (`.wslconfig.symbiosis.bak`) is made
//! on the first edit.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

const WSL2: &str = "wsl2";
const EXPERIMENTAL: &str = "experimental";
const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

pub const NETWORKING_MODES: [&str; 3] = ["nat", "mirrored", "virtioproxy"];
pub const MEMORY_RECLAIM_MODES: [&str; 3] = ["disabled", "gradual", "dropcache"];

// ---------------------------------------------------------------------------
// Types (mirrored in src/lib/wslconfig.ts)
// ---------------------------------------------------------------------------

/// `None` = key absent, WSL's default applies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WslGlobalConfig {
    /// `[wsl2] memory`, bytes. Default: 50% of the host RAM.
    pub memory: Option<u64>,
    /// `[wsl2] processors`. Default: every logical CPU.
    pub processors: Option<u32>,
    /// `[wsl2] swap`, bytes; `Some(0)` disables swap. Default: 25% of the host RAM.
    pub swap: Option<u64>,
    pub localhost_forwarding: Option<bool>,
    /// `nat` | `mirrored` (| `virtioproxy`).
    pub networking_mode: Option<String>,
    pub gui_applications: Option<bool>,
    /// `[experimental] autoMemoryReclaim`: `disabled` | `gradual` | `dropcache`.
    pub auto_memory_reclaim: Option<String>,
    /// `[experimental] sparseVhd` (new distros only; existing ones need `wsl --manage --set-sparse`).
    pub sparse_vhd: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslConfigSnapshot {
    pub config: WslGlobalConfig,
    pub path: String,
    pub exists: bool,
    /// The backup exists (made on the first edit from the app).
    pub has_backup: bool,
    /// Keys the editor doesn't manage, kept untouched (`wsl2.kernel`, ...).
    pub preserved: Vec<String>,
    /// Limits for the sliders.
    pub host_memory: u64,
    pub host_cpus: u32,
}

// ---------------------------------------------------------------------------
// INI document (line-preserving)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct IniDoc {
    lines: Vec<String>,
    crlf: bool,
    bom: bool,
}

enum Line<'a> {
    Section(&'a str),
    Key { key: &'a str, value: &'a str },
    Other,
}

fn classify(line: &str) -> Line<'_> {
    let t = line.trim();
    if t.starts_with('#') || t.starts_with(';') || t.is_empty() {
        return Line::Other;
    }
    if let Some(name) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Line::Section(name.trim());
    }
    match t.split_once('=') {
        Some((key, value)) if !key.trim().is_empty() => Line::Key { key: key.trim(), value },
        _ => Line::Other,
    }
}

/// `value  # comment` → (`value`, `  # comment`).
fn split_comment(value: &str) -> (&str, &str) {
    match value.find('#') {
        Some(i) => {
            let start = value[..i].trim_end().len();
            (value[..start].trim(), &value[start..])
        }
        None => (value.trim(), ""),
    }
}

impl IniDoc {
    pub fn parse(text: &str) -> Self {
        let bom = text.starts_with('\u{FEFF}');
        let text = text.trim_start_matches('\u{FEFF}');
        let crlf = text.contains("\r\n") || text.is_empty();
        let mut lines: Vec<String> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        Self { lines, crlf, bom }
    }

    pub fn render(&self) -> String {
        let nl = if self.crlf { "\r\n" } else { "\n" };
        let mut out = String::new();
        if self.bom {
            out.push('\u{FEFF}');
        }
        for line in &self.lines {
            out.push_str(line);
            out.push_str(nl);
        }
        out
    }

    /// Section name of every line (`None` before the first header).
    fn sections(&self) -> Vec<Option<String>> {
        let mut current = None;
        self.lines
            .iter()
            .map(|line| {
                if let Line::Section(name) = classify(line) {
                    current = Some(name.to_ascii_lowercase());
                }
                current.clone()
            })
            .collect()
    }

    /// Indices of `key` lines inside `section` (both case-insensitive), in file order.
    fn find(&self, section: &str, key: &str) -> Vec<usize> {
        let sections = self.sections();
        self.lines
            .iter()
            .enumerate()
            .filter(|(i, line)| {
                sections[*i].as_deref() == Some(section)
                    && matches!(classify(line), Line::Key { key: k, .. } if k.eq_ignore_ascii_case(key))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The last occurrence wins, like WSL's own parser.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let i = *self.find(section, key).last()?;
        match classify(&self.lines[i]) {
            Line::Key { value, .. } => Some(split_comment(value).0.to_string()),
            _ => None,
        }
    }

    pub fn has(&self, section: &str, key: &str) -> bool {
        !self.find(section, key).is_empty()
    }

    /// `Some`: replaces the value in place (keeping the key spelling, indentation and inline
    /// comment) or inserts it at the end of the section (created if missing).
    /// `None`: removes the key, so WSL's default applies.
    pub fn set(&mut self, section: &str, key: &str, value: Option<&str>) {
        let found = self.find(section, key);
        let Some(value) = value else {
            for i in found.into_iter().rev() {
                self.lines.remove(i);
            }
            return;
        };
        if let Some(&i) = found.last() {
            let line = &self.lines[i];
            let indent = &line[..line.len() - line.trim_start().len()];
            let (k, old) = line.trim_start().split_once('=').unwrap_or((key, ""));
            let comment = split_comment(old).1.to_string();
            self.lines[i] = format!("{indent}{}={value}{comment}", k.trim_end());
            return;
        }
        let sections = self.sections();
        let last_header = (0..self.lines.len())
            .rev()
            .find(|&i| matches!(classify(&self.lines[i]), Line::Section(_)) && sections[i].as_deref() == Some(section));
        match last_header {
            Some(header) => {
                // After the section's last key: blank lines and comments that introduce the next
                // section stay with it.
                let mut at = header + 1;
                let mut i = header + 1;
                while i < self.lines.len() && !matches!(classify(&self.lines[i]), Line::Section(_)) {
                    if matches!(classify(&self.lines[i]), Line::Key { .. }) {
                        at = i + 1;
                    }
                    i += 1;
                }
                self.lines.insert(at, format!("{key}={value}"));
            }
            None => {
                if self.lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    self.lines.push(String::new());
                }
                self.lines.push(format!("[{section}]"));
                self.lines.push(format!("{key}={value}"));
            }
        }
    }

    /// Updates the key where the user has it (`primary`, or `fallback` for settings WSL moved
    /// between sections over time), else writes it in `primary`. `None` removes it from both.
    fn set_either(&mut self, primary: &str, fallback: &str, key: &str, value: Option<&str>) {
        match value {
            None => {
                self.set(primary, key, None);
                self.set(fallback, key, None);
            }
            Some(_) if !self.has(primary, key) && self.has(fallback, key) => self.set(fallback, key, value),
            Some(_) => self.set(primary, key, value),
        }
    }

    fn get_either(&self, primary: &str, fallback: &str, key: &str) -> Option<String> {
        self.get(primary, key).or_else(|| self.get(fallback, key))
    }

    /// `section.key` of every key outside the editor's set.
    fn other_keys(&self) -> Vec<String> {
        let sections = self.sections();
        let managed = |s: &str, k: &str| {
            MANAGED.iter().any(|(_, key)| key.eq_ignore_ascii_case(k)) && (s == WSL2 || s == EXPERIMENTAL)
        };
        let mut out = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            if let Line::Key { key, .. } = classify(line) {
                let section = sections[i].as_deref().unwrap_or("");
                let name = format!("{section}.{key}");
                if !managed(section, key) && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }
}

const MANAGED: [(&str, &str); 8] = [
    (WSL2, "memory"),
    (WSL2, "processors"),
    (WSL2, "swap"),
    (WSL2, "localhostForwarding"),
    (WSL2, "networkingMode"),
    (WSL2, "guiApplications"),
    (EXPERIMENTAL, "autoMemoryReclaim"),
    (EXPERIMENTAL, "sparseVhd"),
];

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// `8GB`, `512MB`, `4G`, `1.5GB`, `0`, bare bytes. Units are binary (WSL's own reading).
pub fn parse_size(raw: &str) -> Option<u64> {
    let s = raw.trim().trim_matches('"').to_ascii_uppercase();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (number, unit) = s.split_at(split);
    let number: f64 = number.parse().ok()?;
    let multiplier = match unit.trim() {
        "" | "B" => 1.0,
        "K" | "KB" => 1024.0,
        "M" | "MB" => MIB as f64,
        "G" | "GB" => GIB as f64,
        "T" | "TB" => (GIB * 1024) as f64,
        _ => return None,
    };
    (number >= 0.0).then(|| (number * multiplier).round() as u64)
}

/// The shortest exact spelling: `0`, `8GB`, `1536MB`.
pub fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        "0".into()
    } else if bytes % GIB == 0 {
        format!("{}GB", bytes / GIB)
    } else if bytes % MIB == 0 {
        format!("{}MB", bytes / MIB)
    } else {
        format!("{}KB", bytes.div_ceil(1024))
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_word(raw: Option<String>) -> Option<String> {
    raw.map(|v| v.trim().trim_matches('"').to_ascii_lowercase()).filter(|v| !v.is_empty())
}

pub fn read_config(doc: &IniDoc) -> WslGlobalConfig {
    WslGlobalConfig {
        memory: doc.get(WSL2, "memory").as_deref().and_then(parse_size),
        processors: doc.get(WSL2, "processors").and_then(|v| v.trim().parse().ok()),
        swap: doc.get(WSL2, "swap").as_deref().and_then(parse_size),
        localhost_forwarding: doc.get(WSL2, "localhostForwarding").as_deref().and_then(parse_bool),
        networking_mode: parse_word(doc.get_either(WSL2, EXPERIMENTAL, "networkingMode")),
        gui_applications: doc.get(WSL2, "guiApplications").as_deref().and_then(parse_bool),
        auto_memory_reclaim: parse_word(doc.get_either(EXPERIMENTAL, WSL2, "autoMemoryReclaim")),
        sparse_vhd: doc.get_either(EXPERIMENTAL, WSL2, "sparseVhd").as_deref().and_then(parse_bool),
    }
}

pub fn validate(config: &WslGlobalConfig, host_memory: u64, host_cpus: u32) -> Result<(), String> {
    if let Some(m) = config.memory {
        if m < GIB {
            return Err("Limite de memória mínimo: 1 GB".into());
        }
        if host_memory > 0 && m > host_memory {
            return Err(format!("O limite de memória passa da RAM do Windows ({})", format_size(host_memory / GIB * GIB)));
        }
    }
    if let Some(p) = config.processors {
        if p == 0 || (host_cpus > 0 && p > host_cpus) {
            return Err(format!("Processadores: de 1 a {host_cpus}"));
        }
    }
    if let Some(s) = config.swap {
        if s > 256 * GIB {
            return Err("Swap máximo: 256 GB".into());
        }
    }
    if let Some(mode) = &config.networking_mode {
        if !NETWORKING_MODES.contains(&mode.as_str()) {
            return Err(format!("Modo de rede inválido: {mode}"));
        }
    }
    if let Some(mode) = &config.auto_memory_reclaim {
        if !MEMORY_RECLAIM_MODES.contains(&mode.as_str()) {
            return Err(format!("autoMemoryReclaim inválido: {mode}"));
        }
    }
    Ok(())
}

/// Writes only the fields that differ between `old` (what the file says) and `new`.
/// Returns whether anything changed.
pub fn apply(doc: &mut IniDoc, old: &WslGlobalConfig, new: &WslGlobalConfig) -> bool {
    let mut changed = false;
    let bool_str = |b: bool| if b { "true" } else { "false" };
    macro_rules! field {
        ($name:ident, $write:expr) => {
            if old.$name != new.$name {
                changed = true;
                $write(doc, new.$name.clone());
            }
        };
    }
    field!(memory, |d: &mut IniDoc, v: Option<u64>| d.set(WSL2, "memory", v.map(format_size).as_deref()));
    field!(processors, |d: &mut IniDoc, v: Option<u32>| d.set(WSL2, "processors", v.map(|p| p.to_string()).as_deref()));
    field!(swap, |d: &mut IniDoc, v: Option<u64>| d.set(WSL2, "swap", v.map(format_size).as_deref()));
    field!(localhost_forwarding, |d: &mut IniDoc, v: Option<bool>| d.set(WSL2, "localhostForwarding", v.map(bool_str)));
    field!(networking_mode, |d: &mut IniDoc, v: Option<String>| d.set_either(WSL2, EXPERIMENTAL, "networkingMode", v.as_deref()));
    field!(gui_applications, |d: &mut IniDoc, v: Option<bool>| d.set(WSL2, "guiApplications", v.map(bool_str)));
    field!(auto_memory_reclaim, |d: &mut IniDoc, v: Option<String>| d.set_either(EXPERIMENTAL, WSL2, "autoMemoryReclaim", v.as_deref()));
    field!(sparse_vhd, |d: &mut IniDoc, v: Option<bool>| d.set_either(EXPERIMENTAL, WSL2, "sparseVhd", v.map(bool_str)));
    changed
}

// ---------------------------------------------------------------------------
// File
// ---------------------------------------------------------------------------

fn config_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE").ok_or("USERPROFILE não definido")?;
    Ok(PathBuf::from(home).join(".wslconfig"))
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name(".wslconfig.symbiosis.bak")
}

fn read_doc(path: &Path) -> Result<Option<IniDoc>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("{} não está em UTF-8: edite-o à mão ou salve-o como UTF-8", path.display()))?;
            Ok(Some(IniDoc::parse(&text)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Falha ao ler {}: {e}", path.display())),
    }
}

fn host_limits() -> (u64, u32) {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let cpus = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1);
    (sys.total_memory(), cpus)
}

fn snapshot_of(path: &Path, doc: Option<&IniDoc>) -> WslConfigSnapshot {
    let (host_memory, host_cpus) = host_limits();
    WslConfigSnapshot {
        config: doc.map(read_config).unwrap_or_default(),
        path: path.display().to_string(),
        exists: doc.is_some(),
        has_backup: backup_path(path).exists(),
        preserved: doc.map(IniDoc::other_keys).unwrap_or_default(),
        host_memory,
        host_cpus,
    }
}

/// Writes `doc` to `path` (backup of the original on the first edit, then temp file + rename).
fn write_doc(path: &Path, doc: &IniDoc, existed: bool) -> Result<(), String> {
    let backup = backup_path(path);
    if existed && !backup.exists() {
        std::fs::copy(path, &backup).map_err(|e| format!("Falha ao criar o backup {}: {e}", backup.display()))?;
    }
    let tmp = path.with_file_name(".wslconfig.symbiosis.tmp");
    std::fs::write(&tmp, doc.render()).map_err(|e| format!("Falha ao gravar {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Falha ao gravar {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_wsl_config() -> Result<WslConfigSnapshot, String> {
    let path = config_path()?;
    let doc = read_doc(&path)?;
    Ok(snapshot_of(&path, doc.as_ref()))
}

#[tauri::command]
pub async fn save_wsl_config(app: AppHandle, config: WslGlobalConfig, restart_wsl: bool) -> Result<WslConfigSnapshot, String> {
    let path = config_path()?;
    let existing = read_doc(&path)?;
    let (host_memory, host_cpus) = host_limits();
    let mut doc = existing.clone().unwrap_or_else(|| IniDoc::parse(""));
    let old = read_config(&doc);
    // Only what changed is validated: a hand-written value the editor can't read is kept.
    let mut check = WslGlobalConfig::default();
    if config.memory != old.memory { check.memory = config.memory; }
    if config.processors != old.processors { check.processors = config.processors; }
    if config.swap != old.swap { check.swap = config.swap; }
    if config.networking_mode != old.networking_mode { check.networking_mode = config.networking_mode.clone(); }
    if config.auto_memory_reclaim != old.auto_memory_reclaim { check.auto_memory_reclaim = config.auto_memory_reclaim.clone(); }
    validate(&check, host_memory, host_cpus)?;

    if apply(&mut doc, &old, &config) {
        write_doc(&path, &doc, existing.is_some())?;
    }
    if restart_wsl {
        // The new limits apply when the VM boots again.
        crate::wsl::shutdown_all(&app).await?;
    }
    Ok(snapshot_of(&path, Some(&doc)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_FILE: &str = "# Minhas configs do WSL\r\n[wsl2]\r\nMemory=6GB   # metade da RAM\r\nkernel=C:\\\\wsl\\\\bzImage\r\nnestedVirtualization=true\r\n\r\n; rede\r\n[experimental]\r\nhostAddressLoopback=true\r\n\r\n[boot]\r\nsystemd=true\r\n";

    #[test]
    fn reads_values_and_units() {
        let doc = IniDoc::parse(USER_FILE);
        let c = read_config(&doc);
        assert_eq!(c.memory, Some(6 * GIB), "keys are case-insensitive, inline comments ignored");
        assert_eq!(c.processors, None);
        assert_eq!(doc.get("wsl2", "KERNEL").as_deref(), Some(r"C:\\wsl\\bzImage"));
        assert_eq!(parse_size("512MB"), Some(512 * MIB));
        assert_eq!(parse_size("4g"), Some(4 * GIB));
        assert_eq!(parse_size("1.5GB"), Some(3 * GIB / 2));
        assert_eq!(parse_size("0"), Some(0));
        assert_eq!(parse_size("muito"), None);
        assert_eq!(parse_size("8XB"), None);
        assert_eq!(format_size(8 * GIB), "8GB");
        assert_eq!(format_size(1536 * MIB), "1536MB");
        assert_eq!(format_size(0), "0");
    }

    #[test]
    fn untouched_when_nothing_changes() {
        let mut doc = IniDoc::parse(USER_FILE);
        let old = read_config(&doc);
        assert!(!apply(&mut doc, &old, &old.clone()));
        assert_eq!(doc.render(), USER_FILE, "byte for byte, CRLF included");
    }

    #[test]
    fn edits_in_place_and_keeps_everything_else() {
        let mut doc = IniDoc::parse(USER_FILE);
        let old = read_config(&doc);
        let new = WslGlobalConfig {
            memory: Some(8 * GIB),
            processors: Some(4),
            swap: Some(0),
            auto_memory_reclaim: Some("gradual".into()),
            sparse_vhd: Some(true),
            ..old.clone()
        };
        assert!(apply(&mut doc, &old, &new));
        let out = doc.render();
        assert_eq!(
            out,
            "# Minhas configs do WSL\r\n[wsl2]\r\nMemory=8GB   # metade da RAM\r\nkernel=C:\\\\wsl\\\\bzImage\r\nnestedVirtualization=true\r\nprocessors=4\r\nswap=0\r\n\r\n; rede\r\n[experimental]\r\nhostAddressLoopback=true\r\nautoMemoryReclaim=gradual\r\nsparseVhd=true\r\n\r\n[boot]\r\nsystemd=true\r\n"
        );
        assert_eq!(read_config(&IniDoc::parse(&out)), new, "round trip");
        assert_eq!(IniDoc::parse(&out).other_keys(), ["wsl2.kernel", "wsl2.nestedVirtualization", "experimental.hostAddressLoopback", "boot.systemd"]);
    }

    #[test]
    fn removing_a_value_restores_the_default() {
        let mut doc = IniDoc::parse(USER_FILE);
        let old = read_config(&doc);
        apply(&mut doc, &old, &WslGlobalConfig { memory: None, ..old.clone() });
        let out = doc.render();
        assert!(!out.to_lowercase().contains("memory="));
        assert!(out.contains("kernel=") && out.contains("[boot]"));
    }

    #[test]
    fn creates_file_and_sections() {
        let mut doc = IniDoc::parse("");
        let new = WslGlobalConfig {
            memory: Some(4 * GIB),
            networking_mode: Some("mirrored".into()),
            auto_memory_reclaim: Some("dropcache".into()),
            ..Default::default()
        };
        apply(&mut doc, &WslGlobalConfig::default(), &new);
        assert_eq!(doc.render(), "[wsl2]\r\nmemory=4GB\r\nnetworkingMode=mirrored\r\n\r\n[experimental]\r\nautoMemoryReclaim=dropcache\r\n");

        // LF files stay LF; a section is appended after the last one.
        let mut doc = IniDoc::parse("[boot]\nsystemd=true\n");
        apply(&mut doc, &WslGlobalConfig::default(), &WslGlobalConfig { swap: Some(2 * GIB), ..Default::default() });
        assert_eq!(doc.render(), "[boot]\nsystemd=true\n\n[wsl2]\nswap=2GB\n");
    }

    #[test]
    fn follows_keys_moved_between_sections() {
        // Older WSL builds documented networkingMode under [experimental]: edit it where it is.
        let text = "[experimental]\nnetworkingMode=NAT\n";
        let mut doc = IniDoc::parse(text);
        let old = read_config(&doc);
        assert_eq!(old.networking_mode.as_deref(), Some("nat"));
        apply(&mut doc, &old, &WslGlobalConfig { networking_mode: Some("mirrored".into()), ..old.clone() });
        assert_eq!(doc.render(), "[experimental]\nnetworkingMode=mirrored\n");
        // Back to the default: gone from both sections.
        let old = read_config(&doc);
        apply(&mut doc, &old, &WslGlobalConfig { networking_mode: None, ..old.clone() });
        assert_eq!(doc.render(), "[experimental]\n");
    }

    #[test]
    fn last_duplicate_wins_and_bom_is_kept() {
        let mut doc = IniDoc::parse("\u{FEFF}[wsl2]\nmemory=2GB\nmemory=3GB\n");
        assert_eq!(read_config(&doc).memory, Some(3 * GIB));
        doc.set("wsl2", "memory", None);
        assert_eq!(doc.render(), "\u{FEFF}[wsl2]\n");
    }

    #[test]
    fn validates_limits() {
        let host = 16 * GIB;
        assert!(validate(&WslGlobalConfig { memory: Some(8 * GIB), processors: Some(8), ..Default::default() }, host, 8).is_ok());
        assert!(validate(&WslGlobalConfig { memory: Some(512 * MIB), ..Default::default() }, host, 8).is_err());
        assert!(validate(&WslGlobalConfig { memory: Some(32 * GIB), ..Default::default() }, host, 8).is_err());
        assert!(validate(&WslGlobalConfig { processors: Some(0), ..Default::default() }, host, 8).is_err());
        assert!(validate(&WslGlobalConfig { processors: Some(9), ..Default::default() }, host, 8).is_err());
        assert!(validate(&WslGlobalConfig { networking_mode: Some("bridged; rm".into()), ..Default::default() }, host, 8).is_err());
        assert!(validate(&WslGlobalConfig { auto_memory_reclaim: Some("gradual".into()), ..Default::default() }, host, 8).is_ok());
        assert!(validate(&WslGlobalConfig { auto_memory_reclaim: Some("fast".into()), ..Default::default() }, host, 8).is_err());
    }
}
