#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod daemon;

use pdt_core::brand::{PRODUCT_NAME, Profile};
use std::time::Duration;

fn main() {
    let profile = Profile::from_env_value(std::env::var("PDT_PROFILE").ok().as_deref());
    let port = profile.port();
    tauri::Builder::default()
        .setup(move |app| {
            daemon::ensure_daemon(port, || daemon::spawn_pdtd(profile), Duration::from_secs(10))?;
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
