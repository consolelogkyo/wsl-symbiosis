//! Persistent user settings: `%APPDATA%\com.wslsymbiosis.app\settings.json`.
//!
//! Holds what auto-detection can't guess on every machine: IDEs installed in other drives,
//! the preferred distro and whether the first-run diagnostic was completed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::explorer::Tool;
use crate::wsl::validate_distro_name;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub onboarding_completed: bool,
    /// Tool id (`antigravity`, `vscode`, `cursor`, `windows_terminal`) → launcher path.
    pub custom_tool_paths: HashMap<String, String>,
    pub default_distro: Option<String>,
    /// IDE shown as the labeled "Abrir em" button (ntigravity, scode, cursor);
    /// None = the first one installed.
    pub default_ide: Option<String>,
    /// Shortcut id (`explorer.newTab`) → binding (`Ctrl+T`, mouse: `ctrl,middle`). Only the ones
    /// the user changed; "" = disabled. Defaults and parsing live in `src/lib/shortcuts.ts`.
    pub shortcuts: HashMap<String, String>,
    /// How git pull/fetch reach HTTPS remotes (there is no terminal to type a password).
    pub git_auth: GitAuth,
    /// Setup enabled the WSL Windows features and Windows still has to restart. Owned by the
    /// backend (provisioner.rs): `save_settings` keeps the stored value.
    pub wsl_reboot_pending: bool,
    /// The full-window setup (0.1.3) was finished or skipped once. False for anyone updating from
    /// 0.1.2, so they see it once. Once true it stays true (`save_settings` never clears it).
    pub setup_wizard_seen: bool,
    /// What the window's ✕ / Alt+F4 does (tray.rs).
    #[serde(deserialize_with = "lenient_close_behavior")]
    pub close_behavior: CloseBehavior,
}

/// An unknown value (written by a newer version) means "ask", instead of failing the whole file
/// (which would reset every setting and reopen the setup).
fn lenient_close_behavior<'de, D: serde::Deserializer<'de>>(d: D) -> Result<CloseBehavior, D::Error> {
    Ok(match String::deserialize(d)?.as_str() {
        "tray" => CloseBehavior::Tray,
        "quit" => CloseBehavior::Quit,
        _ => CloseBehavior::Ask,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseBehavior {
    /// The frontend asks (CloseDialog), with "remember my choice".
    #[default]
    Ask,
    /// Hide to the system tray: terminals and dev servers keep running.
    Tray,
    /// Stop projects, close terminals and exit.
    Quit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitAuth {
    /// The repository's own remote; when HTTPS asks for a password, retry the same remote
    /// over SSH (for that command only).
    #[default]
    Auto,
    /// HTTPS remotes always go over SSH.
    Ssh,
    /// Never rewrite the remote.
    Remote,
}

impl GitAuth {
    pub fn as_str(self) -> &'static str {
        match self {
            GitAuth::Auto => "auto",
            GitAuth::Ssh => "ssh",
            GitAuth::Remote => "remote",
        }
    }
}

const MAX_SHORTCUTS: usize = 64;

pub struct SettingsStore {
    /// `None` when the app data dir can't be resolved: settings live in memory only.
    path: Option<PathBuf>,
    data: Mutex<Settings>,
}

impl SettingsStore {
    /// Missing or corrupt files fall back to defaults (the diagnostic then opens again).
    pub fn load(path: Option<PathBuf>) -> Self {
        let data = path.as_deref().and_then(read_file).unwrap_or_default();
        Self { path, data: Mutex::new(data) }
    }

    pub fn get(&self) -> Settings {
        self.data.lock().unwrap().clone()
    }

    pub fn custom_tool_paths(&self) -> HashMap<String, String> {
        self.data.lock().unwrap().custom_tool_paths.clone()
    }

    pub fn git_auth(&self) -> GitAuth {
        self.data.lock().unwrap().git_auth
    }

    pub fn close_behavior(&self) -> CloseBehavior {
        self.data.lock().unwrap().close_behavior
    }

    pub fn set_wsl_reboot_pending(&self, pending: bool) -> Result<(), String> {
        let mut next = self.get();
        if next.wsl_reboot_pending == pending {
            return Ok(());
        }
        next.wsl_reboot_pending = pending;
        self.save(next).map(|_| ())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Validates, writes atomically (temp file + rename) and only then swaps the in-memory copy.
    pub fn save(&self, next: Settings) -> Result<Settings, String> {
        let next = sanitize(next)?;
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("Falha ao criar {}: {e}", dir.display()))?;
            }
            let json = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, json).map_err(|e| format!("Falha ao gravar {}: {e}", tmp.display()))?;
            std::fs::rename(&tmp, path).map_err(|e| format!("Falha ao gravar {}: {e}", path.display()))?;
        }
        *self.data.lock().unwrap() = next.clone();
        Ok(next)
    }
}

