#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The desktop app: an optional tray view of the host (remote-host spec §3.1). pdt-host keeps pdtd running; the tray
//! starts pdt-host when nothing answers, opens the web UI, shows the version and drives pdtd's update API.
mod api;
mod daemon;

use pdt_core::brand::{PRODUCT_NAME, Profile};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

/// pdt-host gives a first start up to 60 s; leave a margin.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(70);
const REFRESH_EVERY: Duration = Duration::from_secs(60);

/// Why the tray shows its placement item at the top of the menu (PDT-51).
#[derive(Clone)]
enum Placement {
    /// This copy runs from a translocated or read-only folder.
    Misplaced(String),
    /// The running pdt-host is another copy's.
    OtherCopy(daemon::OtherCopy),
}

struct Tray {
    profile: Profile,
    menu: Menu<Wry>,
    /// In the menu only while `placement` is set.
    placement_item: MenuItem<Wry>,
    placement: Mutex<Option<Placement>>,
    restart_item: MenuItem<Wry>,
    update_item: MenuItem<Wry>,
    version_item: MenuItem<Wry>,
    /// "Start at login" (PDT-148), in the menu only where the tray can register itself.
    login_item: CheckMenuItem<Wry>,
    login: Option<daemon::TrayLogin>,
    /// Set while startup or a restart runs, so they never overlap.
    busy: AtomicBool,
    /// Set while an Update click is handled, so a second click never posts a second install.
    updating: AtomicBool,
    /// Set once the first `ensure_daemon` succeeded. Opens requested before then are covered by the startup open.
    ready: AtomicBool,
    /// Started by pdt-host after an update: the browser already shows the UI, so startup opens no new window.
    quiet_start: bool,
}

