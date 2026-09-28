//! Named-pipe server for the native messaging host (and for second app
//! instances forwarding their command line).

use std::sync::atomic::Ordering;

use crowdm_ipc::{FromBrowser, ToBrowser, PROTOCOL_VERSION};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::captures;
use crate::state::{browser_settings, AppState, Conn};

/// Creates the first pipe instance. Failing here means another CrowDM is
/// already serving this user.
#[cfg(windows)]
pub fn bind_first() -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(crowdm_ipc::pipe_name())
}

#[cfg(windows)]
pub fn start(state: AppState, first: tokio::net::windows::named_pipe::NamedPipeServer) {
    use tokio::net::windows::named_pipe::ServerOptions;
    tauri::async_runtime::spawn(async move {
        let mut server = first;
        loop {
            if server.connect().await.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            } else {
                let connected = server;
                let st = state.clone();
                tauri::async_runtime::spawn(async move { handle(st, connected).await });
            }
            server = loop {
                match ServerOptions::new().reject_remote_clients(true).create(crowdm_ipc::pipe_name()) {
                    Ok(s) => break s,
                    Err(e) => {
                        tracing::error!("pipe create failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            };
        }
    });
}

#[cfg(windows)]
async fn handle(state: AppState, pipe: tokio::net::windows::named_pipe::NamedPipeServer) {
    let id = state.next_conn.fetch_add(1, Ordering::Relaxed);
    let (read, mut write) = tokio::io::split(pipe);
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    state.conns.lock().insert(id, Conn { tx, browser: None, extension_version: None });
    tracing::info!("pipe client {id} connected");

    let writer = tauri::async_runtime::spawn(async move {
        while let Some(mut line) = rx.recv().await {
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<FromBrowser>(&line) {
            Ok(msg) => on_message(&state, id, msg),
            Err(e) => {
                tracing::warn!("bad message from pipe client {id}: {e}");
                state.send(id, &ToBrowser::Error { reply_to: None, code: "badMessage".into(), message: e.to_string() });
            }
        }
    }
    tracing::info!("pipe client {id} disconnected");
    state.conns.lock().remove(&id);
    writer.abort();
    captures::forget_conn(&state, id);
    crate::commands::emit_browser_status(&state);
}

fn on_message(state: &AppState, conn: u64, msg: FromBrowser) {
    match msg {
        FromBrowser::Hello { id, browser, extension_version, .. } => {
            if let Some(c) = state.conns.lock().get_mut(&conn) {
                c.browser = Some(browser.clone());
                c.extension_version = Some(extension_version.clone());
            }
            tracing::info!("extension connected: {browser} {extension_version}");
            state.send(
                conn,
                &ToBrowser::Hello {
                    reply_to: id,
                    app_version: env!("CARGO_PKG_VERSION").into(),
                    protocol: PROTOCOL_VERSION,
                    settings: browser_settings(&state.engine.settings()),
                },
            );
            crate::commands::emit_browser_status(state);
        }
        FromBrowser::Capture { id, browser, capture } => {
            tracing::info!("capture from {browser}: {:?} {}", capture.kind, capture.url);
            captures::on_capture(state, conn, id, browser, capture);
        }
        FromBrowser::CaptureGone { id } => captures::capture_gone(state, conn, &id),
        FromBrowser::Ping { id } => state.send(conn, &ToBrowser::Pong { reply_to: id }),
        FromBrowser::Activate { args } => crate::handle_args(state, &args, true),
    }
}

#[cfg(not(windows))]
pub fn start(_state: AppState) {}
