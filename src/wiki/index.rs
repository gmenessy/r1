//! Semantischer Index über dem Markdown-Store (abgeleitete Daten).
//!
//! Primär Cosine-Similarity über lokale Embeddings (Ollama). Fällt
//! ohne Embedding-Modell auf Token-Overlap (Jaccard) zurück, damit
//! proaktives RAG auch komplett ohne Zusatzmodelle funktioniert.

use crate::events::WikiHint;
use crate::wiki::Entry;
use std::collections::HashSet;

pub struct Doc {
    pub title: String,
    pub summary: String,
    pub embedding: Option<Vec<f32>>,
    pub tokens: HashSet<String>,
}

#[derive(Default)]
pub struct Index {
    pub docs: Vec<Doc>,
}

pub fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 3)
        .map(|w| w.to_string())
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    inter / union
}

impl Index {
    pub fn add(&mut self, entry: &Entry, embedding: Option<Vec<f32>>) {
        let title = if entry.meta.title.is_empty() {
            entry.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            entry.meta.title.clone()
        };
        self.docs.push(Doc {
            title,
            summary: entry.meta.summary.clone(),
            embedding,
            tokens: tokenize(&format!(
                "{} {} {}",
                entry.meta.title,
                entry.meta.tags.join(" "),
                entry.body
            )),
        });
    }

    /// Top-3-Treffer für den aktuellen Schreibkontext.
    pub fn search(&self, query_text: &str, query_embedding: Option<&[f32]>) -> Vec<WikiHint> {
        let qtokens = tokenize(query_text);
        let mut scored: Vec<(f32, &Doc)> = self
            .docs
            .iter()
            .map(|doc| {
                let score = match (query_embedding, doc.embedding.as_deref()) {
                    (Some(q), Some(d)) => cosine(q, d),
                    _ => jaccard(&qtokens, &doc.tokens),
                };
                (score, doc)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let threshold = if query_embedding.is_some() { 0.45 } else { 0.08 };
        scored
            .into_iter()
            .take(3)
            .filter(|(score, _)| *score >= threshold)
            .map(|(score, doc)| WikiHint {
                title: doc.title.clone(),
                summary: doc.summary.clone(),
                score,
            })
            .collect()
    }
}