fn read_file(path: &Path) -> Option<Settings> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn sanitize(mut s: Settings) -> Result<Settings, String> {
    let mut paths = HashMap::new();
    for (id, raw) in s.custom_tool_paths {
        let tool = Tool::parse(&id)?;
        if !tool.customizable() {
            return Err(format!("{} não aceita caminho personalizado", tool.label()));
        }
        if let Some(path) = normalize_tool_path(&raw).map_err(|e| format!("{}: {e}", tool.label()))? {
            paths.insert(id, path);
        }
    }
    s.custom_tool_paths = paths;
    s.default_distro = match s.default_distro.map(|d| d.trim().to_string()) {
        Some(d) if !d.is_empty() => {
            validate_distro_name(&d)?;
            Some(d)
        }
        _ => None,
    };
    s.default_ide = match s.default_ide.map(|d| d.trim().to_string()) {
        Some(id) if !id.is_empty() => {
            let tool = Tool::parse(&id)?;
            if !tool.is_ide() {
                return Err(format!("{} não é uma IDE", tool.label()));
            }
            Some(id)
        }
        _ => None,
    };
    if s.shortcuts.len() > MAX_SHORTCUTS {
        return Err("Atalhos demais".into());
    }
    for (id, binding) in &s.shortcuts {
        let id_ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
        let binding_ok = binding.len() <= 64 && binding.chars().all(|c| c.is_ascii_graphic());
        if !id_ok || !binding_ok {
            return Err(format!("Atalho inválido: {id}"));
        }
    }
    Ok(s)
}

/// Accepts what users paste from "Copiar como caminho": surrounding quotes, `%VAR%`s.
/// Empty → `Ok(None)` (use auto-detection). Otherwise it must be an existing .exe/.cmd/.bat.
pub fn normalize_tool_path(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let expanded = expand_env_vars(trimmed);
    let path = PathBuf::from(&expanded);
    if !path.is_absolute() {
        return Err(format!("O caminho precisa ser absoluto (ex.: D:\\Apps\\...): {expanded}"));
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !matches!(ext.as_deref(), Some("exe" | "cmd" | "bat")) {
        return Err("Informe um executável .exe, .cmd ou .bat".into());
    }
    if !path.is_file() {
        return Err(format!("Arquivo não encontrado: {expanded}"));
    }
    Ok(Some(expanded))
}

/// Expands `%NAME%` like cmd.exe; unknown variables are kept verbatim.
fn expand_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    #[serde(flatten)]
    pub settings: Settings,
    /// Where the JSON lives, shown in the diagnostic screen.
    pub file: Option<String>,
}

fn snapshot(store: &SettingsStore, settings: Settings) -> SettingsSnapshot {
    SettingsSnapshot { settings, file: store.path().map(|p| p.display().to_string()) }
}

#[tauri::command]
pub fn get_settings(store: State<'_, SettingsStore>) -> SettingsSnapshot {
    snapshot(&store, store.get())
}

#[tauri::command]
pub fn save_settings(store: State<'_, SettingsStore>, mut settings: Settings) -> Result<SettingsSnapshot, String> {
    let current = store.get();
    settings.wsl_reboot_pending = current.wsl_reboot_pending;
    // The Settings dialog doesn't send it: never let a save reopen the setup.
    settings.setup_wizard_seen |= current.setup_wizard_seen;
    let saved = store.save(settings)?;
    Ok(snapshot(&store, saved))
}

/// "Lembrar minha escolha" in the close dialog: changes only this field.
#[tauri::command]
pub fn set_close_behavior(store: State<'_, SettingsStore>, behavior: CloseBehavior) -> Result<SettingsSnapshot, String> {
    let mut next = store.get();
    next.close_behavior = behavior;
    let saved = store.save(next)?;
    Ok(snapshot(&store, saved))
}

