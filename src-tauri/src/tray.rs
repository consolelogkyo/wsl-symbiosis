//! Background mode: system tray icon, what the window's ✕ does, and a live tray menu.
//!
//! The menu shows the WSL state, the VM's footprint on Windows and the projects started from the
//! app, refreshed by a light loop here in Rust (it keeps running while the window is hidden and
//! the webview's own polling is paused). The footprint comes from one kernel process snapshot;
//! `wsl --list` only runs while the window is hidden (when visible, the frontend reports the
//! selected distro through `set_tray_context`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::sync::Notify;

use crate::metrics::VmSampler;
use crate::projects::{ProjectRunner, TaskInfo, TaskKind};
use crate::settings::{CloseBehavior, SettingsStore};
use crate::wsl::{Distro, DistroState};

const TRAY_ID: &str = "main";
const REFRESH: Duration = Duration::from_secs(5);
/// How long the frontend's report of the selected distro is trusted without a `wsl --list`.
const CONTEXT_FRESH: Duration = Duration::from_secs(15);
const MAX_PROJECTS: usize = 8;

/// Set once a clean exit started: the close request that follows must go through.
static QUITTING: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub struct TrayState {
    /// Distro selected in the window, and whether it was running (`set_tray_context`).
    context: Mutex<Option<(Option<String>, bool, Instant)>>,
    visible: AtomicBool,
    /// Wakes the loop right after an action (stop, shutdown, show/hide).
    wake: Notify,
    menu: Mutex<Option<LiveMenu>>,
}

/// The menu currently installed: header lines are updated in place; the menu is rebuilt only
/// when the project list changes.
struct LiveMenu {
    header: MenuItem<Wry>,
    usage: MenuItem<Wry>,
    projects_key: String,
}

/// Shown in the window as a toast (also when the action happened while it was hidden).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrayNotice {
    tone: &'static str,
    title: String,
    detail: Option<String>,
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

fn set_visible(app: &AppHandle, visible: bool) {
    let state = app.state::<TrayState>();
    state.visible.store(visible, Ordering::SeqCst);
    state.wake.notify_one();
    // The webview pauses its polling while hidden (the tray loop takes over).
    let _ = app.emit("app-visibility", visible);
}

pub(crate) fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    set_visible(app, true);
}

pub(crate) fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    set_visible(app, false);
}

/// Stops projects, closes terminals and exits. The window disappears at once; the cleanup
/// (bounded) runs behind it.
pub(crate) fn quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let handle = app.clone();
        let cleanup = tokio::task::spawn_blocking(move || {
            handle.state::<crate::terminal::TerminalManager>().close_all();
            handle.state::<ProjectRunner>().stop_all_blocking();
        });
        // stop_all_blocking bounds itself; this guards against a stuck WSL service.
        let _ = tokio::time::timeout(Duration::from_secs(20), cleanup).await;
        app.exit(0);
    });
}

