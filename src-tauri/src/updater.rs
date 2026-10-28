// Auto-update support. Checking, downloading and installing are done by tauri-plugin-updater
// from the frontend; this module only covers what the plugin can't know about.

use std::time::Duration;

use tauri::Manager;

use crate::projects::ProjectRunner;
use crate::terminal::TerminalManager;

/// On Windows the updater runs the installer and leaves through `std::process::exit(0)`, which
/// skips `RunEvent::Exit`: terminals and dev servers started by the app would be orphaned (and a
/// running dev server keeps its port). Called by the frontend right before `install()`.
#[tauri::command]
pub async fn prepare_for_update(app: tauri::AppHandle) -> Result<(), String> {
    let handle = app.clone();
    let cleanup = tokio::task::spawn_blocking(move || {
        handle.state::<TerminalManager>().close_all();
        handle.state::<ProjectRunner>().stop_all_blocking();
    });
    // stop_all_blocking already bounds itself; this only guards against a stuck WSL service.
    match tokio::time::timeout(Duration::from_secs(20), cleanup).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("Falha ao encerrar terminais e projetos: {e}")),
        Err(_) => Err("Encerrar terminais e projetos demorou demais.".into()),
    }
}