/// Live validation for the path fields: the normalized path, or why it can't be used.
#[tauri::command]
pub fn check_tool_path(path: String) -> Result<Option<String>, String> {
    normalize_tool_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wsl-symbiosis-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn expands_env_vars() {
        // SAFETY: tests touching this variable don't run concurrently with readers of it.
        unsafe { std::env::set_var("WSL_SYMBIOSIS_TEST_VAR", r"D:\Tools") };
        assert_eq!(expand_env_vars(r"%WSL_SYMBIOSIS_TEST_VAR%\code.cmd"), r"D:\Tools\code.cmd");
        assert_eq!(expand_env_vars("%NOPE_NOT_SET_123%\\x"), "%NOPE_NOT_SET_123%\\x");
        assert_eq!(expand_env_vars("100% done"), "100% done");
        assert_eq!(expand_env_vars("%%"), "%%");
    }

    #[test]
    fn normalizes_tool_paths() {
        let dir = temp_dir("paths");
        let exe = dir.join("Fake IDE.cmd");
        std::fs::write(&exe, "@echo off").unwrap();
        let txt = dir.join("notes.txt");
        std::fs::write(&txt, "x").unwrap();

        let quoted = format!("  \"{}\"  ", exe.display());
        assert_eq!(normalize_tool_path(&quoted).unwrap().as_deref(), Some(exe.to_str().unwrap()));
        assert_eq!(normalize_tool_path("   ").unwrap(), None);
        assert!(normalize_tool_path(r"bin\code.cmd").unwrap_err().contains("absoluto"));
        assert!(normalize_tool_path(txt.to_str().unwrap()).unwrap_err().contains(".exe"));
        assert!(normalize_tool_path(dir.join("missing.exe").to_str().unwrap()).unwrap_err().contains("não encontrado"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deserializes_partial_and_unknown_fields() {
        let s: Settings = serde_json::from_str(r#"{"onboardingCompleted":true,"future":1}"#).unwrap();
        assert!(s.onboarding_completed);
        assert!(s.custom_tool_paths.is_empty());
        assert_eq!(s.default_distro, None);
        assert_eq!(s.git_auth, GitAuth::Auto, "older settings files get the SSH fallback");
        let s: Settings = serde_json::from_str(r#"{"gitAuth":"ssh"}"#).unwrap();
        assert_eq!(s.git_auth, GitAuth::Ssh);
    }

    #[test]
    fn close_behavior_serialization() {
        // Settings files from 0.1.3 and older have no closeBehavior: ask.
        let s: Settings = serde_json::from_str(r#"{"onboardingCompleted":true}"#).unwrap();
        assert_eq!(s.close_behavior, CloseBehavior::Ask);
        for (json, value) in [("ask", CloseBehavior::Ask), ("tray", CloseBehavior::Tray), ("quit", CloseBehavior::Quit)] {
            let s: Settings = serde_json::from_str(&format!(r#"{{"closeBehavior":"{json}"}}"#)).unwrap();
            assert_eq!(s.close_behavior, value);
            let back = serde_json::to_value(&s).unwrap();
            assert_eq!(back["closeBehavior"], json);
        }
        // A value from a newer version falls back to asking, keeping the rest of the file.
        let s: Settings = serde_json::from_str(r#"{"closeBehavior":"minimize","onboardingCompleted":true}"#).unwrap();
        assert_eq!(s.close_behavior, CloseBehavior::Ask);
        assert!(s.onboarding_completed);
    }

    #[test]
    fn saves_and_reloads() {
        let dir = temp_dir("store");
        let file = dir.join("nested").join("settings.json");
        let exe = dir.join("code.cmd");
        std::fs::write(&exe, "@echo off").unwrap();

        let store = SettingsStore::load(Some(file.clone()));
        assert_eq!(store.get(), Settings::default());

        let mut next = Settings { onboarding_completed: true, ..Default::default() };
        next.custom_tool_paths.insert("vscode".into(), format!("\"{}\"", exe.display()));
        next.custom_tool_paths.insert("cursor".into(), "  ".into());
        next.default_distro = Some(" Ubuntu ".into());
        next.default_ide = Some("vscode".into());
        next.shortcuts.insert("explorer.newTab".into(), "Ctrl+Shift+N".into());
        next.shortcuts.insert("explorer.openInNewTab".into(), "".into());
        let saved = store.save(next).unwrap();
        assert_eq!(saved.shortcuts["explorer.newTab"], "Ctrl+Shift+N");
        assert_eq!(saved.shortcuts["explorer.openInNewTab"], "", "empty = disabled, kept");
        assert_eq!(saved.custom_tool_paths.len(), 1, "empty paths mean auto-detect");
        assert_eq!(saved.custom_tool_paths["vscode"], exe.display().to_string());
        assert_eq!(saved.default_distro.as_deref(), Some("Ubuntu"));
        assert_eq!(saved.default_ide.as_deref(), Some("vscode"));

        let reloaded = SettingsStore::load(Some(file.clone()));
        assert_eq!(reloaded.get(), saved);
        assert!(!file.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_invalid_settings_without_touching_state() {
        let store = SettingsStore::load(None);
        let mut bad = Settings::default();
        bad.custom_tool_paths.insert("explorer".into(), r"C:\Windows\explorer.exe".into());
        assert!(store.save(bad).is_err());

        let mut bad = Settings::default();
        bad.custom_tool_paths.insert("notepad".into(), r"C:\x.exe".into());
        assert!(store.save(bad).is_err());

        let bad = Settings { default_distro: Some("bad name; rm".into()), ..Default::default() };
        assert!(store.save(bad).is_err());

        let bad = Settings { default_ide: Some("explorer".into()), ..Default::default() };
        assert!(store.save(bad).is_err());

        let mut bad = Settings::default();
        bad.shortcuts.insert("explorer.newTab".into(), "Ctrl+\nT".into());
        assert!(store.save(bad).is_err());
        assert_eq!(store.get(), Settings::default());
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = temp_dir("corrupt");
        let file = dir.join("settings.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(SettingsStore::load(Some(file)).get(), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
