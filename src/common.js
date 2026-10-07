// Shared helpers for the overlay and settings windows.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const IS_MAC = navigator.userAgent.includes("Mac");

// "CommandOrControl+Shift+V" -> "⌘⇧V" on macOS, "Ctrl+Shift+V" elsewhere.
function prettyShortcut(accel) {
  const mac = { commandorcontrol: "⌘", super: "⌘", command: "⌘", control: "⌃", ctrl: "⌃", alt: "⌥", option: "⌥", shift: "⇧" };
  const win = { commandorcontrol: "Ctrl", super: "Win", command: "Win", control: "Ctrl", ctrl: "Ctrl", alt: "Alt", option: "Alt", shift: "Shift" };
  const map = IS_MAC ? mac : win;
  const parts = accel.split("+").map((p) => map[p.toLowerCase()] ?? p.replace(/^(Key|Digit)/, ""));
  return IS_MAC ? parts.join("") : parts.join("+");
}
