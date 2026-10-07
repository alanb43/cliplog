//! Clipboard history and user settings, persisted in the app's data directory:
//!
//! ```text
//! settings.json          user preferences
//! history.json           the list of clips, newest first
//! data/<id>.clip         raw clipboard data for clips over INLINE_LIMIT bytes
//! images/<id>.thumb.png  preview thumbnail for image clips
//! ```
//!
//! Small clips keep their raw data inside history.json; bigger ones (screenshots,
//! large documents) are written to their own file under `data/` so history.json
//! stays small and quick to rewrite. Plain-text-only clips store just the text.
//!
//! The history is pruned whenever it changes: first to the user's
//! "clips to keep" count, then oldest-first until the total stored size is
//! under the user's storage cap. Files that no longer belong to a clip are
//! deleted on every save.

use crate::clipboard::Snapshot;
use crate::images;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Clips whose raw data is at most this size stay inline in history.json.
pub const INLINE_LIMIT: usize = 32 * 1024;

pub const DEFAULT_HISTORY_SIZE: usize = 100;
pub const HISTORY_SIZE_RANGE: (usize, usize) = (1, 1000);
pub const DEFAULT_MAX_STORAGE_MB: u64 = 500;
pub const MAX_STORAGE_MB_RANGE: (u64, u64) = (10, 100_000);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    #[default]
    Text,
    Image,
    Files,
    /// App-specific data with no text, image or files (e.g. a Figma layer).
    Other,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: u64,
    #[serde(default)]
    pub kind: ClipKind,
    /// Plain text (for text clips) or newline-separated paths (for files).
    /// Used for the preview and search.
    pub text: String,
    pub copied_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageInfo>,
    /// Hash of the raw data, to avoid storing the same copy twice.
    #[serde(default)]
    pub hash: u64,
    /// Bytes this clip takes up on disk (raw data or text).
    #[serde(default)]
    pub size: u64,
    /// Raw clipboard data, when small enough to keep inline.
    #[serde(default, rename = "inlineData", skip_serializing_if = "Option::is_none")]
    pub data: Option<Snapshot>,
    /// Raw data lives in `data/<id>.clip`.
    #[serde(default)]
    pub on_disk: bool,
}

impl Clip {
    pub fn new(id: u64, kind: ClipKind, text: String) -> Self {
        Clip {
            id,
            kind,
            size: text.len() as u64,
            text,
            copied_at: now_ms(),
            image: None,
            hash: 0,
            data: None,
            on_disk: false,
        }
    }

