//! Lokale Inferenz über Ollama (llama.cpp-basiert, nutzt CUDA/Metal
//! automatisch). Komplett offline – Standard-Pfad des Editors.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub async fn ping(http: &reqwest::Client, base: &str) -> bool {
    http.get(format!("{base}/api/tags"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

pub async fn chat(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String> {
    let body = json!({
        "model": model,
        "stream": false,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    let resp = http
        .post(format!("{base}/api/chat"))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("Ollama nicht erreichbar: {e}"))?;
    if !resp.status().is_success() {
        bail!("Ollama HTTP {}: {}", resp.status(), resp.text().await.unwrap_or_default());
    }
    let v: Value = resp.json().await?;
    v["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| anyhow!("Ollama: unerwartetes Antwortformat"))
}

pub async fn embed(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    text: &str,
) -> Result<Vec<f32>> {
    let body = json!({"model": model, "prompt": text});
    let resp = http
        .post(format!("{base}/api/embeddings"))
        .timeout(Duration::from_secs(30))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("Ollama embeddings HTTP {}", resp.status());
    }
    let v: Value = resp.json().await?;
    let arr = v["embedding"]
        .as_array()
        .ok_or_else(|| anyhow!("kein Embedding in Antwort"))?;
    Ok(arr.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect())
}
