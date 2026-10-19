//! Always identify native Windows toasts as CrowDM, including portable builds.
use crate::state::AppState;
pub const APP_ID: &str = "io.github.syst4n.crowdm";
pub fn show(state: &AppState, title: impl Into<String>, body: impl Into<String>) {
    if std::env::var_os("CROWDM_TEST_INSTANCE").is_some() {
        return;
    }
    let title = title.into();
    let body = body.into();
    #[cfg(windows)]
    {
        let icon = state.data_dir.join("notification-icon.png");
        tauri::async_runtime::spawn_blocking(move || {
            if tauri_winrt_notification::Toast::new(APP_ID)
                .title(&title)
                .text1(&body)
                .icon(&icon, tauri_winrt_notification::IconCrop::Square, "CrowDM")
                .show()
                .is_err()
            {
                tracing::warn!("Windows could not display the CrowDM notification");
            }
        });
    }
    #[cfg(not(windows))]
    {
        use tauri_plugin_notification::NotificationExt;
        let _ = state.app.notification().builder().title(title).body(body).show();
    }
}
