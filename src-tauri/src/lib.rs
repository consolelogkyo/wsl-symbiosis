mod explorer;
mod git;
mod metrics;
mod packages;
mod projects;
mod provisioner;
mod settings;
// Public so examples/pty_no_window.rs can exercise the real session code.
pub mod terminal;
mod updater;
mod wsl;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Auto-update via GitHub Releases (latest.json signed with .tauri-keys/symbiosis.key).
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let file = app.path().app_data_dir().ok().map(|dir| dir.join("settings.json"));
            app.manage(settings::SettingsStore::load(file));
            Ok(())
        })
        .manage(wsl::KeepAlive::default())
        .manage(metrics::HostMonitor::default())
        .manage(metrics::GuestSampler::default())
        .manage(terminal::TerminalManager::default())
        .manage(explorer::ToolIconCache::default())
        .manage(projects::ProjectRunner::default())
        .manage(provisioner::ProvisionLock::default())
        .invoke_handler(tauri::generate_handler![
            wsl::list_distros,
            wsl::get_wsl_info,
            wsl::start_distro,
            wsl::shutdown_wsl,
            metrics::get_wsl_resource_metrics,
            metrics::free_wsl_cached_memory,
            metrics::kill_linux_process,
            explorer::get_distro_home,
            explorer::list_directory,
            explorer::create_directory,
            explorer::create_file,
            explorer::rename_entry,
            explorer::delete_entry,
            explorer::read_text_file,
            explorer::write_text_file,
            explorer::open_in_tool,
            explorer::detect_tools,
            explorer::get_tool_icons,
            git::git_branches,
            git::git_switch,
            git::git_fetch,
            git::git_pull,
            git::git_clone,
            packages::scan_environment,
            packages::list_listening_ports,
            packages::install_npm_global,
            packages::uninstall_npm_global,
            packages::switch_nvm_default,
            terminal::create_terminal_session,
            terminal::write_terminal,
            terminal::resize_terminal,
            terminal::close_terminal_session,
            settings::get_settings,
            settings::save_settings,
            settings::check_tool_path,
            projects::inspect_project,
            projects::start_project,
            projects::stop_project,
            projects::list_project_tasks,
            projects::get_project_log,
            projects::dismiss_project_task,
            projects::stop_process_tree,
            updater::prepare_for_update,
            provisioner::inspect_setup_state,
            provisioner::enable_wsl_engine,
            provisioner::install_ubuntu_distro,
            provisioner::list_online_distros,
            provisioner::should_open_setup,
            provisioner::run_provisioning_tasks,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Don't leave wsl.exe shells behind when the window closes.
            if let tauri::RunEvent::Exit = event {
                app.state::<terminal::TerminalManager>().close_all();
                // Dev servers started from the Explorer must not outlive the app.
                app.state::<projects::ProjectRunner>().stop_all_blocking();
            }
        });
}
