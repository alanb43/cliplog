mod images;
mod store;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use store::{Clip, ClipKind, History, ImageInfo, Settings};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct AppState {
    data_dir: PathBuf,
    history: Mutex<History>,
    settings: Mutex<Settings>,
    /// Last clipboard text we've seen, so the watcher only records changes
    /// (and doesn't re-record text Cliplog itself put on the clipboard).
    last_text: Mutex<Option<String>>,
    /// Clipboard change number right after Cliplog itself wrote to it, so the
    /// watcher can skip it.
    ignore_seq: Mutex<Option<u32>>,
}

const OVERLAY: &str = "overlay";
const SETTINGS: &str = "settings";
const POLL_INTERVAL: Duration = Duration::from_millis(400);

// ---------- windows ----------

fn show_overlay(app: &AppHandle) {
    let Some(win) = app.get_webview_window(OVERLAY) else { return };
    #[cfg(target_os = "macos")]
    let _ = app.show();

    // Open on whichever monitor the mouse is on, a third of the way down.
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten());
    match (monitor, win.outer_size()) {
        (Some(m), Ok(size)) => {
            let (mp, ms) = (m.position(), m.size());
            let x = mp.x + (ms.width as i32 - size.width as i32) / 2;
            let y = mp.y + (ms.height as i32 - size.height as i32) / 3;
            let _ = win.set_position(PhysicalPosition::new(x, y));
        }
        _ => {
            let _ = win.center();
        }
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = win.emit("overlay-shown", ());
}

fn hide_overlay_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(OVERLAY) {
        let _ = win.hide();
    }
}

/// Hides the overlay and gives focus back to the app the user was in, so they
/// can paste straight away.
fn dismiss_overlay(app: &AppHandle) {
    hide_overlay_window(app);
    #[cfg(target_os = "macos")]
    {
        let settings_open = app
            .get_webview_window(SETTINGS)
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);
        if !settings_open {
            let _ = app.hide();
        }
    }
}

fn toggle_overlay(app: &AppHandle) {
    let open = app
        .get_webview_window(OVERLAY)
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false);
    if open {
        dismiss_overlay(app);
    } else {
        show_overlay(app);
    }
}

fn show_settings(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(SETTINGS) {
        #[cfg(target_os = "macos")]
        let _ = app.show();
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

// ---------- clipboard watcher ----------

/// Cheap "has the clipboard changed?" check so we don't open the clipboard
/// on every poll (opening it can briefly block other apps on Windows).
#[cfg(target_os = "windows")]
fn clipboard_sequence() -> Option<u32> {
    #[link(name = "user32")]
    extern "system" {
        fn GetClipboardSequenceNumber() -> u32;
    }
    Some(unsafe { GetClipboardSequenceNumber() })
}

#[cfg(target_os = "macos")]
fn clipboard_sequence() -> Option<u32> {
    #[allow(unused_unsafe)]
    let count = unsafe { objc2_app_kit::NSPasteboard::generalPasteboard().changeCount() };
    Some(count as u32)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn clipboard_sequence() -> Option<u32> {
    None
}

fn start_clipboard_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let mut clipboard = loop {
            match arboard::Clipboard::new() {
                Ok(c) => break c,
                Err(_) => std::thread::sleep(Duration::from_secs(2)),
            }
        };
        let mut last_seq = None;
        loop {
            std::thread::sleep(POLL_INTERVAL);
            let seq = clipboard_sequence();
            if seq.is_some() && seq == last_seq {
                continue;
            }
            last_seq = seq;
            let state = app.state::<AppState>();
            if seq.is_some() && seq == *state.ignore_seq.lock().unwrap() {
                continue;
            }

            let changed = if let Ok(text) = clipboard.get_text() {
                record_text(&state, text)
            } else if seq.is_some() {
                // Images are only checked when we know the clipboard changed;
                // reading one is too expensive to do on every poll.
                match clipboard.get_image() {
                    Ok(img) => record_image(&state, img),
                    Err(_) => false,
                }
            } else {
                false
            };
            if changed {
                let _ = app.emit("history-changed", ());
            }
        }
    });
}

fn record_text(state: &AppState, text: String) -> bool {
    {
        let mut last = state.last_text.lock().unwrap();
        if last.as_deref() == Some(text.as_str()) {
            return false;
        }
        *last = Some(text.clone());
    }
    let mut history = state.history.lock().unwrap();
    let changed = history.push(text);
    if changed {
        history.save();
    }
    changed
}

fn record_image(state: &AppState, img: arboard::ImageData) -> bool {
    *state.last_text.lock().unwrap() = None;
    if img.bytes.len() > images::MAX_IMAGE_BYTES {
        return false;
    }
    let (width, height) = (img.width as u32, img.height as u32);
    let hash = images::hash(width, height, &img.bytes);

    let id = {
        let mut history = state.history.lock().unwrap();
        if let Some(id) = history.find_image(hash) {
            // Already have it: just move it to the top.
            if history.clips.first().map(|c| c.id) == Some(id) {
                return false;
            }
            history.promote(id);
            history.save();
            return true;
        }
        history.next_id()
    };

    // Encode outside the lock so the overlay stays responsive.
    let dir = state.history.lock().unwrap().images_dir.clone();
    if let Err(e) = images::save(&dir, id, width, height, img.bytes.into_owned()) {
        eprintln!("could not save image: {e}");
        return false;
    }
    let mut history = state.history.lock().unwrap();
    history.push_image(id, ImageInfo { width, height, hash });
    history.save();
    true
}

