//! Anthropic Messages API (raw HTTP – es gibt kein offizielles Rust-SDK).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

pub const MODEL: &str = "claude-opus-4-8";

pub async fn complete(
    http: &reqwest::Client,
    api_key: &str,
    system: &str,
    user: &str,
) -> Result<String> {
    let body = json!({
        "model": MODEL,
        "max_tokens": 8192,
        "system": system,
        "messages": [{"role": "user", "content": user}],
    });
    let resp = http
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("Anthropic API nicht erreichbar: {e}"))?;

    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        bail!(
            "Anthropic API {status}: {}",
            v["error"]["message"].as_str().unwrap_or("unbekannter Fehler")
        );
    }
    // stop_reason vor content prüfen – ein Refusal hat ggf. leeren content.
    if v["stop_reason"].as_str() == Some("refusal") {
        bail!("Claude hat die Anfrage abgelehnt (refusal)");
    }
    let text: String = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if text.is_empty() {
        bail!("Anthropic API: leere Antwort");
    }
    Ok(text.trim().to_string())
}
