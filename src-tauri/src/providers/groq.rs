use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroqModel {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroqModelsResponse {
    pub object: String,
    pub data: Vec<GroqModel>,
}

/// Fetch available models from Groq
pub async fn fetch_groq_models(key: &str) -> Result<Vec<GroqModel>> {
    let client = Client::new();
    let resp = client
        .get("https://api.groq.com/openai/v1/models")
        .bearer_auth(key)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow::anyhow!("Groq models list failed ({}): {}", status, text));
    }

    let models_resp: GroqModelsResponse = serde_json::from_str(&text)?;
    Ok(models_resp.data)
}

/// Get preferred Groq models for script generation (JSON output, good reasoning)
/// Priority: non-preview, stable, JSON mode support
pub fn preferred_groq_models(all_models: &[GroqModel]) -> Vec<String> {
    let preferred = [
        "llama-3.1-8b-instant",       // Fast, stable, JSON mode
        "llama-3.2-3b-preview",       // Small, fast preview
        "llama-3.2-90b-text-preview",  // Large preview (text only)
        "qwen-2.5-72b-instruct",      // Strong reasoning
        "gemma2-9b-it",               // Good alternative
        "openai/gpt-oss-20b",         // New open models
        "openai/gpt-oss-120b",        // New open models (large)
        "mixtral-8x7b-32768",         // Legacy but still available
    ];

    let available: std::collections::HashSet<&str> = all_models.iter().map(|m| m.id.as_str()).collect();
    preferred.iter()
        .filter(|m| available.contains(*m))
        .map(|s| s.to_string())
        .collect()
}