impl Tray {
    /// pdtd's port now: it follows Settings → Network (PDT-231), so it is looked up each time.
    fn port(&self) -> u16 {
        daemon::port(self.profile)
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port())
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
    let port = daemon::port(profile);
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let update_item = MenuItem::with_id(app, "update", "Checking for updates…", false, None::<&str>)?;
    let restart_item = MenuItem::with_id(app, "restart", "Restart daemon", false, None::<&str>)?;
    let version_item = MenuItem::with_id(app, "version", "Version …", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let placement_item = MenuItem::with_id(app, "placement", "", true, None::<&str>)?;
    let login = daemon::tray_login(profile);
    let login_item = CheckMenuItem::with_id(app, "login", "Start at login", login.is_some(), false, None::<&str>)?;
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
    if login.is_some() {
        menu.insert(&login_item, 3)?;
    }
    let suffix = if profile == Profile::Dev { " (dev)" } else { "" };
    // macOS menu bars get the monochrome mark as a template image (the system tints it for light and dark bars, PDT-199);
    // elsewhere the tray shows the app icon.
    let icon = if cfg!(target_os = "macos") {
        tauri::include_image!("icons/tray-template.png")
    } else {
        app.default_window_icon().cloned().ok_or("the app has no icon")?
    };
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
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
        menu,
        placement_item,
        placement: Mutex::new(None),
        restart_item,
        update_item,
        version_item,
        login_item,
        login,
        busy: AtomicBool::new(true),
        updating: AtomicBool::new(false),
        ready: AtomicBool::new(false),
        quiet_start: daemon::quiet_start(std::env::args()),
    });

    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let tray = handle.state::<Tray>();
        // A copy macOS runs from a translocated or read-only folder is never registered at login (PDT-51); it still
        // starts pdt-host, so the app works until it is moved.
        let misplaced = daemon::own_problem();
        if let Some(why) = &misplaced {
            set_placement(&handle, Some(Placement::Misplaced(why.clone())));
            explain_misplaced(&handle, why);
        }
        // The tray starts at login too, unless the user turned that off (PDT-148).
        if let Some(login) = &tray.login {
            let _ = tray.login_item.set_checked(login.ensure());
        }
        // A first macOS launch starts pdt-host through launchd; the tray then only waits for pdtd.
        let registered = daemon::ensure_registered(profile);
        let spawn = || if registered { Ok(()) } else { daemon::spawn_host(profile) };
        match daemon::ensure_daemon(port, pdt_host::health::probe, spawn, STARTUP_TIMEOUT) {
            Ok(()) => {
                let _ = tray.restart_item.set_enabled(true);
                tray.ready.store(true, Ordering::SeqCst);
                tray.end();
                if !tray.quiet_start {
                    open_ui(&handle);
                }
                if misplaced.is_none()
                    && let Some(other) = daemon::other_copy(profile)
                {
                    // Only a copy that cannot stay where it is is worth interrupting for; any other shows in the menu.
                    let ask_now = other.problem.is_some();
                    set_placement(&handle, Some(Placement::OtherCopy(other)));
                    if ask_now {
                        placement_clicked(&handle);
                    }
                }
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
    let (text, enabled) = match api::call(tray.port(), "GET", "/api/host/update", None) {
        Ok((200, s)) => api::update_label(&s),
        _ => ("Update check failed".into(), true),
    };
    let _ = tray.update_item.set_text(text);
    let _ = tray.update_item.set_enabled(enabled);
    if let Ok((200, h)) = api::call(tray.port(), "GET", "/api/health", None) {
        let _ = tray.version_item.set_text(format!("Version {}", h["version"].as_str().unwrap_or("?")));
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "open" => open_ui(app),
        "update" => update_clicked(app),
        "restart" => restart_daemon(app),
        "placement" => placement_clicked(app),
        "login" => login_clicked(app),
        // Only the tray exits (decision 16): pdt-host keeps pdtd running.
        "quit" => app.exit(0),
        _ => {}
    }
}

/// "Start at login": the item has already toggled itself; make the registration follow, or put it back.
fn login_clicked(app: &AppHandle) {
    let tray = app.state::<Tray>();
    let Some(login) = &tray.login else { return };
    let on = tray.login_item.is_checked().unwrap_or(false);
    if let Err(e) = login.set(on) {
        let _ = tray.login_item.set_checked(!on);
        error_dialog(app, format!("Could not change starting at login: {e}")).show(|_| {});
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

/// Shows (or, with `None`, removes) the placement item at the top of the menu.
fn set_placement(app: &AppHandle, placement: Option<Placement>) {
    let tray = app.state::<Tray>();
    let mut current = tray.placement.lock().unwrap_or_else(|e| e.into_inner());
    let label = match &placement {
        Some(Placement::Misplaced(_)) => Some("Move the app to Applications to get updates…".to_string()),
        Some(Placement::OtherCopy(other)) => {
            Some(format!("Another copy runs ({}): switch to this one…", bundle_name(&other.bundle)))
        }
        None => None,
    };
    match (current.is_some(), label) {
        (was, Some(label)) => {
            let _ = tray.placement_item.set_text(label);
            if !was {
                let _ = tray.menu.prepend(&tray.placement_item);
            }
        }
        (true, None) => {
            let _ = tray.menu.remove(&tray.placement_item);
        }
        (false, None) => {}
    }
    *current = placement;
}

fn bundle_name(bundle: &std::path::Path) -> String {
    bundle.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| bundle.display().to_string())
}

fn explain_misplaced(app: &AppHandle, why: &str) {
    app.dialog()
        .message(format!(
            "{PRODUCT_NAME} can't start at login or install updates from where it is: {why}.\n\nIt keeps working until then."
        ))
        .kind(MessageDialogKind::Warning)
        .title(PRODUCT_NAME)
        .show(|_| {});
}

fn placement_clicked(app: &AppHandle) {
    let placement = app.state::<Tray>().placement.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match placement {
        Some(Placement::Misplaced(why)) => explain_misplaced(app, &why),
        Some(Placement::OtherCopy(other)) => {
            let app = app.clone();
            std::thread::spawn(move || switch_flow(&app, &other));
        }
        None => {}
    }
}

/// "Switch to this copy": this copy's pdt-host takes over from the other's and registers at login, then pdtd restarts
/// from this copy (PDT-51).
fn switch_flow(app: &AppHandle, other: &daemon::OtherCopy) {
    let here = std::env::current_exe()
        .ok()
        .and_then(|e| pdt_host::install::bundle_of(e.parent()?))
        .map(|b| b.display().to_string())
        .unwrap_or_else(|| "this copy".into());
    let problem = other.problem.as_deref().map(|p| format!(" It can't stay there: {p}.")).unwrap_or_default();
    let question = format!(
        "{PRODUCT_NAME} is running from another copy: {}.{problem}\n\nSwitch to this copy ({here})? It will start at login and install updates. The daemon restarts; sessions resume afterwards.",
        other.bundle.display()
    );
    if !ask(app, &question, "Switch") {
        return;
    }
    let tray = app.state::<Tray>();
    if !tray.begin() {
        return;
    }
    let _ = tray.restart_item.set_enabled(false);
    let profile = tray.profile;
    let result = daemon::switch_to_this_copy(profile).and_then(|()| restart_and_wait(tray.port(), profile));
    match result {
        Ok(()) => set_placement(app, None),
        Err(e) => error_dialog(app, format!("Could not switch to this copy: {}", daemon::with_log_tail(e, profile)))
            .show(|_| {}),
    }
    let _ = tray.restart_item.set_enabled(true);
    tray.end();
    refresh(app);
}

/// Asks pdtd to stop and waits until pdt-host has started it again (starting pdt-host if none runs).
fn restart_and_wait(port: u16, profile: Profile) -> Result<(), String> {
    pdt_host::health::request_shutdown(port).and_then(|()| {
        std::thread::sleep(Duration::from_secs(2));
        // The new pdtd may take another port (Settings → Network).
        let port = daemon::port(profile);
        daemon::ensure_daemon(port, pdt_host::health::probe, || daemon::spawn_host(profile), STARTUP_TIMEOUT)
    })
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
        if let Err(e) = restart_and_wait(tray.port(), profile) {
            error_dialog(&app, format!("The daemon did not come back: {}", daemon::with_log_tail(e, profile)))
                .show(|_| {});
        }
        let _ = tray.restart_item.set_enabled(true);
        tray.end();
    });
}

