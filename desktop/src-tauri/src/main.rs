#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The desktop app: a tray icon that keeps pdtd running and opens the web UI. It has no windows of its own.
mod daemon;
mod platform;
mod update;

use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME, Profile, SLUG};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;
use update::Status;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

struct Tray {
    profile: Profile,
    port: u16,
    version: String,
    restart_item: MenuItem<Wry>,
    update_item: MenuItem<Wry>,
    status: Mutex<Status>,
    /// Set while a startup, restart, update or quit runs, so they never overlap.
    busy: AtomicBool,
    /// Set once the first `ensure_daemon` succeeded. Opens requested before then are covered by the startup open.
    ready: AtomicBool,
}

impl Tray {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_status(&self, status: Status) {
        let (text, enabled) = update::menu_label(&status);
        let _ = self.update_item.set_text(text);
        let _ = self.update_item.set_enabled(enabled);
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }

    /// Claims the busy flag; false when another long action is running.
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
        // macOS activates the running app instead of starting a second one when it is launched again.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_ui(_app),
        _ => {}
    });
}

fn setup(app: &mut tauri::App, profile: Profile) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let port = profile.port();
    let version = app.package_info().version.to_string();
    let initial = if profile == Profile::Dev {
        Status::Disabled("Updates apply to the stable app".into())
    } else {
        let exe = std::env::current_exe().unwrap_or_default();
        let local_app_data = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
        match update::install_check(platform::Os::current(), &exe, local_app_data.as_deref()) {
            Ok(()) => Status::Checking,
            Err(why) => Status::Disabled(why),
        }
    };
    let (update_text, update_enabled) = update::menu_label(&initial);
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let restart_item = MenuItem::with_id(app, "restart", "Restart daemon", false, None::<&str>)?;
    let update_item = MenuItem::with_id(app, "update", update_text, update_enabled, None::<&str>)?;
    let info = MenuItem::with_id(app, "version", format!("Version {version}"), false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &restart_item,
            &update_item,
            &PredefinedMenuItem::separator(app)?,
            &info,
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
        // Windows: left click opens the UI, right click shows the menu. macOS: any click shows the menu.
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
        version,
        restart_item,
        update_item,
        status: Mutex::new(initial.clone()),
        busy: AtomicBool::new(true),
        ready: AtomicBool::new(false),
    });

    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let tray = handle.state::<Tray>();
        match daemon::ensure_daemon(port, daemon::probe, || daemon::spawn_pdtd(profile), STARTUP_TIMEOUT) {
            Ok(()) => {
                let _ = tray.restart_item.set_enabled(true);
                tray.ready.store(true, Ordering::SeqCst);
                tray.end();
                open_ui(&handle);
            }
            Err(e) => {
                let h = handle.clone();
                error_dialog(&handle, daemon::with_log_tail(e, profile)).show(move |_| h.exit(1));
            }
        }
    });
    if initial == Status::Checking {
        let handle = app.handle().clone();
        std::thread::spawn(move || run_check(&handle));
    }
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "open" => open_ui(app),
        "restart" => restart_daemon(app),
        "update" => update_clicked(app),
        "quit" => quit(app),
        _ => {}
    }
}

fn error_dialog(app: &AppHandle, message: String) -> tauri_plugin_dialog::MessageDialogBuilder<Wry> {
    app.dialog().message(message).kind(MessageDialogKind::Error).title(PRODUCT_NAME)
}

/// Opens the web UI in the default browser.
fn open_ui(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    if !tray.ready.load(Ordering::SeqCst) {
        // Still starting: the UI opens once pdtd answers, which covers this request.
        return;
    }
    let url = tray.url();
    if let Err(e) = app.opener().open_url(url.clone(), None::<&str>) {
        error_dialog(app, format!("Could not open {url}: {e}")).show(|_| {});
    }
}

