mod clipboard;
mod files;
mod images;
mod store;

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use clipboard::Snapshot;
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
    /// Last clipboard text we've seen, on platforms without raw clipboard
    /// access, so the watcher only records changes.
    last_text: Mutex<Option<String>>,
    /// Clipboard change number right after Cliplog itself wrote to it, so the
    /// watcher can skip it.
    ignore_seq: Mutex<Option<u32>>,
    /// Where stored files are written out when pasted (see files.rs).
    paste_dir: PathBuf,
}

const OVERLAY: &str = "overlay";
const SETTINGS: &str = "settings";
const POLL_INTERVAL: Duration = Duration::from_millis(400);
const SETTLE_DELAY: Duration = Duration::from_millis(150);

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

fn start_clipboard_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        // arboard is used for decoding images (thumbnails) and as a text-only
        // fallback on platforms without raw clipboard access.
        let mut fallback = loop {
            match arboard::Clipboard::new() {
                Ok(c) => break c,
                Err(_) => std::thread::sleep(Duration::from_secs(2)),
            }
        };
        let mut last_seq = None;
        loop {
            std::thread::sleep(POLL_INTERVAL);
            // Cheap "has the clipboard changed?" check, so the clipboard is
            // only read after a copy.
            let seq = clipboard::change_count();
            if seq.is_some() && seq == last_seq {
                continue;
            }
            if seq.is_some() {
                // Apps clear the clipboard and then write to it; the change
                // counter only moves on the clear. Give the writer a moment so
                // we don't read a half-written clipboard.
                std::thread::sleep(SETTLE_DELAY);
                if clipboard::change_count() != seq {
                    continue; // changed again; handle it on the next poll
                }
            }
            last_seq = seq;
            let state = app.state::<AppState>();
            if seq.is_some() && seq == *state.ignore_seq.lock().unwrap() {
                continue; // Cliplog itself just wrote this.
            }

            let changed = if clipboard::SUPPORTED {
                match clipboard::read() {
                    Ok(Some(snapshot)) => record_snapshot(&state, &mut fallback, snapshot),
                    Ok(None) => false, // empty, or marked as secret
                    Err(e) => {
                        eprintln!("could not read clipboard: {e}");
                        false
                    }
                }
            } else {
                match fallback.get_text() {
                    Ok(text) => record_text(&state, text),
                    Err(_) => false,
                }
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
    let changed = history.push_text(text);
    if changed {
        history.save();
    }
    changed
}

fn record_snapshot(state: &AppState, decoder: &mut arboard::Clipboard, mut snapshot: Snapshot) -> bool {
    let (id, max_bytes, memory_limit, images_dir, blob_path) = {
        let history = state.history.lock().unwrap();
        let id = history.next_id();
        (id, history.max_bytes(), history.memory_limit(), history.images_dir(), history.blob_path(id))
    };
    // Something bigger than the whole storage cap: keep just its text, if any.
    if snapshot.total_bytes() as u64 > max_bytes {
        snapshot.retain_text();
        if snapshot.items.is_empty() {
            return false;
        }
    }

    let files = snapshot.files();
    let text = snapshot.text().unwrap_or_default();
    let (kind, preview) = if !files.is_empty() {
        (ClipKind::Files, files.join("\n"))
    } else if !text.trim().is_empty() {
        (ClipKind::Text, text)
    } else if snapshot.has_image() {
        (ClipKind::Image, String::new())
    } else {
        (ClipKind::Other, String::new())
    };
    // Small copied files are stored in the clip like any other data; bigger
    // ones stay links to the original.
    let stored_files = match kind {
        ClipKind::Files => files::embed(&mut snapshot, &files, memory_limit as u64),
        _ => 0,
    };
    let hash = snapshot.hash();
    if state.history.lock().unwrap().is_same_as_newest(kind, &preview, hash) {
        return false;
    }

    let mut clip = Clip::new(id, kind, preview);
    clip.hash = hash;
    clip.stored_files = stored_files;
    if kind == ClipKind::Image {
        if let Ok(img) = decoder.get_image() {
            let (width, height) = (img.width as u32, img.height as u32);
            if img.bytes.len() <= images::MAX_IMAGE_BYTES
                && images::save_thumbnail(&images_dir, id, width, height, img.bytes.into_owned()).is_ok()
            {
                clip.image = Some(ImageInfo { width, height });
            }
        }
    }

    // Plain text needs nothing but the text. Clips up to the in-memory limit
    // keep their raw data in history.json; bigger ones get their own file.
    if !(kind == ClipKind::Text && snapshot.is_plain_text_only()) {
        let size = snapshot.total_bytes();
        clip.size = size as u64;
        if size <= memory_limit {
            clip.data = Some(snapshot);
        } else {
            let tmp = blob_path.with_extension("clip.tmp");
            if let Err(e) = fs::write(&tmp, snapshot.encode()).and_then(|_| fs::rename(&tmp, &blob_path)) {
                eprintln!("could not save clip: {e}");
                return false;
            }
            clip.on_disk = true;
        }
    }

    let mut history = state.history.lock().unwrap();
    history.insert(clip);
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
    state.history.lock().unwrap().clips.iter().map(Clip::summary).collect()
}

#[tauri::command]
fn copy_clip(app: AppHandle, state: tauri::State<AppState>, id: u64) -> Result<(), String> {
    let (clip, blob_path, images_dir) = {
        let mut history = state.history.lock().unwrap();
        let clip = history.promote(id).ok_or("Clip not found")?;
        history.save();
        (clip, history.blob_path(id), history.images_dir())
    };
    let snapshot = match (&clip.data, clip.on_disk) {
        (Some(data), _) => Some(data.clone()),
        (None, true) => Some(
            fs::read(&blob_path)
                .ok()
                .and_then(|b| Snapshot::decode(&b))
                .ok_or("This clip's saved data is missing")?,
        ),
        (None, false) => None,
    };

    // Stored files are written out so Finder/Explorer can paste them.
    let snapshot = snapshot.map(|s| files::materialize(&s, &state.paste_dir)).transpose()?;

    match snapshot {
        Some(snapshot) if clipboard::SUPPORTED => clipboard::write(&snapshot)?,
        _ => {
            let mut fallback = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            if clip.kind == ClipKind::Image {
                // Image saved by Cliplog 0.1.
                let (width, height, bytes) = images::load_legacy(&images_dir, id)?;
                fallback
                    .set_image(arboard::ImageData {
                        width: width as usize,
                        height: height as usize,
                        bytes: bytes.into(),
                    })
                    .map_err(|e| e.to_string())?;
            } else {
                *state.last_text.lock().unwrap() = Some(clip.text.clone());
                fallback.set_text(clip.text).map_err(|e| e.to_string())?;
            }
        }
    }
    *state.ignore_seq.lock().unwrap() = clipboard::change_count();
    let _ = app.emit("history-changed", ());
    dismiss_overlay(&app);
    Ok(())
}

#[tauri::command]
fn get_thumbnail(state: tauri::State<AppState>, id: u64) -> Option<String> {
    let dir = state.history.lock().unwrap().images_dir();
    images::thumbnail_data_url(&dir, id)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageUsage {
    clips: usize,
    bytes: u64,
}

#[tauri::command]
fn get_storage_usage(state: tauri::State<AppState>) -> StorageUsage {
    let history = state.history.lock().unwrap();
    StorageUsage { clips: history.clips.len(), bytes: history.total_bytes() }
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
    {
        let mut history = state.history.lock().unwrap();
        history.set_limits(&settings);
        history.save();
    }
    let _ = app.emit("history-changed", ());
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
            get_storage_usage,
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
            let paste_dir = app.path().app_cache_dir()?.join("paste");
            let _ = std::fs::remove_dir_all(&paste_dir);
            std::fs::create_dir_all(&data_dir)?;
            let (settings, first_run) = Settings::load(&data_dir);
            let history = History::load(&data_dir, &settings);
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
                paste_dir,
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
