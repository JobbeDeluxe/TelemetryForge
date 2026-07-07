mod app_state;
mod config;
mod display_driver;
mod package;
mod remote_auth;
mod remote_server;
mod renderer;
mod scene;
mod sensors;
mod superwidgets;
mod ui;
mod windows_startup;

use app_state::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};

static EXIT_REQUESTED_FROM_TRAY: AtomicBool = AtomicBool::new(false);

fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("TelemetryForge")
        .inner_size(1120.0, 760.0)
        .min_inner_size(900.0, 650.0)
        .resizable(true)
        .build()?;
    Ok(())
}

fn create_main_window_async(app: AppHandle) {
    std::thread::spawn(move || {
        if let Err(error) = create_main_window(&app) {
            tracing::error!(%error, "could not create TelemetryForge window");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let start_minimized = std::env::args().any(|arg| arg == "--minimized");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_log::Builder::new().build())
        .manage(AppState::load())
        .invoke_handler(tauri::generate_handler![
            ui::get_config,
            ui::get_remote_info,
            ui::set_remote_security,
            ui::get_active_screen,
            ui::save_config,
            ui::list_screens,
            ui::set_quick_screen,
            ui::save_screen,
            ui::load_screen,
            ui::new_screen,
            ui::delete_screen,
            ui::export_package,
            ui::share_package,
            ui::import_package,
            ui::get_preview,
            ui::preview_config,
            ui::test_sensors,
            ui::select_background,
            ui::select_background_folder,
            ui::select_gif,
            ui::list_superwidgets,
            ui::import_superwidget,
            ui::list_displays,
            ui::start_rendering,
            ui::stop_rendering,
            ui::render_once,
            ui::test_display,
            ui::set_display_brightness,
            ui::get_status,
            ui::get_autostart,
            ui::set_autostart,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.destroy() {
                    tracing::error!(%error, "could not destroy hidden editor window");
                }
            }
        })
        .setup(move |app| {
            let remote_state = app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(remote_server::manage(remote_state));
            let show = MenuItem::with_id(app, "show", "Open TelemetryForge", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).tooltip("TelemetryForge");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => {
                    create_main_window_async(app.clone());
                }
                "quit" => {
                    EXIT_REQUESTED_FROM_TRAY.store(true, Ordering::Relaxed);
                    app.exit(0);
                }
                _ => {}
            })
            .build(app)?;

            if start_minimized {
                if let Err(error) =
                    ui::start_rendering_from_windows_startup(app.state::<AppState>())
                {
                    tracing::error!(%error, "could not start rendering during Windows startup");
                }
            } else {
                create_main_window(app.handle())?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build TelemetryForge")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !EXIT_REQUESTED_FROM_TRAY.load(Ordering::Relaxed) {
                    api.prevent_exit();
                }
            }
        });
}