    /// The clip as sent to the overlay (without raw data).
    pub fn summary(&self) -> Clip {
        Clip { data: None, ..self.clone() }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct History {
    pub clips: Vec<Clip>,
    #[serde(skip)]
    dir: PathBuf,
    #[serde(skip)]
    max_clips: usize,
    #[serde(skip)]
    max_bytes: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl History {
    pub fn load(dir: &Path, settings: &Settings) -> Self {
        let mut history: History = fs::read_to_string(dir.join("history.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        history.dir = dir.to_path_buf();
        let _ = fs::create_dir_all(history.data_dir());
        let _ = fs::create_dir_all(history.images_dir());
        let images_dir = history.images_dir();
        for clip in &mut history.clips {
            if clip.size == 0 {
                // Clips saved by older versions.
                clip.size = match clip.kind {
                    ClipKind::Image => fs::metadata(images::legacy_path(&images_dir, clip.id))
                        .map(|m| m.len())
                        .unwrap_or(0),
                    _ => clip.text.len() as u64,
                };
            }
        }
        history.set_limits(settings);
        history
    }

    pub fn data_dir(&self) -> PathBuf {
        self.dir.join("data")
    }

    pub fn images_dir(&self) -> PathBuf {
        self.dir.join("images")
    }

    pub fn blob_path(&self, id: u64) -> PathBuf {
        self.data_dir().join(format!("{id}.clip"))
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            let path = self.dir.join("history.json");
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
        let keep = |id: u64| self.clips.iter().any(|c| c.id == id);
        images::prune(&self.images_dir(), keep);
        images::prune(&self.data_dir(), keep);
    }

    /// Applies the user's limits and prunes right away if they shrank.
    pub fn set_limits(&mut self, settings: &Settings) {
        self.max_clips = settings.history_size;
        self.max_bytes = settings.max_storage_mb * 1024 * 1024;
        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        self.clips.truncate(self.max_clips.max(1));
        let mut total = self.total_bytes();
        // Always keep the newest clip, even if it alone is over the cap.
        while total > self.max_bytes && self.clips.len() > 1 {
            total -= self.clips.pop().map(|c| c.size).unwrap_or(0);
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.clips.iter().map(|c| c.size).sum()
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn next_id(&self) -> u64 {
        self.clips.iter().map(|c| c.id).max().unwrap_or(0).max(now_ms()) + 1
    }

    /// True if `clip` is the same as the newest clip, so recording it would
    /// change nothing.
    pub fn is_same_as_newest(&self, kind: ClipKind, text: &str, hash: u64) -> bool {
        self.clips.first().is_some_and(|c| {
            if kind == ClipKind::Text && c.kind == ClipKind::Text {
                c.text == text
            } else {
                hash != 0 && c.hash == hash
            }
        })
    }

    /// Adds a clip at the top, replacing older copies of the same content.
    pub fn insert(&mut self, clip: Clip) {
        self.clips.retain(|c| {
            let same_text = clip.kind == ClipKind::Text && c.kind == ClipKind::Text && c.text == clip.text;
            let same_data = clip.hash != 0 && c.hash == clip.hash;
            !(same_text || same_data)
        });
        self.clips.insert(0, clip);
        self.enforce_limits();
    }

    /// Adds plain text (used where raw clipboard access isn't available).
    /// Returns false if the text was ignored.
    pub fn push_text(&mut self, text: String) -> bool {
        if text.trim().is_empty() || self.is_same_as_newest(ClipKind::Text, &text, 0) {
            return false;
        }
        self.insert(Clip::new(self.next_id(), ClipKind::Text, text));
        true
    }

    /// Moves an existing clip to the top and returns a copy of it.
    pub fn promote(&mut self, id: u64) -> Option<Clip> {
        let idx = self.clips.iter().position(|c| c.id == id)?;
        let mut clip = self.clips.remove(idx);
        clip.copied_at = now_ms();
        self.clips.insert(0, clip.clone());
        Some(clip)
    }

    pub fn remove(&mut self, id: u64) {
        self.clips.retain(|c| c.id != id);
    }

    pub fn clear(&mut self) {
        self.clips.clear();
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Global shortcut, e.g. "CommandOrControl+Shift+V".
    pub shortcut: String,
    /// Clips shown when the overlay opens.
    pub quick_count: usize,
    /// Clips shown after pressing "Show more".
    pub expanded_count: usize,
    pub launch_at_login: bool,
    /// How many clips are kept.
    pub history_size: usize,
    /// Total disk space clips may use, in megabytes.
    pub max_storage_mb: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "CommandOrControl+Shift+V".into(),
            quick_count: 25,
            expanded_count: DEFAULT_HISTORY_SIZE,
            launch_at_login: true,
            history_size: DEFAULT_HISTORY_SIZE,
            max_storage_mb: DEFAULT_MAX_STORAGE_MB,
        }
    }
}

impl Settings {
    fn path(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    /// Returns the settings and whether this is the first run (no file yet).
    pub fn load(dir: &Path) -> (Self, bool) {
        match fs::read_to_string(Self::path(dir)) {
            Ok(s) => (serde_json::from_str::<Settings>(&s).unwrap_or_default().clamped(), false),
            Err(_) => (Self::default(), true),
        }
    }

    pub fn save(&self, dir: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(Self::path(dir), json);
        }
    }

    pub fn clamped(mut self) -> Self {
        self.history_size = self.history_size.clamp(HISTORY_SIZE_RANGE.0, HISTORY_SIZE_RANGE.1);
        self.max_storage_mb = self.max_storage_mb.clamp(MAX_STORAGE_MB_RANGE.0, MAX_STORAGE_MB_RANGE.1);
        self.quick_count = self.quick_count.clamp(1, self.history_size);
        self.expanded_count = self.expanded_count.clamp(1, self.history_size);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::Format;

    fn history(max_clips: usize, max_mb: u64) -> History {
        let mut h = History::default();
        h.max_clips = max_clips;
        h.max_bytes = max_mb * 1024 * 1024;
        h
    }

    fn data_clip(h: &History, hash: u64, size: u64) -> Clip {
        let mut c = Clip::new(h.next_id(), ClipKind::Image, String::new());
        c.hash = hash;
        c.size = size;
        c
    }

    #[test]
    fn push_text_dedupes_and_moves_to_top() {
        let mut h = history(100, 500);
        assert!(h.push_text("a".into()));
        assert!(h.push_text("b".into()));
        assert!(!h.push_text("b".into()));
        assert!(h.push_text("a".into()));
        let texts: Vec<_> = h.clips.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["a", "b"]);
    }

    #[test]
    fn ignores_blank_and_prunes_to_clip_count() {
        let mut h = history(100, 500);
        assert!(!h.push_text("   \n".into()));
        for i in 0..120 {
            h.push_text(format!("clip {i}"));
        }
        assert_eq!(h.clips.len(), 100);
        assert_eq!(h.clips[0].text, "clip 119");

        let settings = Settings { history_size: 10, ..Settings::default() };
        h.set_limits(&settings);
        assert_eq!(h.clips.len(), 10);
        assert_eq!(h.clips[9].text, "clip 110");
    }

    #[test]
    fn prunes_oldest_to_storage_cap_but_keeps_newest() {
        let mut h = history(100, 10);
        let mb = 1024 * 1024;
        for i in 1..=4 {
            let c = data_clip(&h, i, 4 * mb);
            h.insert(c);
        }
        // 4 MB each, 10 MB cap: only the newest two fit.
        assert_eq!(h.clips.iter().map(|c| c.hash).collect::<Vec<_>>(), [4, 3]);

        let huge = data_clip(&h, 9, 50 * mb);
        h.insert(huge);
        assert_eq!(h.clips.len(), 1);
    }

    #[test]
    fn same_data_replaces_older_copy() {
        let mut h = history(100, 500);
        let a = data_clip(&h, 7, 10);
        h.insert(a);
        h.push_text("x".into());
        assert!(!h.is_same_as_newest(ClipKind::Image, "", 7));
        let again = data_clip(&h, 7, 10);
        h.insert(again);
        assert_eq!(h.clips.len(), 2);
        assert!(h.is_same_as_newest(ClipKind::Image, "", 7));
    }

    #[test]
    fn promote_moves_clip_to_top() {
        let mut h = history(100, 500);
        h.push_text("a".into());
        h.push_text("b".into());
        let id = h.clips[1].id;
        assert_eq!(h.promote(id).map(|c| c.text).as_deref(), Some("a"));
        assert_eq!(h.clips[0].text, "a");
    }

    #[test]
    fn inline_data_roundtrips_and_old_history_loads() {
        let mut h = history(100, 500);
        let mut c = Clip::new(1, ClipKind::Text, "hi".into());
        c.data = Some(Snapshot {
            items: vec![vec![Format { name: "public.html".into(), data: b"<b>hi</b>".to_vec() }]],
        });
        h.clips.push(c);
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains("inlineData"));
        let back: History = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clips[0].data, h.clips[0].data);
        assert!(serde_json::to_string(&back.clips[0].summary()).unwrap().find("inlineData").is_none());

        let old = r#"{"clips":[{"id":1,"text":"hi","copiedAt":5}]}"#;
        let loaded: History = serde_json::from_str(old).unwrap();
        assert_eq!(loaded.clips[0].kind, ClipKind::Text);
    }

    #[test]
    fn settings_are_clamped() {
        let s = Settings { history_size: 5000, max_storage_mb: 1, quick_count: 0, expanded_count: 2000, ..Settings::default() }.clamped();
        assert_eq!((s.history_size, s.max_storage_mb, s.quick_count, s.expanded_count), (1000, 10, 1, 1000));
    }
}
