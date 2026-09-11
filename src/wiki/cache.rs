//! Persistenter Embedding-Cache (`.vibe/embeddings.json`).
//!
//! Abgeleitete Daten gemäß ADR-0001: Schlüssel ist ein Hash über
//! Embedding-Modell + Inhalt, ein veralteter oder gelöschter Cache wird
//! beim nächsten Start einfach neu aufgebaut.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(Default)]
pub struct EmbeddingCache {
    path: PathBuf,
    entries: HashMap<u64, Vec<f32>>,
    dirty: bool,
}

pub fn content_key(model: &str, text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}

impl EmbeddingCache {
    pub fn default_path() -> PathBuf {
        PathBuf::from(std::env::var("VIBE_CACHE_DIR").unwrap_or_else(|_| ".vibe".into()))
            .join("embeddings.json")
    }

    pub fn load(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<HashMap<String, Vec<f32>>>(&raw).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| k.parse::<u64>().ok().map(|k| (k, v)))
                    .collect()
            })
            .unwrap_or_default();
        Self { path, entries, dirty: false }
    }

    pub fn get(&self, key: u64) -> Option<&Vec<f32>> {
        self.entries.get(&key)
    }

    pub fn insert(&mut self, key: u64, embedding: Vec<f32>) {
        self.entries.insert(key, embedding);
        self.dirty = true;
    }

    /// Schreibt nur, wenn sich etwas geändert hat. Fehler sind unkritisch
    /// (Cache ist abgeleitet) und werden verschluckt.
    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let serializable: HashMap<String, &Vec<f32>> =
            self.entries.iter().map(|(k, v)| (k.to_string(), v)).collect();
        if let Ok(json) = serde_json::to_string(&serializable) {
            if std::fs::write(&self.path, json).is_ok() {
                self.dirty = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_disk() {
        let path = std::env::temp_dir().join("vibe-cache-test").join("embeddings.json");
        let _ = std::fs::remove_file(&path);
        let key = content_key("nomic", "hallo");

        let mut cache = EmbeddingCache::load(path.clone());
        assert!(cache.get(key).is_none());
        cache.insert(key, vec![0.1, 0.2]);
        cache.flush();

        let reloaded = EmbeddingCache::load(path.clone());
        assert_eq!(reloaded.get(key), Some(&vec![0.1, 0.2]));
        assert!(reloaded.get(content_key("nomic", "anders")).is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn key_depends_on_model_and_text() {
        assert_ne!(content_key("a", "x"), content_key("b", "x"));
        assert_ne!(content_key("a", "x"), content_key("a", "y"));
        assert_eq!(content_key("a", "x"), content_key("a", "x"));
    }
}
