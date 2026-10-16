#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The desktop app: an optional tray view of the host (remote-host spec §3.1). pdt-host keeps pdtd running; the tray
//! starts pdt-host when nothing answers, opens the web UI, shows the version and drives pdtd's update API.
mod api;
mod daemon;

use pdt_core::brand::{PRODUCT_NAME, Profile};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

/// pdt-host gives a first start up to 60 s; leave a margin.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(70);
const REFRESH_EVERY: Duration = Duration::from_secs(60);

struct Tray {
    profile: Profile,
    port: u16,
    restart_item: MenuItem<Wry>,
    update_item: MenuItem<Wry>,
    version_item: MenuItem<Wry>,
    /// Set while startup or a restart runs, so they never overlap.
    busy: AtomicBool,
    /// Set once the first `ensure_daemon` succeeded. Opens requested before then are covered by the startup open.
    ready: AtomicBool,
}

impl Tray {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Claims the busy flag; false when startup or a restart is running.
    fn begin(&self) -> bool {
        self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    fn end(&self) {
        self.busy.store(false, Ordering::SeqCst);
    }
}

fn main() {
    let profile = daemon::desktop_profile(std::env::var("PDT_PROFILE").ok().as_deref(), cfg!(debug_assertions));
    let mut context = tauri::generate_context!();
    if profile == Profile::Dev {
        // A separate identifier gives the dev app its own single-instance lock, so it runs beside the stable app.
        let config = context.config_mut();
        config.identifier = format!("{}.dev", config.identifier);
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_ui(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            setup(app, profile)?;
            Ok(())
        })
        .build(context)
        .expect("error while building the desktop app");
    app.run(|_app, event| match event {
        // There are no windows; only Quit (an explicit exit code) ends the app.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_ui(_app),
        _ => {}
    });
}

fn setup(app: &mut tauri::App, profile: Profile) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    let port = profile.port();
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let update_item = MenuItem::with_id(app, "update", "Checking for updates…", false, None::<&str>)?;
    let restart_item = MenuItem::with_id(app, "restart", "Restart daemon", false, None::<&str>)?;
    let version_item = MenuItem::with_id(app, "version", "Version …", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &update_item,
            &restart_item,
            &PredefinedMenuItem::separator(app)?,
            &version_item,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let suffix = if profile == Profile::Dev { " (dev)" } else { "" };
    let icon = app.default_window_icon().cloned().ok_or("the app has no icon")?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip(format!("{PRODUCT_NAME}{suffix} — port {port}"))
        .menu(&menu)
        .show_menu_on_left_click(!cfg!(windows))
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if cfg!(windows)
                && let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                    event
            {
                open_ui(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(Tray {
        profile,
        port,
        restart_item,
        update_item,
        version_item,
        busy: AtomicBool::new(true),
        ready: AtomicBool::new(false),
    });

    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let tray = handle.state::<Tray>();
        daemon::ensure_registered(profile);
        match daemon::ensure_daemon(port, pdt_host::health::probe, || daemon::spawn_host(profile), STARTUP_TIMEOUT) {
            Ok(()) => {
                let _ = tray.restart_item.set_enabled(true);
                tray.ready.store(true, Ordering::SeqCst);
                tray.end();
                open_ui(&handle);
                loop {
                    refresh(&handle);
                    std::thread::sleep(REFRESH_EVERY);
                }
            }
            Err(e) => {
                let h = handle.clone();
                error_dialog(&handle, daemon::with_log_tail(e, profile)).show(move |_| h.exit(1));
            }
        }
    });
    Ok(())
}

/// Updates the Update and Version items from pdtd.
fn refresh(app: &AppHandle) {
    let tray = app.state::<Tray>();
    if let Ok((200, s)) = api::call(tray.port, "GET", "/api/host/update", None) {
        let (text, enabled) = api::update_label(&s);
        let _ = tray.update_item.set_text(text);
        let _ = tray.update_item.set_enabled(enabled);
    }
    if let Ok((200, h)) = api::call(tray.port, "GET", "/api/health", None) {
        let _ = tray.version_item.set_text(format!("Version {}", h["version"].as_str().unwrap_or("?")));
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "open" => open_ui(app),
        "update" => update_clicked(app),
        "restart" => restart_daemon(app),
        // Only the tray exits (decision 16): pdt-host keeps pdtd running.
        "quit" => app.exit(0),
        _ => {}
    }
}

fn error_dialog(app: &AppHandle, message: String) -> tauri_plugin_dialog::MessageDialogBuilder<Wry> {
    app.dialog().message(message).kind(MessageDialogKind::Error).title(PRODUCT_NAME)
}

fn ask(app: &AppHandle, message: &str, ok: &str) -> bool {
    app.dialog()
        .message(message)
        .title(PRODUCT_NAME)
        .buttons(MessageDialogButtons::OkCancelCustom(ok.into(), "Cancel".into()))
        .blocking_show()
}

fn open_ui(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    if !tray.ready.load(Ordering::SeqCst) {
        return;
    }
    let url = tray.url();
    if let Err(e) = app.opener().open_url(url.clone(), None::<&str>) {
        error_dialog(app, format!("Could not open {url}: {e}")).show(|_| {});
    }
}

/// Asks pdtd to stop; pdt-host starts it again (the tray starts pdt-host if none is running). Waits until it answers.
fn restart_daemon(app: &AppHandle) {
    let tray = app.state::<Tray>();
    if !tray.begin() {
        return;
    }
    let _ = tray.restart_item.set_enabled(false);
    let app = app.clone();
    std::thread::spawn(move || {
        let tray = app.state::<Tray>();
        let profile = tray.profile;
        let result = pdt_host::health::request_shutdown(tray.port).and_then(|()| {
            std::thread::sleep(Duration::from_secs(2));
            daemon::ensure_daemon(tray.port, pdt_host::health::probe, || daemon::spawn_host(profile), STARTUP_TIMEOUT)
        });
        if let Err(e) = result {
            error_dialog(&app, format!("The daemon did not come back: {}", daemon::with_log_tail(e, profile)))
                .show(|_| {});
        }
        let _ = tray.restart_item.set_enabled(true);
        tray.end();
    });
}

/// Update: install the available release (asking first, and again if the host is busy), or check for one.
fn update_clicked(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let port = app.state::<Tray>().port;
        let status = api::call(port, "GET", "/api/host/update", None).map(|(_, v)| v).unwrap_or_default();
        if status["state"] != "available" {
            let _ = api::call(port, "POST", "/api/host/update/check", None);
            refresh(&app);
            return;
        }
        let version = status["latest"]["version"].as_str().unwrap_or("?").to_string();
        if !ask(&app, &format!("Update to v{version}? Sessions resume afterwards."), "Update") {
            return;
        }
        let mut force = false;
        loop {
            let body = serde_json::json!({ "force": force }).to_string();
            match api::call(port, "POST", "/api/host/update/install", Some(&body)) {
                Ok((200, _)) => break,
                Ok((409, v)) if v["code"] == "host_busy" && !force => {
                    if !ask(&app, v["message"].as_str().unwrap_or("The host is busy. Update anyway?"), "Update") {
                        return;
                    }
                    force = true;
                }
                Ok((_, v)) => {
                    error_dialog(
                        &app,
                        format!("The update failed: {}", v["message"].as_str().unwrap_or("unknown error")),
                    )
                    .show(|_| {});
                    break;
                }
                Err(e) => {
                    error_dialog(&app, format!("The update failed: {e}")).show(|_| {});
                    break;
                }
            }
        }
        refresh(&app);
    });
}
