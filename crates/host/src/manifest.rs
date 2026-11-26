//! The output manifest: the files the last build wrote to the output
//! directory, kept in `.lsdocs-cache/outputs.json`, so the first build of
//! a later session deletes what that build wrote and this one does not
//! (an old hashed theme asset, a page whose source is gone). Within a
//! session, the host knows this from the build before.
//!
//! The manifest only decides what to delete: files are still compared
//! with what is on disk before being skipped. It names the output
//! directory it is for, and is ignored for another one. It is kept with
//! or without the build cache.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The manifest's file name in the cache directory.
const FILE: &str = "outputs.json";

/// Bumped when the manifest's layout changes; others are ignored.
const FORMAT: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: u32,
    /// The output directory, absolute.
    output_dir: PathBuf,
    /// The files written, relative to it with `/` separators, sorted.
    files: Vec<String>,
}

/// The manifest's path for the site at `root`.
fn path(root: &Path) -> PathBuf {
    crate::cache::dir(root).join(FILE)
}

fn absolute(dir: &Path) -> PathBuf {
    std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// The files the last build wrote to `output_dir`, or none if there is no
/// readable manifest for that directory. Paths that would leave the
/// output directory are dropped.
pub(crate) fn read(root: &Path, output_dir: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path(root)) else {
        return Vec::new();
    };
    match serde_json::from_slice::<Manifest>(&bytes) {
        Ok(m) if m.format == FORMAT && m.output_dir == absolute(output_dir) => m
            .files
            .into_iter()
            .filter(|f| crate::is_safe_output_path(f))
            .collect(),
        Ok(_) => Vec::new(),
        Err(e) => {
            tracing::debug!("ignoring the output manifest: {e}");
            Vec::new()
        }
    }
}

/// Records `files` as what the last build wrote to `output_dir`. A
/// failure is logged, not returned: without the manifest, a later session
/// only prunes less.
pub(crate) fn write<'a>(root: &Path, output_dir: &Path, files: impl Iterator<Item = &'a String>) {
    let mut files: Vec<String> = files.cloned().collect();
    files.sort();
    let manifest = Manifest {
        format: FORMAT,
        output_dir: absolute(output_dir),
        files,
    };
    let path = path(root);
    let result = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| e.to_string())
        .and_then(|mut bytes| {
            bytes.push(b'\n');
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|e| e.to_string())
        });
    if let Err(e) = result {
        tracing::warn!("cannot write the output manifest {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn site() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lsdocs-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_for_the_same_output_dir_only() {
        let root = site();
        let out = root.join("_site");
        let files = ["b.html".to_owned(), "a.css".to_owned()];
        write(&root, &out, files.iter());
        assert_eq!(read(&root, &out), ["a.css", "b.html"]);
        assert!(read(&root, &root.join("other")).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn drops_paths_outside_the_output_dir_and_bad_manifests() {
        let root = site();
        let out = root.join("_site");
        assert!(read(&root, &out).is_empty());
        let manifest = Manifest {
            format: FORMAT,
            output_dir: absolute(&out),
            files: vec!["../x".into(), "ok.html".into(), "/abs".into()],
        };
        std::fs::create_dir_all(crate::cache::dir(&root)).unwrap();
        std::fs::write(path(&root), serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(read(&root, &out), ["ok.html"]);
        std::fs::write(path(&root), b"not json").unwrap();
        assert!(read(&root, &out).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
