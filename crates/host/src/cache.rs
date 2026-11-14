//! The build cache: plugin results on disk, keyed by a hash of everything
//! they depend on, so unchanged work is not done again. Cached are parse,
//! collect and render results, and apply results keyed on the page plus a
//! digest of everything collected in that pass (so an edit that changes no
//! collected data, such as rewording a paragraph, leaves the other pages'
//! results valid).
//!
//! Only plugins that declare a cache key in their capabilities
//! (`Capabilities::cache`) are cached. A result's key covers the cache
//! format, the lsdocs version, the plugin's name, version, options and
//! cache key, the site settings (but not the file list), the phase, and the
//! request's input. Entries are MessagePack files named by their key in
//! `.lsdocs-cache/` at the site root. After a build, entries it did not use
//! are deleted, so the cache holds what the last build needed.
//!
//! Cached results replay the diagnostics they carry; logs, progress and
//! diagnostics a plugin sent as notifications are not replayed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use lsdocs_protocol::doctree::Document;
use lsdocs_protocol::{SiteInfo, msgpack};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::plugin::PluginClient;

/// Bumped when the layout of cache entries changes.
const FORMAT: &str = "lsdocs-cache-1";

/// Where the cache lives, relative to the site root.
pub const DIR: &str = ".lsdocs-cache";

/// The build cache.
pub(crate) struct Cache {
    dir: PathBuf,
    /// The site settings part of every key.
    site: Vec<u8>,
    /// Keys read or written by this build.
    used: Mutex<HashSet<String>>,
    hits: AtomicUsize,
}

impl Cache {
    /// The cache in `dir`, for a site with these settings.
    pub fn new(dir: PathBuf, site: &SiteInfo) -> Self {
        let site = SiteInfo {
            files: Vec::new(),
            ..site.clone()
        };
        Self {
            dir,
            site: serde_json::to_vec(&site).unwrap_or_default(),
            used: Mutex::default(),
            hits: AtomicUsize::new(0),
        }
    }

    /// The key for `plugin`'s result in `phase` for an input made of
    /// `parts`, or `None` if the plugin may not be cached.
    pub fn key(&self, plugin: &PluginClient, phase: &str, parts: &[&[u8]]) -> Option<String> {
        let plugin_key = plugin.capabilities.cache.as_deref()?;
        let options = serde_json::to_vec(&plugin.options).unwrap_or_default();
        let mut hasher = blake3::Hasher::new();
        let fields: [&[u8]; 8] = [
            FORMAT.as_bytes(),
            env!("CARGO_PKG_VERSION").as_bytes(),
            plugin.plugin_name.as_bytes(),
            plugin.version.as_bytes(),
            &options,
            plugin_key.as_bytes(),
            &self.site,
            phase.as_bytes(),
        ];
        for field in fields.iter().chain(parts) {
            // Length-prefixed, so field boundaries cannot be confused.
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        Some(hasher.finalize().to_hex().to_string())
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.msgpack"))
    }

    fn mark(&self, key: &str) {
        self.used
            .lock()
            .expect("not poisoned")
            .insert(key.to_owned());
    }

    /// The entry for `key`, if there is a readable one.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let bytes = std::fs::read(self.path(key)).ok()?;
        let value = msgpack::decode(&bytes).ok()?;
        self.mark(key);
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(value)
    }

    /// Stores `value` under `key`. Failing to write only costs a cache miss
    /// next time, so errors are logged, not returned.
    pub fn put<T: Serialize>(&self, key: &str, value: &T) {
        self.mark(key);
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            let bytes = msgpack::encode(value).map_err(std::io::Error::other)?;
            // Write then rename, so a reader never sees a partial entry.
            let temp = self.dir.join(format!("{key}.tmp"));
            std::fs::write(&temp, bytes)?;
            std::fs::rename(&temp, self.path(key))
        })();
        if let Err(e) = result {
            tracing::debug!("could not write cache entry {key}: {e}");
        }
    }

    /// How many results came from the cache.
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    /// Deletes the entries this build did not use.
    pub fn prune(&self) {
        let used = self.used.lock().expect("not poisoned");
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let keep = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| used.contains(stem))
                && path.extension().is_some_and(|e| e == "msgpack");
            if !keep {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// The key for a request about `document`, plus `extra` input, if there is
/// a cache and `plugin` may be cached.
pub(crate) fn document_key(
    cache: &Option<Arc<Cache>>,
    plugin: &PluginClient,
    phase: &str,
    document: &Document,
    extra: &[u8],
) -> Option<String> {
    let cache = cache.as_ref()?;
    plugin.capabilities.cache.as_ref()?;
    let bytes = msgpack::encode(document).ok()?;
    cache.key(plugin, phase, &[&bytes, extra])
}

/// The cached result for `key`, if any.
pub(crate) fn get<T: DeserializeOwned>(
    cache: &Option<Arc<Cache>>,
    key: &Option<String>,
) -> Option<T> {
    cache.as_ref()?.get(key.as_ref()?)
}

/// Caches `value` under `key`, if there is a cache and a key.
pub(crate) fn put<T: Serialize>(cache: &Option<Arc<Cache>>, key: &Option<String>, value: &T) {
    if let (Some(cache), Some(key)) = (cache, key) {
        cache.put(key, value);
    }
}

/// A digest of `value`, for keys that depend on it.
pub(crate) fn digest(value: &impl Serialize) -> Vec<u8> {
    let bytes = msgpack::encode(value).unwrap_or_default();
    blake3::hash(&bytes).as_bytes().to_vec()
}

/// The cache directory of the site at `root`.
pub fn dir(root: &Path) -> PathBuf {
    root.join(DIR)
}
