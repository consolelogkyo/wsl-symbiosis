mod backup;
mod explorer;
mod git;
mod lines;
mod metrics;
mod packages;
mod projects;
mod provisioner;
mod settings;
mod ssh;
// Public so examples/pty_no_window.rs can exercise the real session code.
pub mod terminal;
mod tray;
mod updater;
mod vhdx;
mod wsl;
mod wslconfig;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // First: a second launch (Start menu, while the app sits in the tray) focuses this one.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Auto-update via GitHub Releases (latest.json signed with .tauri-keys/symbiosis.key).
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let file = app.path().app_data_dir().ok().map(|dir| dir.join("settings.json"));
            app.manage(settings::SettingsStore::load(file));
            tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && tray::on_close_requested(window.app_handle()) {
                    api.prevent_close();
                }
            }
        })
        .manage(wsl::KeepAlive::default())
        .manage(metrics::HostMonitor::default())
        .manage(metrics::GuestSampler::default())
        .manage(terminal::TerminalManager::default())
        .manage(explorer::ToolIconCache::default())
        .manage(projects::ProjectRunner::default())
        .manage(provisioner::ProvisionLock::default())
        .manage(tray::TrayState::default())
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
            explorer::create_env_from_example,
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
            settings::set_close_behavior,
            tray::hide_to_tray,
            tray::quit_application,
            tray::set_tray_context,
            wslconfig::get_wsl_config,
            wslconfig::save_wsl_config,
            vhdx::compact_distro_vhdx,
            ssh::get_git_identity_and_ssh,
            ssh::save_git_identity,
            ssh::generate_ssh_key,
            ssh::import_windows_ssh_keys,
            ssh::fix_ssh_permissions,
            ssh::test_ssh_github,
            backup::export_distro_backup,
            backup::import_distro_backup,
            backup::get_backup_defaults,
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
