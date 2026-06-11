//! OpenAI Chat Completions API (raw HTTP).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

pub const MODEL: &str = "gpt-4o";

pub async fn complete(
    http: &reqwest::Client,
    api_key: &str,
    system: &str,
    user: &str,
) -> Result<String> {
    let body = json!({
        "model": MODEL,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    let resp = http
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("OpenAI API nicht erreichbar: {e}"))?;

    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        bail!(
            "OpenAI API {status}: {}",
            v["error"]["message"].as_str().unwrap_or("unbekannter Fehler")
        );
    }
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| anyhow!("OpenAI API: leere Antwort"))
}