/// `CloseRequested` on the main window (✕, Alt+F4, taskbar): follows `closeBehavior`.
/// Returns whether the close must be prevented.
pub(crate) fn on_close_requested(app: &AppHandle) -> bool {
    if QUITTING.load(Ordering::SeqCst) {
        return false;
    }
    match app.state::<SettingsStore>().close_behavior() {
        CloseBehavior::Tray => hide_main(app),
        CloseBehavior::Quit => quit(app),
        // The frontend shows CloseDialog and answers with hide_to_tray / quit_application.
        CloseBehavior::Ask => {
            let _ = app.emit("close-requested", ());
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Tray icon and menu
// ---------------------------------------------------------------------------

fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB { format!("{:.1} GB", b / GIB) } else { format!("{:.0} MB", b / 1024.0 / 1024.0) }
}

/// What the menu shows, gathered by the loop.
#[derive(Debug, Clone, PartialEq)]
pub struct TraySnapshot {
    pub distro: Option<(String, bool)>,
    /// VM + helpers memory and VM CPU; `None` while WSL is shut down.
    pub usage: Option<(u64, f32)>,
    pub projects: Vec<TrayProject>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrayProject {
    pub id: String,
    pub name: String,
    pub urls: Vec<String>,
}

impl TraySnapshot {
    pub fn header(&self) -> String {
        match &self.distro {
            Some((name, running)) => {
                format!("WSL Symbiosis · {name} ({})", if *running { "Em execução" } else { "Parada" })
            }
            None => "WSL Symbiosis · nenhuma distro".into(),
        }
    }

    pub fn usage_line(&self) -> String {
        match self.usage {
            Some((memory, cpu)) => format!("Consumo WSL: {} RAM · CPU {cpu:.0}%", format_bytes(memory)),
            None => "WSL desligado (0 MB)".into(),
        }
    }

    /// Windows truncates tray tooltips at 127 characters.
    pub fn tooltip(&self) -> String {
        let mut lines = vec!["WSL Symbiosis".to_string()];
        if let Some((name, running)) = &self.distro {
            lines.push(format!("{name} · {}", if *running { "em execução" } else { "parada" }));
        }
        lines.push(match self.usage {
            Some((memory, cpu)) => format!("{} RAM · CPU {cpu:.0}%", format_bytes(memory)),
            None => "WSL desligado".into(),
        });
        match self.projects.len() {
            0 => {}
            1 => lines.push("1 projeto rodando".into()),
            n => lines.push(format!("{n} projetos rodando")),
        }
        let text = lines.join("\n");
        text.chars().take(127).collect()
    }

    /// Changes only when the project section must be rebuilt.
    fn projects_key(&self) -> String {
        self.projects.iter().map(|p| format!("{}|{}|{}", p.id, p.name, p.urls.join(","))).collect::<Vec<_>>().join(";")
    }
}

/// `● name (:5173)`: the first local URL's port, when the dev server printed one.
pub fn project_label(name: &str, urls: &[String]) -> String {
    let port = urls.first().and_then(|u| {
        let after = u.split("://").nth(1)?;
        let port = after.split('/').next()?.rsplit_once(':')?.1;
        port.chars().all(|c| c.is_ascii_digit()).then(|| port.to_string())
    });
    match port {
        Some(port) => format!("● {name} (:{port})"),
        None => format!("● {name}"),
    }
}

fn build_menu(app: &AppHandle, snap: &TraySnapshot) -> tauri::Result<(Menu<Wry>, LiveMenu)> {
    let header = MenuItem::with_id(app, "info-header", snap.header(), false, None::<&str>)?;
    let usage = MenuItem::with_id(app, "info-usage", snap.usage_line(), false, None::<&str>)?;
    let mut owned: Vec<Box<dyn IsMenuItem<Wry>>> = vec![
        Box::new(header.clone()),
        Box::new(usage.clone()),
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "info-projects", "Servidores em execução", false, None::<&str>)?),
    ];
    if snap.projects.is_empty() {
        owned.push(Box::new(MenuItem::with_id(app, "info-none", "Nenhum projeto em execução", false, None::<&str>)?));
    }
    for project in snap.projects.iter().take(MAX_PROJECTS) {
        let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
        for (i, url) in project.urls.iter().enumerate() {
            let id = format!("project-open:{}:{i}", project.id);
            items.push(Box::new(MenuItem::with_id(app, id, format!("Abrir no navegador ({url})"), true, None::<&str>)?));
        }
        if project.urls.is_empty() {
            items.push(Box::new(MenuItem::with_id(app, format!("info-nourl:{}", project.id), "Sem endereço local ainda", false, None::<&str>)?));
        }
        items.push(Box::new(PredefinedMenuItem::separator(app)?));
        items.push(Box::new(MenuItem::with_id(app, format!("project-stop:{}", project.id), "Parar servidor", true, None::<&str>)?));
        let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
        owned.push(Box::new(Submenu::with_items(app, project_label(&project.name, &project.urls), true, &refs)?));
    }
    let running = snap.distro.as_ref().is_some_and(|(_, r)| *r);
    owned.extend::<[Box<dyn IsMenuItem<Wry>>; 7]>([
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "open", "Abrir WSL Symbiosis", true, None::<&str>)?),
        Box::new(MenuItem::with_id(app, "drop-caches", "Liberar cache de RAM (drop caches)", running, None::<&str>)?),
        Box::new(MenuItem::with_id(app, "shutdown", "Desligar WSL (wsl --shutdown)", snap.usage.is_some(), None::<&str>)?),
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "quit", "Fechar definitivamente", true, None::<&str>)?),
        Box::new(PredefinedMenuItem::separator(app)?),
    ]);
    // A trailing separator renders as empty space on Windows.
    owned.pop();
    let refs: Vec<&dyn IsMenuItem<Wry>> = owned.iter().map(|b| b.as_ref()).collect();
    let menu = Menu::with_items(app, &refs)?;
    Ok((menu, LiveMenu { header, usage, projects_key: snap.projects_key() }))
}