fn restart_daemon(app: &AppHandle) {
    let tray = app.state::<Tray>();
    if !tray.begin() {
        return;
    }
    let _ = tray.restart_item.set_enabled(false);
    let app = app.clone();
    std::thread::spawn(move || {
        let tray = app.state::<Tray>();
        let (port, profile) = (tray.port, tray.profile);
        let exited = || daemon::daemon_exited(profile);
        let result = daemon::stop_daemon(port, daemon::probe, daemon::request_shutdown, exited, STOP_TIMEOUT)
            .and_then(|()| daemon::ensure_daemon(port, daemon::probe, || daemon::spawn_pdtd(profile), STARTUP_TIMEOUT));
        if let Err(e) = result {
            error_dialog(&app, format!("Could not restart {DAEMON_BIN}: {}", daemon::with_log_tail(e, profile)))
                .show(|_| {});
        }
        let _ = tray.restart_item.set_enabled(true);
        tray.end();
    });
}

/// Stops pdtd (sessions resume on the next launch) and exits.
fn quit(app: &AppHandle) {
    let tray = app.state::<Tray>();
    if !tray.begin() {
        // A startup, restart or update is running; quitting under it could orphan or kill a half-started daemon.
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let tray = app.state::<Tray>();
        let profile = tray.profile;
        let exited = || daemon::daemon_exited(profile);
        match daemon::stop_daemon(tray.port, daemon::probe, daemon::request_shutdown, exited, STOP_TIMEOUT) {
            Ok(()) => app.exit(0),
            Err(e) => {
                let quit_anyway = error_dialog(&app, format!("Could not stop {DAEMON_BIN}: {e}\n\nQuit anyway?"))
                    .buttons(MessageDialogButtons::OkCancelCustom("Quit".into(), "Cancel".into()))
                    .blocking_show();
                if quit_anyway {
                    app.exit(0);
                } else {
                    tray.end();
                }
            }
        }
    });
}

/// Runs a check and updates the menu label. Returns the new status.
fn run_check(app: &AppHandle) -> Status {
    let tray = app.state::<Tray>();
    tray.set_status(Status::Checking);
    let status = update::check(&tray.version);
    tray.set_status(status.clone());
    status
}

fn update_clicked(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let tray = app.state::<Tray>();
        let status = match tray.status() {
            Status::Disabled(_) | Status::Checking => return,
            Status::Available(r) => Status::Available(r),
            Status::UpToDate | Status::Failed(_) => run_check(&app),
        };
        match status {
            Status::Available(release) => confirm_and_update(&app, release),
            Status::UpToDate => {
                app.dialog()
                    .message(format!("{PRODUCT_NAME} v{} is the latest version.", tray.version))
                    .title(PRODUCT_NAME)
                    .blocking_show();
            }
            Status::Failed(f) => {
                app.dialog()
                    .message(format!("The update check failed.\n\n{}", f.detail))
                    .kind(MessageDialogKind::Warning)
                    .title(PRODUCT_NAME)
                    .blocking_show();
            }
            Status::Disabled(_) | Status::Checking => {}
        }
    });
}

fn confirm_and_update(app: &AppHandle, release: update::Release) {
    let tray = app.state::<Tray>();
    let go = app
        .dialog()
        .message(format!("Update to v{}? Running sessions will restart.", release.version))
        .title(PRODUCT_NAME)
        .buttons(MessageDialogButtons::OkCancelCustom("Update".into(), "Cancel".into()))
        .blocking_show();
    if !go {
        return;
    }
    if !tray.begin() {
        app.dialog()
            .message("Another action is in progress; try again in a moment.")
            .title(PRODUCT_NAME)
            .blocking_show();
        return;
    }
    let _ = tray.update_item.set_text(format!("Downloading v{}…", release.version));
    let _ = tray.update_item.set_enabled(false);
    let dir = std::env::temp_dir().join(format!("{SLUG}-update")).join(&release.tag);
    let log = daemon::launch_log_path(tray.profile)
        .and_then(|p| p.parent().map(|d| d.join("update.log")))
        .filter(|p| p.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()))
        .unwrap_or_else(|| std::env::temp_dir().join(format!("{SLUG}-update.log")));
    let result =
        update::download(&release, &dir).and_then(|asset| update::launch_helper(&asset, &dir, tray.port, &log));
    match result {
        // The helper waits for this process to exit, then stops pdtd, installs and starts the new app.
        Ok(()) => app.exit(0),
        Err(e) => {
            tray.set_status(Status::Available(release));
            tray.end();
            error_dialog(app, format!("The update failed: {e}")).show(|_| {});
        }
    }
}