// ---------- shortcut & autostart ----------

fn apply_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(shortcut).map_err(|e| e.to_string())
}

fn apply_autostart(app: &AppHandle, enabled: bool) {
    let autolaunch = app.autolaunch();
    let currently = autolaunch.is_enabled().unwrap_or(false);
    if enabled && !currently {
        let _ = autolaunch.enable();
    } else if !enabled && currently {
        let _ = autolaunch.disable();
    }
}

// ---------- commands (called from the web UI) ----------

#[tauri::command]
fn get_history(state: tauri::State<AppState>) -> Vec<Clip> {
    state.history.lock().unwrap().clips.clone()
}

#[tauri::command]
fn copy_clip(app: AppHandle, state: tauri::State<AppState>, id: u64) -> Result<(), String> {
    let (clip, images_dir) = {
        let mut history = state.history.lock().unwrap();
        let clip = history.promote(id).ok_or("Clip not found")?;
        history.save();
        (clip, history.images_dir.clone())
    };
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    match clip.kind {
        ClipKind::Text => {
            *state.last_text.lock().unwrap() = Some(clip.text.clone());
            clipboard.set_text(clip.text).map_err(|e| e.to_string())?;
        }
        ClipKind::Image => {
            let (width, height, bytes) = images::load(&images_dir, id)?;
            clipboard
                .set_image(arboard::ImageData {
                    width: width as usize,
                    height: height as usize,
                    bytes: bytes.into(),
                })
                .map_err(|e| e.to_string())?;
        }
    }
    *state.ignore_seq.lock().unwrap() = clipboard_sequence();
    let _ = app.emit("history-changed", ());
    dismiss_overlay(&app);
    Ok(())
}

#[tauri::command]
fn get_thumbnail(state: tauri::State<AppState>, id: u64) -> Option<String> {
    let dir = state.history.lock().unwrap().images_dir.clone();
    images::thumbnail_data_url(&dir, id)
}

#[tauri::command]
fn delete_clip(app: AppHandle, state: tauri::State<AppState>, id: u64) {
    let mut history = state.history.lock().unwrap();
    history.remove(id);
    history.save();
    drop(history);
    let _ = app.emit("history-changed", ());
}

#[tauri::command]
fn clear_history(app: AppHandle, state: tauri::State<AppState>) {
    let mut history = state.history.lock().unwrap();
    history.clear();
    history.save();
    drop(history);
    let _ = app.emit("history-changed", ());
}

#[tauri::command]
fn hide_overlay(app: AppHandle) {
    dismiss_overlay(&app);
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let settings = settings.clamped();
    let previous = state.settings.lock().unwrap().clone();
    if settings.shortcut != previous.shortcut {
        if let Err(e) = apply_shortcut(&app, &settings.shortcut) {
            let _ = apply_shortcut(&app, &previous.shortcut);
            return Err(format!("Couldn't use that shortcut ({e}). Try another combination."));
        }
    }
    apply_autostart(&app, settings.launch_at_login);
    settings.save(&state.data_dir);
    *state.settings.lock().unwrap() = settings.clone();
    let _ = app.emit("settings-changed", settings.clone());
    Ok(settings)
}

#[tauri::command]
fn pause_shortcut(app: AppHandle) {
    // While the user records a new shortcut, the old one shouldn't fire.
    let _ = app.global_shortcut().unregister_all();
}

#[tauri::command]
fn resume_shortcut(app: AppHandle, state: tauri::State<AppState>) {
    let shortcut = state.settings.lock().unwrap().shortcut.clone();
    let _ = apply_shortcut(&app, &shortcut);
}

// ---------- app ----------

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Opening Cliplog again while it's running shows the history.
            show_overlay(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_overlay(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_history,
            copy_clip,
            get_thumbnail,
            delete_clip,
            clear_history,
            hide_overlay,
            get_settings,
            save_settings,
            pause_shortcut,
            resume_shortcut,
        ])
        .setup(|app| {
            // Menu-bar app on macOS: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let (settings, first_run) = Settings::load(&data_dir);
            let history = History::load(&data_dir);
            let handle = app.handle().clone();

            if first_run {
                settings.save(&data_dir);
                apply_autostart(&handle, settings.launch_at_login);
            }
            if let Err(e) = apply_shortcut(&handle, &settings.shortcut) {
                eprintln!("could not register shortcut {}: {e}", settings.shortcut);
            }

            app.manage(AppState {
                data_dir,
                history: Mutex::new(history),
                settings: Mutex::new(settings),
                last_text: Mutex::new(None),
                ignore_seq: Mutex::new(None),
            });

            // Tray / menu-bar icon.
            let show = MenuItem::with_id(app, "show", "Show Clipboard History", true, None::<&str>)?;
            let prefs = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Cliplog", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show, &prefs, &sep, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/tray.png"))
                .icon_as_template(true)
                .tooltip("Cliplog")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_overlay(app),
                    "settings" => show_settings(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            start_clipboard_watcher(handle.clone());

            let launched_at_login = std::env::args().any(|a| a == "--background");
            if first_run {
                show_settings(&handle);
            } else if !launched_at_login {
                show_overlay(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Clicking anywhere else closes the overlay.
            WindowEvent::Focused(false) if window.label() == OVERLAY => {
                let _ = window.hide();
            }
            // Closing a window just hides it; Cliplog keeps running in the tray.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building Cliplog");

    app.run(|_app, _event| {
        // Re-opening the app from Finder/Dock on macOS shows the history.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = _event {
            show_overlay(_app);
        }
    });
}
