#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod daemon;

use pdt_core::brand::{PRODUCT_NAME, Profile};
use std::time::Duration;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

fn main() {
    let profile = Profile::from_env_value(std::env::var("PDT_PROFILE").ok().as_deref());
    let port = profile.port();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            if let Err(e) =
                daemon::ensure_daemon(port, daemon::probe, || daemon::spawn_pdtd(profile), Duration::from_secs(15))
            {
                // Non-blocking: blocking dialogs must not run on the main thread, where setup runs.
                app.dialog()
                    .message(daemon::with_log_tail(e, profile))
                    .kind(MessageDialogKind::Error)
                    .title(PRODUCT_NAME)
                    .show(|_| std::process::exit(1));
                return Ok(());
            }
            let url = format!("http://127.0.0.1:{port}/").parse()?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title(PRODUCT_NAME)
                .inner_size(1440.0, 900.0)
                .min_inner_size(360.0, 480.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the desktop shell");
}
