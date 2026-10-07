const listEl = document.getElementById("list");
const emptyEl = document.getElementById("empty");
const searchEl = document.getElementById("search");
const moreEl = document.getElementById("more");

let clips = [];
let settings = { quickCount: 25, expandedCount: 100 };
let expanded = false;
let selected = 0;
let visible = [];
const thumbs = new Map(); // clip id -> data URL

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

const baseName = (path) => path.split(/[\\/]/).pop() || path;

// Text used for the row, tooltip and search.
function label(clip) {
  switch (clip.kind) {
    case "image":
      return clip.image ? `Image ${clip.image.width}×${clip.image.height}` : "Image";
    case "files": {
      const files = clip.text.split("\n");
      return files.length === 1 ? baseName(files[0]) : `${files.length} files: ${files.map(baseName).join(", ")}`;
    }
    case "other":
      return `Content from an app (${formatBytes(clip.size)})`;
    default:
      return clip.text;
  }
}

function thumbnail(clip) {
  const img = document.createElement("img");
  img.alt = label(clip);
  if (thumbs.has(clip.id)) {
    img.src = thumbs.get(clip.id);
  } else {
    invoke("get_thumbnail", { id: clip.id }).then((url) => {
      if (url) { thumbs.set(clip.id, url); img.src = url; }
    });
  }
  return img;
}

function timeAgo(ms) {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

function render() {
  const q = searchEl.value.trim().toLowerCase();
  const limit = expanded ? settings.expandedCount : settings.quickCount;
  visible = q ? clips.filter((c) => label(c).toLowerCase().includes(q)) : clips.slice(0, limit);
  selected = Math.min(selected, Math.max(visible.length - 1, 0));

  listEl.replaceChildren(
    ...visible.map((clip, i) => {
      const li = document.createElement("li");
      li.role = "option";
      if (i === selected) li.classList.add("selected");
      const full = clip.kind === "files" ? clip.text : label(clip);
      li.title = full.length > 2000 ? full.slice(0, 2000) + "…" : full;

      const num = document.createElement("span");
      num.className = "num";
      num.textContent = i < 9 ? (IS_MAC ? "⌘" : "Ctrl+") + (i + 1) : "";

      const text = document.createElement("span");
      text.className = "text";
      if (clip.kind === "image" && clip.image) {
        li.classList.add("image");
        const size = document.createElement("small");
        size.textContent = `${clip.image.width}×${clip.image.height}`;
        text.append(thumbnail(clip), size);
      } else if (clip.kind === "text") {
        text.textContent = clip.text.replace(/\s+/g, " ").trim().slice(0, 300);
      } else {
        const tag = document.createElement("span");
        tag.className = "tag";
        tag.textContent = { files: clip.text.includes("\n") ? "Files" : "File", image: "Image", other: "App data" }[clip.kind];
        if (clip.savedFiles?.length) {
          tag.textContent += " · saved";
          tag.title = "Cliplog saved a copy, so this pastes even if the original is moved or deleted.";
        }
        text.append(tag, label(clip));
      }

      const when = document.createElement("span");
      when.className = "when";
      when.textContent = timeAgo(clip.copiedAt);

      const del = document.createElement("button");
      del.className = "del";
      del.title = "Remove from history";
      del.textContent = "×";
      del.addEventListener("click", (e) => {
        e.stopPropagation();
        invoke("delete_clip", { id: clip.id });
      });

      li.append(num, text, when, del);
      li.addEventListener("click", () => copy(clip));
      li.addEventListener("mousemove", () => {
        if (selected !== i) { selected = i; highlight(); }
      });
      return li;
    })
  );

  emptyEl.hidden = visible.length > 0;
  emptyEl.textContent = q ? "No matches." : "Nothing here yet. Copy something and it will show up here.";

  const hasMore = clips.length > settings.quickCount && settings.expandedCount > settings.quickCount;
  moreEl.hidden = !!q || !hasMore;
  moreEl.textContent = expanded
    ? `Show fewer (${settings.quickCount})`
    : `Show more (${Math.min(settings.expandedCount, clips.length)})`;
}

function highlight() {
  [...listEl.children].forEach((li, i) => li.classList.toggle("selected", i === selected));
  listEl.children[selected]?.scrollIntoView({ block: "nearest" });
}

async function copy(clip) {
  if (clip) await invoke("copy_clip", { id: clip.id });
}

async function refresh() {
  [clips, settings] = await Promise.all([invoke("get_history"), invoke("get_settings")]);
  render();
}

moreEl.addEventListener("click", () => {
  expanded = !expanded;
  render();
  searchEl.focus();
});

searchEl.addEventListener("input", () => {
  selected = 0;
  render();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    invoke("hide_overlay");
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    selected = Math.min(selected + 1, visible.length - 1);
    highlight();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    selected = Math.max(selected - 1, 0);
    highlight();
  } else if (e.key === "Enter") {
    e.preventDefault();
    copy(visible[selected]);
  } else if ((IS_MAC ? e.metaKey : e.ctrlKey) && /^[1-9]$/.test(e.key)) {
    e.preventDefault();
    copy(visible[Number(e.key) - 1]);
  }
});

listen("overlay-shown", () => {
  searchEl.value = "";
  expanded = false;
  selected = 0;
  refresh();
  searchEl.focus();
});
listen("history-changed", refresh);
listen("settings-changed", refresh);

refresh();
