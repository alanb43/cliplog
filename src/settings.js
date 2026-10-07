const form = document.getElementById("form");
const shortcutBtn = document.getElementById("shortcut");
const hint = document.getElementById("shortcut-hint");
const quick = document.getElementById("quick");
const expanded = document.getElementById("expanded");
const login = document.getElementById("login");
const keep = document.getElementById("keep");
const storage = document.getElementById("storage");
const usage = document.getElementById("usage");
const statusEl = document.getElementById("status");
const clearBtn = document.getElementById("clear");

let shortcut = "";
let recording = false;
const DEFAULT_HINT = hint.textContent;

function showStatus(msg, isError = false) {
  statusEl.textContent = msg;
  statusEl.className = isError ? "error" : "ok";
}

async function load() {
  const s = await invoke("get_settings");
  shortcut = s.shortcut;
  shortcutBtn.textContent = prettyShortcut(shortcut);
  quick.value = s.quickCount;
  expanded.value = s.expandedCount;
  login.checked = s.launchAtLogin;
  keep.value = s.historySize;
  storage.value = s.maxStorageMb;
  const u = await invoke("get_storage_usage");
  const mb = u.bytes / 1024 / 1024;
  usage.textContent = `Currently storing ${u.clips} clip${u.clips === 1 ? "" : "s"} using ${mb < 0.1 ? "under 0.1" : mb.toFixed(1)} MB. ` +
    "Oldest clips are removed first when either limit is reached.";
}

function stopRecording() {
  if (!recording) return;
  recording = false;
  shortcutBtn.classList.remove("recording");
  shortcutBtn.textContent = prettyShortcut(shortcut);
  hint.textContent = DEFAULT_HINT;
  invoke("resume_shortcut");
}

shortcutBtn.addEventListener("click", () => {
  if (recording) return stopRecording();
  recording = true;
  shortcutBtn.classList.add("recording");
  shortcutBtn.textContent = "Press keys…";
  hint.textContent = "Use at least one of Ctrl, Alt/Option or Cmd/Win plus a key. Esc to cancel.";
  invoke("pause_shortcut");
});
shortcutBtn.addEventListener("blur", stopRecording);

document.addEventListener("keydown", (e) => {
  if (!recording) return;
  e.preventDefault();
  if (e.key === "Escape") return stopRecording();
  if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
  if (!(e.ctrlKey || e.altKey || e.metaKey)) {
    hint.textContent = "Add Ctrl, Alt/Option or Cmd/Win so it doesn't clash with normal typing.";
    return;
  }
  const mods = [];
  if (e.metaKey) mods.push("Super");
  if (e.ctrlKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  shortcut = [...mods, e.code.replace(/^(Key|Digit)/, "")].join("+");
  stopRecording();
  showStatus("Press Save to start using the new shortcut.");
});

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  try {
    await invoke("save_settings", {
      settings: {
        shortcut,
        quickCount: Number(quick.value),
        expandedCount: Number(expanded.value),
        launchAtLogin: login.checked,
        historySize: Number(keep.value),
        maxStorageMb: Number(storage.value),
      },
    });
    await load();
    showStatus(`Saved. Press ${prettyShortcut(shortcut)} anywhere to open your clipboard history.`);
  } catch (err) {
    await load();
    showStatus(String(err), true);
  }
});

let clearArmed = false;
clearBtn.addEventListener("click", async () => {
  if (!clearArmed) {
    clearArmed = true;
    clearBtn.textContent = "Click again to clear";
    setTimeout(() => { clearArmed = false; clearBtn.textContent = "Clear history"; }, 3000);
    return;
  }
  clearArmed = false;
  clearBtn.textContent = "Clear history";
  await invoke("clear_history");
  showStatus("History cleared.");
});

load();
