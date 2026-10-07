//! Clipboard history and user settings, persisted as small JSON files in the
//! app's data directory.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many clips are kept on disk.
pub const MAX_HISTORY: usize = 100;
/// Clips larger than this are not stored (keeps the history file small).
pub const MAX_CLIP_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: u64,
    pub text: String,
    pub copied_at: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct History {
    pub clips: Vec<Clip>,
    #[serde(skip)]
    path: PathBuf,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl History {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("history.json");
        let mut history: History = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        history.clips.truncate(MAX_HISTORY);
        history.path = path;
        history
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            let tmp = self.path.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
    }

    /// Adds a newly copied text to the top. If the same text is already in the
    /// history it is moved to the top instead of duplicated.
    /// Returns false if the text was ignored.
    pub fn push(&mut self, text: String) -> bool {
        if text.trim().is_empty() || text.len() > MAX_CLIP_BYTES {
            return false;
        }
        if self.clips.first().is_some_and(|c| c.text == text) {
            return false;
        }
        self.clips.retain(|c| c.text != text);
        let id = self.clips.iter().map(|c| c.id).max().unwrap_or(0).max(now_ms()) + 1;
        self.clips.insert(
            0,
            Clip {
                id,
                text,
                copied_at: now_ms(),
            },
        );
        self.clips.truncate(MAX_HISTORY);
        true
    }

    /// Moves an existing clip to the top and returns its text.
    pub fn promote(&mut self, id: u64) -> Option<String> {
        let idx = self.clips.iter().position(|c| c.id == id)?;
        let mut clip = self.clips.remove(idx);
        clip.copied_at = now_ms();
        let text = clip.text.clone();
        self.clips.insert(0, clip);
        Some(text)
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "CommandOrControl+Shift+V".into(),
            quick_count: 25,
            expanded_count: MAX_HISTORY,
            launch_at_login: true,
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
        self.quick_count = self.quick_count.clamp(1, MAX_HISTORY);
        self.expanded_count = self.expanded_count.clamp(1, MAX_HISTORY);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> History {
        History::default()
    }

    #[test]
    fn push_dedupes_and_moves_to_top() {
        let mut h = history();
        assert!(h.push("a".into()));
        assert!(h.push("b".into()));
        assert!(!h.push("b".into()));
        assert!(h.push("a".into()));
        let texts: Vec<_> = h.clips.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["a", "b"]);
    }

    #[test]
    fn push_ignores_blank_and_caps_length() {
        let mut h = history();
        assert!(!h.push("   \n".into()));
        for i in 0..(MAX_HISTORY + 20) {
            h.push(format!("clip {i}"));
        }
        assert_eq!(h.clips.len(), MAX_HISTORY);
        assert_eq!(h.clips[0].text, format!("clip {}", MAX_HISTORY + 19));
    }

    #[test]
    fn promote_moves_clip_to_top() {
        let mut h = history();
        h.push("a".into());
        h.push("b".into());
        let id = h.clips[1].id;
        assert_eq!(h.promote(id).as_deref(), Some("a"));
        assert_eq!(h.clips[0].text, "a");
    }
}
