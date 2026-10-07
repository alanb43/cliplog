//! Saved copies of small copied files.
//!
//! Copying a file in Finder/Explorer only puts a reference (its path) on the
//! clipboard. For files up to `MAX_SAVED_FILE_BYTES`, Cliplog also saves a
//! copy at `files/<clip id>/<n>/<original name>`, and pasting the clip later
//! points at that copy instead of the original. So pasting gives you the file
//! as it was when you copied it, even if the original was since moved, edited
//! or deleted. Bigger files and folders stay references to the original.

use crate::store::INLINE_LIMIT;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Files up to this size get a saved copy (same threshold as clips that are
/// kept inline in history.json instead of in their own file).
pub const MAX_SAVED_FILE_BYTES: u64 = INLINE_LIMIT as u64;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedFile {
    /// Path of the file that was copied.
    pub original: String,
    /// Path of Cliplog's copy.
    pub saved: String,
}

/// Saves copies of the small regular files in `paths`. Returns the copies
/// made and their total size in bytes.
pub fn save_copies(root: &Path, id: u64, paths: &[String]) -> (Vec<SavedFile>, u64) {
    let mut saved = Vec::new();
    let mut total = 0;
    for (i, original) in paths.iter().enumerate() {
        let src = Path::new(original);
        let Ok(meta) = fs::metadata(src) else { continue };
        let Some(name) = src.file_name() else { continue };
        if !meta.is_file() || meta.len() > MAX_SAVED_FILE_BYTES {
            continue;
        }
        let dest: PathBuf = root.join(id.to_string()).join(i.to_string()).join(name);
        let copied = dest
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::copy(src, &dest));
        if let Ok(bytes) = copied {
            total += bytes;
            saved.push(SavedFile { original: original.clone(), saved: dest.to_string_lossy().into_owned() });
        }
    }
    (saved, total)
}

/// Mixes each file's size and modification time into `base`, so copying the
/// same file again after editing it counts as a new clip.
pub fn fingerprint(base: u64, paths: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    base.hash(&mut h);
    for p in paths {
        if let Ok(meta) = fs::metadata(p) {
            meta.len().hash(&mut h);
            meta.modified().ok().hash(&mut h);
        }
    }
    h.finish()
}

/// Maps an original path to its saved copy, if one exists on disk.
pub fn lookup<'a>(saved: &'a [SavedFile], original: &str) -> Option<&'a str> {
    saved
        .iter()
        .find(|s| s.original == original && Path::new(&s.saved).is_file())
        .map(|s| s.saved.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_only_small_regular_files() {
        let tmp = std::env::temp_dir().join(format!("cliplog-files-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        let small = tmp.join("src/notes.txt");
        let big = tmp.join("src/big.bin");
        fs::write(&small, b"hello").unwrap();
        fs::write(&big, vec![0u8; MAX_SAVED_FILE_BYTES as usize + 1]).unwrap();
        let paths: Vec<String> = [&small, &big, &tmp.join("src")]
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        let (saved, bytes) = save_copies(&tmp.join("files"), 7, &paths);
        assert_eq!(bytes, 5);
        assert_eq!(saved.len(), 1);
        assert!(saved[0].saved.ends_with("notes.txt"));
        assert_eq!(fs::read(&saved[0].saved).unwrap(), b"hello");

        // The saved copy survives the original being deleted.
        fs::remove_file(&small).unwrap();
        assert_eq!(lookup(&saved, &paths[0]), Some(saved[0].saved.as_str()));
        assert_eq!(lookup(&saved, &paths[1]), None);
        let _ = fs::remove_dir_all(&tmp);
    }
}
