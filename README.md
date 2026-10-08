# Cliplog

A tiny, free clipboard history for **macOS** and **Windows**.

Cliplog remembers the last 100 things you copied: text, images, files, and anything else apps put on the clipboard. Press a keyboard shortcut anywhere and your 25 most recent clips pop up. Click one, or use the arrow keys and press Enter, and it's back on your clipboard ready to paste.

- **Download:** https://alanb43.github.io/cliplog
- **Size:** about 3 MB on macOS. It uses the system's built-in web view, so no copy of Chrome is bundled.
- **Private:** history is stored only on your computer. There's no network access, no accounts and no analytics.

## Features

- Remembers **everything** you copy, not just text. Each clip is saved in every format the source app provided (plain text, rich text, HTML, images, file references, app-specific data), so pasting it later works just like pasting the original.
- Copying something already in the history moves it to the top instead of adding a duplicate.
- You choose how many clips to keep (default 100), how much disk space they may use (default 500 MB), and the max size of a clip kept in memory (default 32 KB). The oldest clips are removed first.
- Global shortcut, configurable in Settings (default <kbd>⌘ Shift V</kbd> / <kbd>Ctrl Shift V</kbd>).
- Shows 25 clips by default. **Show more** reveals more, and both numbers can be changed in Settings.
- Search box, keyboard navigation (<kbd>↑</kbd> <kbd>↓</kbd> <kbd>Enter</kbd>, <kbd>⌘/Ctrl 1–9</kbd>), and <kbd>Esc</kbd> or clicking away to close.
- Lives in the menu bar or system tray. Can optionally start at login.
- Opening the app while it's already running shows your history.

## What gets saved

| You copy… | Cliplog shows | Pasting it later gives you |
|---|---|---|
| Text (from any app) | the text | the same text, with its formatting where the target app supports it |
| An image or screenshot | a thumbnail | the same image |
| Files in Finder / Explorer | the file names | the same files (see note below) |
| App-specific content (a Figma layer, Excel cells, a Photoshop selection, …) | “Content from an app” | the same content, as long as the app you paste into understands it |

> **Note on copied files:** copying a file puts a *reference* to it on the clipboard (its location), not the file itself.
> - **Files up to the max clip size in memory (default 32 KB):** Cliplog stores the file's contents in the clip, just like text or image data. Pasting gives you the file as it was when you copied it, even if the original was later moved, edited or deleted. Finder and Explorer can only paste real files, so Cliplog writes the file to a temporary cache folder at paste time. That folder only ever holds the last pasted clip's files.
> - **Bigger files and folders:** Cliplog keeps only the reference, shown as "File link" in the popup. If the original is moved, renamed or deleted, pasting that clip won't find it.

Other edge cases:

- Some apps provide clipboard data only while they're still running. If an app quits before Cliplog can read its copy, that copy may be missing or incomplete.
- A small number of formats only work while the source app is running (OLE objects on Windows, file promises on macOS), so Cliplog skips them.
- Anything bigger than your storage limit keeps only its text. Single formats over 100 MB are skipped.

## Settings

| Setting | Default | What it does |
|---|---|---|
| Keyboard shortcut | ⌘⇧V / Ctrl+Shift+V | Opens the popup from anywhere |
| Clips shown at first / after "Show more" | 25 / 100 | How many clips the popup lists |
| Clips to keep | 100 | Older clips are removed |
| Max storage | 500 MB | Oldest clips are removed until everything fits |
| Max clip size in memory | 32 KB | Clips up to this size stay in memory (copied files included); bigger clips are saved to disk, bigger files are kept as links |
| Start at login | on | Launch Cliplog when you log in |

## Privacy

Cliplog never sends anything anywhere. It has no network access, accounts or analytics.

## Where's my data?

| OS      | Folder |
|---------|--------|
| macOS   | `~/Library/Application Support/io.github.alanb43.cliplog/` |
| Windows | `%APPDATA%\io.github.alanb43.cliplog\` |

| File | Contents |
|---|---|
| `settings.json` | your preferences |
| `history.json` | the list of clips; clips up to the max clip size in memory are stored here directly |
| `data/<id>.clip` | larger clips such as screenshots, one file each |
| `images/<id>.thumb.png` | thumbnails shown in the popup |

Clips are pruned whenever the history changes: first down to "Clips to keep", then oldest-first until everything fits under "Max storage". Files belonging to removed clips are deleted right away. Delete the folder to reset Cliplog.

## Building from source

Cliplog is built with [Tauri 2](https://tauri.app). The backend is Rust and the UI is plain HTML, CSS and JavaScript with no frontend framework or bundler.

Requirements: [Rust](https://rustup.rs), [Node.js](https://nodejs.org) 18+, and on Windows the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (already included in Windows 10/11).

```bash
npm install
npm run dev      # run with live reload
npm run build    # build an installer into src-tauri/target/release/bundle/
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored   # also tests the real clipboard (overwrites it)
```

### Project layout

```
src/               UI: overlay.html (the popup), settings.html
src-tauri/src/     Rust backend
  clipboard.rs     reading/writing every clipboard format (macOS + Windows)
  store.rs         history, settings, storage and pruning
  images.rs        thumbnails
  lib.rs           clipboard watcher, shortcut, tray, windows
website/           Download page, published to GitHub Pages
.github/workflows/ CI, release builds for Mac + Windows, website deploy
```

## Releasing

1. Bump `version` in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`.
2. `git tag v0.1.1 && git push origin v0.1.1`

The **Release** workflow builds a universal macOS `.dmg` and a Windows `-setup.exe` and attaches them to a GitHub release. The website's download buttons always point at the newest release.

macOS builds are signed with a Developer ID certificate and notarized by Apple (both the app and the `.dmg`), so they open without Gatekeeper warnings. This needs these repository secrets: `APPLE_CERTIFICATE` (base64 `.p12`), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password) and `APPLE_TEAM_ID`. Windows builds aren't signed yet, so SmartScreen asks users to confirm the first time they run the installer.

## License

[MIT](LICENSE): free to use, copy, modify and distribute.