/// Update: install the available release (asking first, and again if the host is busy), or check for one.
fn update_clicked(app: &AppHandle) {
    let tray = app.state::<Tray>();
    if tray.updating.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        update_flow(&app);
        app.state::<Tray>().updating.store(false, Ordering::SeqCst);
    });
}

fn update_flow(app: &AppHandle) {
    let port = app.state::<Tray>().port();
    let status = api::call(port, "GET", "/api/host/update", None).map(|(_, v)| v).unwrap_or_default();
    if status["state"] != "available" {
        let _ = api::call(port, "POST", "/api/host/update/check", None);
        refresh(app);
        return;
    }
    let version = status["latest"]["version"].as_str().unwrap_or("?").to_string();
    if !ask(app, &format!("Update to v{version}? Sessions resume afterwards."), "Update") {
        return;
    }
    let mut force = false;
    loop {
        let body = serde_json::json!({ "force": force }).to_string();
        match api::call(port, "POST", "/api/host/update/install", Some(&body)) {
            Ok((200, _)) => break,
            Ok((409, v)) if v["code"] == "host_busy" && !force => {
                if !ask(app, v["message"].as_str().unwrap_or("The host is busy. Update anyway?"), "Update") {
                    return;
                }
                force = true;
            }
            Ok((_, v)) => {
                let message = v["message"].as_str().unwrap_or("unknown error");
                error_dialog(app, format!("The update failed: {message}")).show(|_| {});
                break;
            }
            Err(e) => {
                error_dialog(app, format!("The update failed: {e}")).show(|_| {});
                break;
            }
        }
    }
    refresh(app);
}
