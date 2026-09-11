//! Hybrid-Modell-Schicht (LLM-Agnostizismus).
//!
//! Local-First: Gemma über Ollama ist Standard (offline, kostenlos,
//! datenschutzkonform). Cloud-Modelle (Anthropic, OpenAI) werden per
//! `/model` zur Laufzeit zugeschaltet. Ghost-Text und Wiki-Embeddings
//! laufen IMMER lokal, unabhängig vom gewählten Hauptmodell.

pub mod anthropic;
pub mod ollama;
pub mod openai;
pub mod prompts;

use anyhow::{bail, Result};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Gemma,
    Claude,
    Gpt,
}

/// Modellnamen, die einen Cloud-Anbieter meinen (für die Consent-Abfrage
/// in der UI – bevor der Worker überhaupt etwas sendet).
pub fn is_cloud_name(name: &str) -> bool {
    matches!(name, "claude" | "anthropic" | "gpt" | "openai" | "gpt-4o")
}

#[derive(Clone)]
pub struct Llm {
    http: reqwest::Client,
    pub kind: ModelKind,
    ollama_base: String,
    gemma_model: String,
    ghost_model: String,
    embed_model: String,
    anthropic_key: Option<String>,
    openai_key: Option<String>,
}

impl Llm {
    pub fn from_env() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        let gemma_model = std::env::var("VIBE_GEMMA_MODEL").unwrap_or_else(|_| "gemma3".into());
        Self {
            http,
            kind: ModelKind::Gemma,
            ollama_base: std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434".into()),
            // Ghost-Korrekturen dürfen ein kleineres, schnelleres Modell nutzen.
            ghost_model: std::env::var("VIBE_GHOST_MODEL").unwrap_or_else(|_| gemma_model.clone()),
            gemma_model,
            embed_model: std::env::var("VIBE_EMBED_MODEL")
                .unwrap_or_else(|_| "nomic-embed-text".into()),
            anthropic_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            openai_key: std::env::var("OPENAI_API_KEY").ok(),
        }
    }

    /// Modellwechsel zur Laufzeit. Gibt das Label für die Statuszeile zurück.
    pub fn switch(&mut self, name: &str) -> Result<String, String> {
        match name {
            "gemma" | "local" => {
                self.kind = ModelKind::Gemma;
            }
            "claude" | "anthropic" => {
                if self.anthropic_key.is_none() {
                    return Err("ANTHROPIC_API_KEY nicht gesetzt".into());
                }
                self.kind = ModelKind::Claude;
            }
            "gpt" | "openai" | "gpt-4o" => {
                if self.openai_key.is_none() {
                    return Err("OPENAI_API_KEY nicht gesetzt".into());
                }
                self.kind = ModelKind::Gpt;
            }
            other => return Err(format!("Unbekanntes Modell '{other}' – gemma|claude|gpt")),
        }
        Ok(self.label())
    }

    pub fn is_cloud(&self) -> bool {
        self.kind != ModelKind::Gemma
    }

    pub fn label(&self) -> String {
        match self.kind {
            ModelKind::Gemma => format!("Gemma · lokal ({})", self.gemma_model),
            ModelKind::Claude => format!("Claude · Cloud ({})", anthropic::MODEL),
            ModelKind::Gpt => format!("GPT · Cloud ({})", openai::MODEL),
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self.kind {
            ModelKind::Gemma => "Gemma",
            ModelKind::Claude => "Claude",
            ModelKind::Gpt => "GPT",
        }
    }

    pub fn embed_model(&self) -> &str {
        &self.embed_model
    }

    /// Vollständige Antwort vom aktuell gewählten Modell.
    pub async fn complete(&self, system: &str, user: &str) -> Result<String> {
        match self.kind {
            ModelKind::Gemma => {
                ollama::chat(&self.http, &self.ollama_base, &self.gemma_model, system, user).await
            }
            ModelKind::Claude => {
                let Some(key) = &self.anthropic_key else {
                    bail!("ANTHROPIC_API_KEY nicht gesetzt")
                };
                anthropic::complete(&self.http, key, system, user).await
            }
            ModelKind::Gpt => {
                let Some(key) = &self.openai_key else {
                    bail!("OPENAI_API_KEY nicht gesetzt")
                };
                openai::complete(&self.http, key, system, user).await
            }
        }
    }

    /// Immer lokal (Ghost-Text-Pfad) – nie Cloud, nie Kosten.
    pub async fn complete_local(&self, system: &str, user: &str) -> Result<String> {
        ollama::chat(&self.http, &self.ollama_base, &self.ghost_model, system, user).await
    }

    /// Embedding eines Wiki-Dokuments (Index-Seite).
    pub async fn embed_document(&self, text: &str) -> Option<Vec<f32>> {
        self.embed_with_prefix("search_document: ", text).await
    }

    /// Embedding einer Suchanfrage (Schreibkontext).
    pub async fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        self.embed_with_prefix("search_query: ", text).await
    }

    /// nomic-embed-Modelle erwarten Task-Präfixe; andere Modelle bekommen
    /// den Rohtext. `None`, wenn das Embedding-Modell fehlt.
    async fn embed_with_prefix(&self, prefix: &str, text: &str) -> Option<Vec<f32>> {
        let input = if self.embed_model.starts_with("nomic") {
            format!("{prefix}{text}")
        } else {
            text.to_string()
        };
        ollama::embed(&self.http, &self.ollama_base, &self.embed_model, &input)
            .await
            .ok()
    }

    /// Erreichbarkeits-Check für die lokale Engine (Startup).
    pub async fn local_available(&self) -> bool {
        ollama::ping(&self.http, &self.ollama_base).await
    }
}
