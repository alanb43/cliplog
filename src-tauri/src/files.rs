//! Copied files.
//!
//! Copying a file in Finder/Explorer only puts a reference (its path) on the
//! clipboard. Files up to the user's "max clip size in memory" are stored in
//! the clip itself, like any other clip data: their contents are added to the
//! clip's snapshot as extra formats named `EMBED_PREFIX + <original path>`.
//! Bigger files and folders are kept only as a link to the original.
//!
//! Finder and Explorer can only paste real files, so when a clip with stored
//! files is pasted, `materialize` writes them to a cache folder and points the
//! clipboard at those. The cache only ever holds the most recently pasted
//! clip's files.

use crate::clipboard::{Format, Snapshot};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Name prefix of the Cliplog-only formats holding file contents. They are
/// never written to the clipboard.
pub const EMBED_PREFIX: &str = "io.github.alanb43.cliplog.file:";

/// Stores the contents of each regular file in `paths` that is at most
/// `max_bytes` in the snapshot. Returns how many were stored.
pub fn embed(snapshot: &mut Snapshot, paths: &[String], max_bytes: u64) -> usize {
    let embedded: Vec<Format> = paths
        .iter()
        .filter(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= max_bytes))
        .filter_map(|p| Some(Format { name: format!("{EMBED_PREFIX}{p}"), data: fs::read(p).ok()? }))
        .collect();
    let count = embedded.len();
    if count > 0 {
        snapshot.items.push(embedded);
    }
    count
}

/// Returns the snapshot to put on the clipboard: stored files are written
/// to `cache_dir` and the clip's file references point at them.
pub fn materialize(snapshot: &Snapshot, cache_dir: &Path) -> Result<Snapshot, String> {
    let mut out = snapshot.clone();
    let mut embedded = Vec::new();
    for item in &mut out.items {
        let (files, rest): (Vec<Format>, Vec<Format>) =
            item.drain(..).partition(|f| f.name.starts_with(EMBED_PREFIX));
        embedded.extend(files);
        *item = rest;
    }
    out.items.retain(|i| !i.is_empty());
    if embedded.is_empty() {
        return Ok(out);
    }

    let _ = fs::remove_dir_all(cache_dir);
    let mut paths = HashMap::new();
    for (i, f) in embedded.iter().enumerate() {
        let original = &f.name[EMBED_PREFIX.len()..];
        let name = Path::new(original).file_name().ok_or("bad file name")?;
        let dest = cache_dir.join(i.to_string()).join(name);
        fs::create_dir_all(dest.parent().unwrap())
            .and_then(|_| fs::write(&dest, &f.data))
            .map_err(|e| format!("Couldn't prepare the file for pasting: {e}"))?;
        paths.insert(original.to_string(), dest.to_string_lossy().into_owned());
    }
    Ok(out.with_file_paths(|p| paths.get(p).cloned()).unwrap_or(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::MAC_FILE_URL;

    #[test]
    fn small_files_are_stored_and_pasted_from_cache() {
        let tmp = std::env::temp_dir().join(format!("cliplog-files-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        let small = tmp.join("src/My Notes.txt");
        let big = tmp.join("src/big.bin");
        fs::write(&small, b"hello").unwrap();
        fs::write(&big, vec![0u8; 101]).unwrap();
        let paths: Vec<String> = [&small, &big, &tmp.join("src")]
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let url = |p: &str| format!("file://{}", p.replace(' ', "%20")).into_bytes();
        let mut snap = Snapshot {
            items: paths.iter().map(|p| vec![Format { name: MAC_FILE_URL.into(), data: url(p) }]).collect(),
        };

        assert_eq!(embed(&mut snap, &paths, 100), 1);
        // The clip keeps working after the original is deleted.
        fs::remove_file(&small).unwrap();

        let cache = tmp.join("cache");
        let pasted = materialize(&snap, &cache).unwrap();
        let files = pasted.files();
        assert_eq!(files.len(), 3);
        assert!(files[0].starts_with(&*cache.to_string_lossy()) && files[0].ends_with("My Notes.txt"));
        assert_eq!(fs::read(&files[0]).unwrap(), b"hello");
        assert_eq!(files[1], paths[1]); // too big: still a link
        assert!(pasted.items.iter().flatten().all(|f| !f.name.starts_with(EMBED_PREFIX)));
        let _ = fs::remove_dir_all(&tmp);
    }
}