fn apply_snapshot(app: &AppHandle, snap: &TraySnapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let _ = tray.set_tooltip(Some(snap.tooltip()));
    let state = app.state::<TrayState>();
    let mut live = state.menu.lock().unwrap();
    let running = snap.distro.as_ref().is_some_and(|(_, r)| *r);
    // Actions' enabled state also depends on running / VM up: part of the rebuild key.
    let key = format!("{}#{running}#{}", snap.projects_key(), snap.usage.is_some());
    match live.as_ref() {
        Some(menu) if menu.projects_key == key => {
            let _ = menu.header.set_text(snap.header());
            let _ = menu.usage.set_text(snap.usage_line());
        }
        _ => {
            if let Ok((menu, mut handles)) = build_menu(app, snap) {
                handles.projects_key = key;
                let _ = tray.set_menu(Some(menu));
                *live = Some(handles);
            }
        }
    }
}

/// The distro the menu talks about: the window's selection, else the configured default, else
/// WSL's default unless it's a system distro (docker-desktop), else the first real one.
pub fn pick_distro(list: &[Distro], selected: Option<&str>, configured: Option<&str>) -> Option<(String, bool)> {
    let real = |d: &&Distro| !crate::provisioner::is_system_distro(&d.name);
    let found = selected
        .and_then(|s| list.iter().find(|d| d.name == s))
        .or_else(|| configured.and_then(|c| list.iter().find(|d| d.name == c)))
        .or_else(|| list.iter().filter(real).find(|d| d.is_default))
        .or_else(|| list.iter().find(real))
        .or_else(|| list.first())?;
    Some((found.name.clone(), found.state == DistroState::Running))
}

async fn gather(app: &AppHandle, sampler: &mut VmSampler, cpus: usize, last_distro: &mut Option<(String, bool)>) -> TraySnapshot {
    let state = app.state::<TrayState>();
    let visible = state.visible.load(Ordering::SeqCst);
    let context = state.context.lock().unwrap().clone();
    let fresh = context.as_ref().filter(|(_, _, at)| visible && at.elapsed() < CONTEXT_FRESH);
    let distro = match fresh {
        Some((Some(name), running, _)) => Some((name.clone(), *running)),
        _ => match crate::wsl::list_distros().await {
            Ok(list) => {
                let selected = context.as_ref().and_then(|(d, _, _)| d.clone());
                let configured = app.state::<SettingsStore>().get().default_distro;
                pick_distro(&list, selected.as_deref(), configured.as_deref())
            }
            // The WSL service didn't answer: keep the last known state.
            Err(_) => last_distro.clone(),
        },
    };
    *last_distro = distro.clone();

    let (vm, helpers) = sampler.sample(cpus);
    let usage = vm.map(|vm| (vm.memory + helpers.memory, vm.cpu_usage));

    let projects = app
        .state::<ProjectRunner>()
        .list()
        .into_iter()
        .filter(|t: &TaskInfo| t.kind == TaskKind::Run && t.active())
        .map(|t| TrayProject { id: t.id, name: t.name, urls: t.urls })
        .collect();
    TraySnapshot { distro, usage, projects }
}

fn notice(app: &AppHandle, tone: &'static str, title: impl Into<String>, detail: Option<String>) {
    let _ = app.emit("tray-notice", TrayNotice { tone, title: title.into(), detail });
}

fn current_distro(app: &AppHandle) -> Option<(String, bool)> {
    app.state::<TrayState>().context.lock().unwrap().as_ref().and_then(|(d, running, _)| d.clone().map(|d| (d, *running)))
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        "open" => show_main(app),
        "quit" => quit(app),
        "shutdown" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match crate::wsl::shutdown_all(&app).await {
                    Ok(()) => notice(&app, "success", "WSL desligado", Some("wsl --shutdown pela bandeja".into())),
                    Err(e) => notice(&app, "error", "Falha ao desligar o WSL", Some(e)),
                }
                app.state::<TrayState>().wake.notify_one();
            });
        }
        "drop-caches" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // The window's distro, else the same pick as the menu header.
                let target = match current_distro(&app) {
                    Some((name, _)) => Some(name),
                    None => crate::wsl::list_distros().await.ok().and_then(|l| {
                        let configured = app.state::<SettingsStore>().get().default_distro;
                        pick_distro(&l, None, configured.as_deref()).map(|(n, _)| n)
                    }),
                };
                let Some(target) = target else {
                    return notice(&app, "error", "Nenhuma distro para liberar o cache", None);
                };
                match crate::metrics::drop_caches(&target).await {
                    Ok(r) => notice(
                        &app,
                        "success",
                        "Cache de RAM liberado",
                        Some(format!(
                            "{target}: cache {} → {}",
                            format_bytes(r.cache_before),
                            format_bytes(r.cache_after)
                        )),
                    ),
                    Err(e) => notice(&app, "error", "Falha ao liberar o cache", Some(e)),
                }
                app.state::<TrayState>().wake.notify_one();
            });
        }
        other => {
            if let Some(rest) = other.strip_prefix("project-open:") {
                let Some((task, index)) = rest.rsplit_once(':') else { return };
                let url = app
                    .state::<ProjectRunner>()
                    .list()
                    .into_iter()
                    .find(|t| t.id == task)
                    .and_then(|t| index.parse::<usize>().ok().and_then(|i| t.urls.get(i).cloned()));
                if let Some(url) = url {
                    use tauri_plugin_opener::OpenerExt;
                    let _ = app.opener().open_url(url, None::<&str>);
                }
            } else if let Some(task) = other.strip_prefix("project-stop:") {
                let app = app.clone();
                let task = task.to_string();
                tauri::async_runtime::spawn(async move {
                    let sink = crate::projects::app_sink(app.clone());
                    if let Err(e) = app.state::<ProjectRunner>().stop(&sink, &task).await {
                        notice(&app, "error", "Falha ao parar o servidor", Some(e));
                    }
                    app.state::<TrayState>().wake.notify_one();
                });
            }
        }
    }
}

