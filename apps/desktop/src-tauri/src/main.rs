//! CrowDM desktop application entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod captures;
mod commands;
mod pipe;
mod platform;
mod state;
mod tray;
mod windows;

use crowdm_engine::settings::AfterQueue;
use crowdm_engine::{Engine, EngineEvent, JobState};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use crate::state::AppState;

fn init_logging(data_dir: &std::path::Path) {
    let dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("app.log");
    if std::fs::metadata(&path).map(|m| m.len() > 4 * 1024 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("app.old.log"));
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    let filter = tracing_subscriber::EnvFilter::try_from_env("CROWDM_LOG").unwrap_or_else(|_| "info".into());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::sync::Mutex::new(file)).try_init();
}

/// Acts on command-line arguments (from this launch or a second instance).
pub fn handle_args(state: &AppState, args: &[String], from_other_instance: bool) {
    let urls: Vec<&String> = args.iter().filter(|a| a.starts_with("http://") || a.starts_with("https://")).collect();
    for u in &urls {
        captures::add_manual(state, Some((*u).clone()));
    }
    if urls.is_empty() && from_other_instance && !args.iter().any(|a| a == "--background") {
        windows::show_main(&state.app);
    }
}

pub fn quit(app: &AppHandle) {
    let state = app.state::<AppState>().inner().clone();
    state.quitting.store(true, std::sync::atomic::Ordering::Relaxed);
    // Stop transfers cleanly so they resume on the next start.
    tauri::async_runtime::block_on(state.engine.shutdown());
    // Let browsers keep any download that was still waiting on a dialog.
    for c in state.capture_views() {
        captures::to_browser(&state, c.key);
    }
    app.exit(0);
}

fn fmt_speed(bps: u64) -> String {
    let mb = bps as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB/s")
    } else {
        format!("{:.0} KB/s", bps as f64 / 1024.0)
    }
}

fn forward_engine_events(state: AppState) {
    let mut rx = state.engine.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let app = &state.app;
            match &ev {
                EngineEvent::Progress { items } => {
                    let total: u64 = items.iter().map(|p| p.speed).sum();
                    tray::set_tooltip(app, &format!("CrowDM: {} downloading, {}", items.len(), fmt_speed(total)));
                }
                EngineEvent::JobCompleted { job } => {
                    let s = state.engine.settings();
                    if s.notify_on_complete {
                        let _ = app.notification().builder().title("Download complete").body(&job.job.file_name).show();
                    }
                }
                EngineEvent::JobFailed { job } => {
                    let msg = job.job.error.as_ref().map(|e| e.message.clone()).unwrap_or_default();
                    let _ = app.notification().builder().title(format!("Download failed: {}", job.job.file_name)).body(msg).show();
                }
                EngineEvent::QueueFinished => after_queue(&state),
                _ => {}
            }
            if app.get_webview_window(windows::MAIN).is_some() {
                let _ = app.emit_to(windows::MAIN, "engine", &ev);
            }
            if matches!(ev, EngineEvent::JobUpdated { .. } | EngineEvent::JobRemoved { .. }) {
                let any_active = state.engine.jobs().iter().any(|j| j.job.state == JobState::Downloading);
                if !any_active {
                    tray::set_tooltip(app, "CrowDM");
                }
            }
        }
    });
}

fn after_queue(state: &AppState) {
    let action = state.engine.settings().after_queue;
    let app = &state.app;
    let note = |body: &str| {
        let _ = app.notification().builder().title("Queue finished").body(body).show();
    };
    match action {
        AfterQueue::Nothing => {}
        AfterQueue::Notify => note("All queued downloads are done."),
        AfterQueue::ExitApp => {
            note("All queued downloads are done. CrowDM is closing.");
            // quit() blocks on the engine; leave the async runtime first.
            let app = app.clone();
            std::thread::spawn(move || quit(&app));
        }
        AfterQueue::Sleep => {
            note("All queued downloads are done. The computer is going to sleep.");
            #[cfg(windows)]
            platform::sleep();
        }
        AfterQueue::Shutdown => {
            note("All queued downloads are done. Windows will shut down in 60 seconds.");
            #[cfg(windows)]
            platform::shutdown_soon();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = platform::data_dir();

    // Single instance: the per-user pipe doubles as the instance lock.
    #[cfg(windows)]
    let first_pipe = match tauri::async_runtime::block_on(async { pipe::bind_first() }) {
        Ok(p) => p,
        Err(e) => {
            if platform::forward_to_running_instance(&args) {
                return;
            }
            eprintln!("CrowDM could not start its browser connection ({e}); is it already running?");
            return;
        }
    };

    init_logging(&data_dir);
    tracing::info!("CrowDM {} starting, args {:?}", env!("CARGO_PKG_VERSION"), args);
    let background = args.iter().any(|a| a == "--background");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_jobs,
            commands::get_settings,
            commands::save_settings,
            commands::capture_list,
            commands::capture_new,
            commands::capture_set_url,
            commands::capture_accept,
            commands::capture_to_browser,
            commands::capture_dismiss,
            commands::capture_rendered,
            commands::category_dir,
            commands::show_main_window,
            commands::pause_jobs,
            commands::resume_jobs,
            commands::remove_jobs,
            commands::restart_job,
            commands::update_job,
            commands::set_queued,
            commands::reorder_job,
            commands::open_job_file,
            commands::reveal_job,
            commands::open_folder,
            commands::pause_all,
            commands::resume_all,
            commands::set_queue_running,
            commands::browser_status,
            commands::register_browsers,
            commands::open_extensions_folder,
            commands::open_data_folder,
            commands::memory_usage,
        ])
        .setup(move |app| {
            let engine = tauri::async_runtime::block_on(async { Engine::open(&data_dir.join("crowdm.sqlite")) })
                .map_err(|e| format!("could not open the CrowDM database: {e}"))?;
            let state = AppState::new(app.handle().clone(), engine, data_dir.clone());
            app.manage(state.clone());
            #[cfg(windows)]
            pipe::start(state.clone(), first_pipe);
            #[cfg(windows)]
            if let Err(e) = platform::register_browsers(&data_dir) {
                tracing::warn!("browser registration failed: {e}");
            }
            tray::create(app)?;
            forward_engine_events(state.clone());
            let settings = state.engine.settings();
            if settings.prewarm_dialog {
                windows::ensure_capture(app.handle());
            }
            if !background && !settings.start_minimized {
                windows::show_main(app.handle());
            }
            handle_args(&state, &args, false);
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            let Some(state) = app.try_state::<AppState>() else { return };
            if let WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    windows::CAPTURE => {
                        // Closing the dialog is not a decision to drop the file:
                        // intercepted downloads go back to the browser.
                        api.prevent_close();
                        for c in state.capture_views() {
                            captures::to_browser(&state, c.key);
                        }
                        windows::hide_capture(app);
                    }
                    windows::MAIN => {
                        if !state.engine.settings().close_to_tray && !state.is_quitting() {
                            api.prevent_close();
                            quit(app);
                        }
                        // Otherwise the window is destroyed; the app stays in the tray.
                    }
                    _ => {}
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building CrowDM");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // Closing the last window keeps CrowDM running in the tray.
            let quitting = app.try_state::<AppState>().map(|s| s.is_quitting()).unwrap_or(true);
            if code.is_none() && !quitting {
                api.prevent_exit();
            }
        }
    });
}
