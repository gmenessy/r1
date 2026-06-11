//! Autonomes LLM-Wiki – Speicherschicht.
//!
//! ADR-0001: Source of Truth sind reine Markdown-Dateien mit
//! YAML-Frontmatter in einem lokalen Ordner. Der semantische Index
//! (Embeddings) ist abgeleitet und jederzeit rekonstruierbar – siehe
//! docs/ARCHITECTURE.md.

pub mod index;

use anyhow::{Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Meta {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub meta: Meta,
    pub body: String,
}

#[derive(Clone)]
pub struct WikiStore {
    pub dir: PathBuf,
}

fn slugify(s: &str) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "notiz".into()
    } else {
        slug.chars().take(48).collect()
    }
}

impl WikiStore {
    pub fn from_env() -> Self {
        let dir = std::env::var("VIBE_WIKI_DIR").unwrap_or_else(|_| "vibe-wiki".into());
        Self { dir: PathBuf::from(dir) }
    }

    pub fn save(&self, meta: &Meta, body: &str) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("Wiki-Ordner {} anlegen", self.dir.display()))?;
        let stamp = Local::now().format("%Y%m%d-%H%M%S");
        let path = self.dir.join(format!("{}-{stamp}.md", slugify(&meta.title)));
        let frontmatter = serde_yaml::to_string(meta)?;
        let content = format!("---\n{frontmatter}---\n\n{body}\n");
        std::fs::write(&path, content)
            .with_context(|| format!("{} schreiben", path.display()))?;
        Ok(path)
    }

    pub fn load_all(&self) -> Vec<Entry> {
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for item in read.flatten() {
            let path = item.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    entries.push(parse_entry(&path, &raw));
                }
            }
        }
        entries
    }
}

fn parse_entry(path: &Path, raw: &str) -> Entry {
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some((fm, body)) = rest.split_once("\n---\n") {
            if let Ok(meta) = serde_yaml::from_str::<Meta>(fm) {
                return Entry {
                    path: path.to_path_buf(),
                    meta,
                    body: body.trim().to_string(),
                };
            }
        }
    }
    // Datei ohne (gültiges) Frontmatter: Dateiname als Titel.
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Entry {
        path: path.to_path_buf(),
        meta: Meta { title, ..Meta::default() },
        body: raw.trim().to_string(),
    }
}