/// Creates the tray icon and starts the refresh loop.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    app.state::<TrayState>().visible.store(true, Ordering::SeqCst);
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("WSL Symbiosis")
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
            | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => show_main(tray.app_handle()),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut sampler = VmSampler::default();
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let mut last_distro = None;
        loop {
            let snap = gather(&handle, &mut sampler, cpus, &mut last_distro).await;
            apply_snapshot(&handle, &snap);
            let state = handle.state::<TrayState>();
            tokio::select! {
                _ = tokio::time::sleep(REFRESH) => {}
                _ = state.wake.notified() => {
                    // Let the action's effects land (task status, VM going down).
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
            }
        }
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn hide_to_tray(app: AppHandle) {
    hide_main(&app);
}

#[tauri::command]
pub fn quit_application(app: AppHandle) {
    quit(&app);
}

/// The window's selected distro (and whether it runs), so the menu matches what the user sees.
#[tauri::command]
pub fn set_tray_context(state: tauri::State<'_, TrayState>, distro: Option<String>, running: bool) {
    let changed = {
        let mut context = state.context.lock().unwrap();
        let changed = context.as_ref().is_none_or(|(d, r, _)| *d != distro || *r != running);
        *context = Some((distro, running, Instant::now()));
        changed
    };
    if changed {
        state.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distro(name: &str, running: bool, default: bool) -> Distro {
        Distro {
            name: name.into(),
            state: if running { DistroState::Running } else { DistroState::Stopped },
            state_label: String::new(),
            version: 2,
            is_default: default,
        }
    }

    #[test]
    fn picks_the_distro_the_user_cares_about() {
        let list = [distro("docker-desktop", true, true), distro("Ubuntu", false, false), distro("Debian", true, false)];
        assert_eq!(pick_distro(&list, Some("Debian"), None), Some(("Debian".into(), true)));
        assert_eq!(pick_distro(&list, Some("Gone"), Some("Debian")), Some(("Debian".into(), true)));
        assert_eq!(pick_distro(&list, None, None), Some(("Ubuntu".into(), false)), "docker-desktop is never the pick");
        assert_eq!(pick_distro(&list[..1], None, None), Some(("docker-desktop".into(), true)));
        assert_eq!(pick_distro(&[], None, None), None);
    }

    #[test]
    fn menu_texts() {
        let snap = TraySnapshot {
            distro: Some(("Ubuntu".into(), true)),
            usage: Some((3 * 1024 * 1024 * 1024 + 300 * 1024 * 1024, 4.4)),
            projects: vec![TrayProject { id: "task-1".into(), name: "web".into(), urls: vec!["http://localhost:5173/".into()] }],
        };
        assert_eq!(snap.header(), "WSL Symbiosis · Ubuntu (Em execução)");
        assert_eq!(snap.usage_line(), "Consumo WSL: 3.3 GB RAM · CPU 4%");
        assert_eq!(snap.tooltip(), "WSL Symbiosis\nUbuntu · em execução\n3.3 GB RAM · CPU 4%\n1 projeto rodando");
        let off = TraySnapshot { distro: Some(("Ubuntu".into(), false)), usage: None, projects: vec![] };
        assert_eq!(off.header(), "WSL Symbiosis · Ubuntu (Parada)");
        assert_eq!(off.usage_line(), "WSL desligado (0 MB)");
        assert!(off.tooltip().chars().count() <= 127);
    }

    #[test]
    fn labels_projects_by_port() {
        assert_eq!(project_label("web", &["http://localhost:5173/".into()]), "● web (:5173)");
        assert_eq!(project_label("api", &["http://localhost:3333/api".into()]), "● api (:3333)");
        assert_eq!(project_label("lib", &[]), "● lib");
    }
}
