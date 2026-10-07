# Cliplog

A tiny, free clipboard history for **macOS** and **Windows**.

Cliplog remembers the last 100 things you copied. Press a keyboard shortcut anywhere and your 25 most recent clips pop up. Click one, or use the arrow keys and press Enter, and it's back on your clipboard ready to paste.

- **Download:** https://alanb43.github.io/cliplog
- **Size:** about 3 MB on macOS. It uses the system's built-in web view, so no copy of Chrome is bundled.
- **Private:** history is stored only on your computer. There's no network access, no accounts and no analytics.

## Features

- Keeps the last 100 text clips. Copying something already in the history moves it to the top instead of adding a duplicate.
- Global shortcut, configurable in Settings (default <kbd>⌘ Shift V</kbd> / <kbd>Ctrl Shift V</kbd>).
- Shows 25 clips by default. **Show more** reveals more, and both numbers can be changed in Settings.
- Search box, keyboard navigation (<kbd>↑</kbd> <kbd>↓</kbd> <kbd>Enter</kbd>, <kbd>⌘/Ctrl 1–9</kbd>), and <kbd>Esc</kbd> or clicking away to close.
- Lives in the menu bar or system tray. Can optionally start at login.
- Opening the app while it's already running shows your history.

## Where's my data?

| OS      | Folder |
|---------|--------|
| macOS   | `~/Library/Application Support/io.github.alanb43.cliplog/` |
| Windows | `%APPDATA%\io.github.alanb43.cliplog\` |

`history.json` holds your clips and `settings.json` holds your preferences. Delete them to reset Cliplog.

## Building from source

Cliplog is built with [Tauri 2](https://tauri.app). The backend is Rust and the UI is plain HTML, CSS and JavaScript with no frontend framework or bundler.

Requirements: [Rust](https://rustup.rs), [Node.js](https://nodejs.org) 18+, and on Windows the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (already included in Windows 10/11).

```bash
npm install
npm run dev      # run with live reload
npm run build    # build an installer into src-tauri/target/release/bundle/
cargo test --manifest-path src-tauri/Cargo.toml
```

### Project layout

```
src/               UI: overlay.html (the popup), settings.html
src-tauri/src/     Rust: clipboard watcher, shortcut, tray, persistence
website/           Download page, published to GitHub Pages
.github/workflows/ CI, release builds for Mac + Windows, website deploy
```

## Releasing

1. Bump `version` in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`.
2. `git tag v0.1.1 && git push origin v0.1.1`

The **Release** workflow builds a universal macOS `.dmg` and a Windows `-setup.exe` and attaches them to a GitHub release. The website's download buttons always point at the newest release.

The builds aren't code-signed, so macOS Gatekeeper and Windows SmartScreen ask users to confirm the first time they open the app. The website explains how. Signing requires an Apple Developer account ($99/year) and a Windows code-signing certificate.

## License

[MIT](LICENSE): free to use, copy, modify and distribute.
