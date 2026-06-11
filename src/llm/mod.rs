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

#[derive(Clone)]
pub struct Llm {
    http: reqwest::Client,
    pub kind: ModelKind,
    ollama_base: String,
    gemma_model: String,
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
        Self {
            http,
            kind: ModelKind::Gemma,
            ollama_base: std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434".into()),
            gemma_model: std::env::var("VIBE_GEMMA_MODEL").unwrap_or_else(|_| "gemma3".into()),
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
                Ok(format!("Gemma · lokal ({})", self.gemma_model))
            }
            "claude" | "anthropic" => {
                if self.anthropic_key.is_none() {
                    return Err("ANTHROPIC_API_KEY nicht gesetzt".into());
                }
                self.kind = ModelKind::Claude;
                Ok(format!("Claude · Cloud ({})", anthropic::MODEL))
            }
            "gpt" | "openai" | "gpt-4o" => {
                if self.openai_key.is_none() {
                    return Err("OPENAI_API_KEY nicht gesetzt".into());
                }
                self.kind = ModelKind::Gpt;
                Ok(format!("GPT · Cloud ({})", openai::MODEL))
            }
            other => Err(format!("Unbekanntes Modell '{other}' – gemma|claude|gpt")),
        }
    }

    pub fn label(&self) -> String {
        match self.kind {
            ModelKind::Gemma => format!("Gemma · lokal ({})", self.gemma_model),
            ModelKind::Claude => format!("Claude · Cloud ({})", anthropic::MODEL),
            ModelKind::Gpt => format!("GPT · Cloud ({})", openai::MODEL),
        }
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
        ollama::chat(&self.http, &self.ollama_base, &self.gemma_model, system, user).await
    }

    /// Embedding über Ollama; `None` wenn das Embedding-Modell fehlt.
    pub async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        ollama::embed(&self.http, &self.ollama_base, &self.embed_model, text)
            .await
            .ok()
    }

    /// Erreichbarkeits-Check für die lokale Engine (Startup).
    pub async fn local_available(&self) -> bool {
        ollama::ping(&self.http, &self.ollama_base).await
    }
}
