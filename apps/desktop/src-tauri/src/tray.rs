//! Notification-area icon and menu.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, Manager};

use crate::state::AppState;
use crate::{captures, windows};

pub fn create(app: &App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open CrowDM", true, None::<&str>)?;
    let add = MenuItem::with_id(app, "add", "Add download...", true, None::<&str>)?;
    let pause_all = MenuItem::with_id(app, "pause_all", "Pause all", true, None::<&str>)?;
    let resume_all = MenuItem::with_id(app, "resume_all", "Resume all", true, None::<&str>)?;
    let queue = MenuItem::with_id(app, "queue", "Start or stop queue", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit CrowDM", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &add, &sep1, &pause_all, &resume_all, &queue, &sep2, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("CrowDM")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let state = app.state::<AppState>().inner().clone();
            match event.id().as_ref() {
                "open" => windows::show_main(app),
                "add" => {
                    captures::add_manual(&state, None);
                }
                "pause_all" => state.engine.pause_all(),
                "resume_all" => state.engine.resume_all(),
                "queue" => {
                    if state.engine.queue_running() {
                        state.engine.stop_queue()
                    } else {
                        state.engine.start_queue()
                    }
                }
                "quit" => crate::quit(app),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                windows::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Tooltip with the current transfer summary.
pub fn set_tooltip(app: &tauri::AppHandle, text: &str) {
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(text));
    }
}
