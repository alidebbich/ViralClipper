use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use crate::common::{Clip, Transcript, OllamaTranscript};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaRequest {
    pub model: String,
    pub prompt: String,
    pub stream: bool,
    pub format: Option<String>,
    pub options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaOptions {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub num_predict: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaResponse {
    pub model: String,
    pub created_at: String,
    pub response: String,
    pub done: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RerankResult {
    pub clip_id: String,
    pub score: f64,
    pub reason: String,
    pub suggested_title: String,
    pub suggested_hook: String,
}

pub async fn rerank_clips(
    clips: &[Clip],
    transcript: &OllamaTranscript,
) -> Result<Vec<Clip>> {
    // Build context for LLM
    let channel_niche = "general"; // Could be user-configurable
    let audience = "general social media";

    let clips_json = serde_json::to_string(clips)?;

    let prompt = build_rerank_prompt(&clips_json, &transcript.text, channel_niche, audience);

    let results = call_ollama_rerank(&prompt).await?;

    // Apply rerank results to clips
    let mut reranked = clips.to_vec();
    for result in results {
        if let Some(clip) = reranked.iter_mut().find(|c| c.id == result.clip_id) {
            clip.viral_score = Some(result.score);
            clip.llm_reason = Some(result.reason);
            clip.title = result.suggested_title;
            // Store hook separately if needed
        }
    }

    // Sort by viral score descending
    reranked.sort_by(|a, b| {
        b.viral_score.unwrap_or(0.0)
            .partial_cmp(&a.viral_score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(reranked)
}

fn build_rerank_prompt(
    clips_json: &str,
    transcript: &str,
    niche: &str,
    audience: &str,
) -> String {
    format!(
        r#"You are a Shorts retention editor. Rank these clips by viral potential.

Context:
- Channel niche: {}
- Audience: {}
- Full transcript: {}

Clips to rank (JSON):
{}

Return ONLY valid JSON array with this exact structure for each clip:
[
  {{
    "clip_id": "string",
    "score": number (1-10),
    "reason": "one-line reason",
    "suggested_title": "catchy title under 50 chars",
    "suggested_hook": "first 3 seconds hook text"
  }}
]

Rank top to bottom. Be decisive."#,
        niche, audience, transcript, clips_json
    )
}

async fn call_ollama_rerank(prompt: &str) -> Result<Vec<RerankResult>> {
    let client = Client::new();
    let ollama_url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());

    let request = OllamaRequest {
        model: "qwen2.5:7b-instruct-q4_K_M".to_string(),
        prompt: prompt.to_string(),
        stream: false,
        format: Some("json".to_string()),
        options: Some(OllamaOptions {
            temperature: Some(0.3),
            top_p: Some(0.9),
            top_k: Some(40),
            num_predict: Some(1024),
        }),
    };

    let response = client
        .post(format!("{}/api/generate", ollama_url))
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let err = response.text().await?;
        return Err(anyhow::anyhow!("Ollama error: {}", err));
    }

    let ollama_resp: OllamaResponse = response.json().await?;

    // Parse JSON response
    let results: Vec<RerankResult> = serde_json::from_str(&ollama_resp.response)?;

    Ok(results)
}

pub async fn ensure_model_available() -> Result<()> {
    let client = Client::new();
    let ollama_url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());

    let response = client
        .get(format!("{}/api/tags", ollama_url))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to list Ollama models"));
    }

    let json: serde_json::Value = response.json().await?;
    let default_vec = vec![];
    let models = json["models"].as_array().unwrap_or(&default_vec);
    let model_exists = models.iter().any(|m| {
        m["name"].as_str().unwrap_or("").contains("qwen2.5:7b")
    });

    if !model_exists {
        println!("Pulling qwen2.5:7b-instruct-q4_K_M...");
        let pull_request = OllamaRequest {
            model: "qwen2.5:7b-instruct-q4_K_M".to_string(),
            prompt: "".to_string(),
            stream: false,
            format: None,
            options: None,
        };

        let _ = client
            .post(format!("{}/api/pull", ollama_url))
            .json(&pull_request)
            .send()
            .await?;
    }

    Ok(())
}

pub async fn optimize_clip_content(
    clip: &Clip,
    transcript: &Transcript,
) -> Result<(String, String)> {
    let client = Client::new();
    let ollama_url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());

    let prompt = format!(
        r#"You are a viral content optimizer. Create an engaging title and hook for this Short clip.

Clip duration: {}s
Clip transcript: {}

Return JSON: {{"title": "...", "hook": "..."}}"#,
        clip.duration_s,
        transcript.segments.iter()
            .filter(|s| s.start >= clip.start_s && s.end <= clip.end_s)
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join(" ")
    );

    let request = OllamaRequest {
        model: "qwen2.5:7b-instruct-q4_K_M".to_string(),
        prompt,
        stream: false,
        format: Some("json".to_string()),
        options: Some(OllamaOptions {
            temperature: Some(0.5),
            top_p: Some(0.9),
            top_k: Some(40),
            num_predict: Some(256),
        }),
    };

    let response = client
        .post(format!("{}/api/generate", ollama_url))
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        return Ok((clip.title.clone(), "Watch this!".to_string()));
    }

    let ollama_resp: OllamaResponse = response.json().await?;
    let metadata: serde_json::Value = serde_json::from_str(&ollama_resp.response)?;

    let title = metadata["title"].as_str().unwrap_or(&clip.title).to_string();
    let hook = metadata["hook"].as_str().unwrap_or("Watch this!").to_string();

    Ok((title, hook))
}