use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use crate::common::{Clip, Transcript};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiRequest {
    pub contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<GeminiSafetySetting>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiContent {
    pub role: String,
    pub parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiPart {
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiGenerationConfig {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub max_output_tokens: Option<i32>,
    #[serde(rename = "responseMimeType")]
    pub response_mime_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiSafetySetting {
    pub category: String,
    pub threshold: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiResponse {
    pub candidates: Vec<GeminiCandidate>,
    #[serde(rename = "promptFeedback")]
    pub prompt_feedback: Option<GeminiPromptFeedback>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiCandidate {
    pub content: GeminiContent,
    #[serde(rename = "finishReason")]
    pub finish_reason: Option<String>,
    pub index: Option<i32>,
    #[serde(rename = "safetyRatings")]
    pub safety_ratings: Option<Vec<GeminiSafetyRating>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiSafetyRating {
    pub category: String,
    pub probability: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiPromptFeedback {
    #[serde(rename = "blockReason")]
    pub block_reason: Option<String>,
    #[serde(rename = "safetyRatings")]
    pub safety_ratings: Option<Vec<GeminiSafetyRating>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ViralPredictionResult {
    pub predicted_views: i64,
    pub predicted_likes: i64,
    pub predicted_shares: i64,
    pub predicted_retention: f64,
    pub viral_score: f64, // 0-100
    pub hook_analysis: String,
    pub retention_curve: Vec<RetentionPoint>,
    pub recommended_hooks: Vec<String>,
    pub optimization_tips: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RetentionPoint {
    pub time_percent: f64,
    pub retention_percent: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContentOptimizationResult {
    pub optimized_title: String,
    pub optimized_caption: String,
    pub hashtags: Vec<String>,
    pub best_posting_time: String,
    pub thumbnail_prompts: Vec<String>,
}

pub async fn predict_viral_potential(
    api_key: &str,
    clip: &Clip,
    transcript: &Transcript,
    platform: &str,
) -> Result<ViralPredictionResult> {
    let client = Client::new();
    let model = "gemini-1.5-pro";
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let transcript_text = transcript.segments.iter()
        .map(|s| s.text.clone())
        .collect::<Vec<_>>()
        .join(" ");

    let clip_text = transcript.segments.iter()
        .filter(|s| s.start >= clip.start_s && s.end <= clip.end_s)
        .map(|s| s.text.clone())
        .collect::<Vec<_>>()
        .join(" ");

    let prompt = build_viral_prediction_prompt(&clip_text, &transcript_text, platform, clip.duration_s);

    let request = GeminiRequest {
        contents: vec![GeminiContent {
            role: "user".to_string(),
            parts: vec![GeminiPart { text: prompt }],
        }],
        generation_config: Some(GeminiGenerationConfig {
            temperature: Some(0.3),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(4096),
            response_mime_type: Some("application/json".to_string()),
        }),
        safety_settings: Some(vec![
            GeminiSafetySetting { category: "HARM_CATEGORY_HARASSMENT".to_string(), threshold: "BLOCK_MEDIUM_AND_ABOVE".to_string() },
            GeminiSafetySetting { category: "HARM_CATEGORY_HATE_SPEECH".to_string(), threshold: "BLOCK_MEDIUM_AND_ABOVE".to_string() },
            GeminiSafetySetting { category: "HARM_CATEGORY_SEXUALLY_EXPLICIT".to_string(), threshold: "BLOCK_MEDIUM_AND_ABOVE".to_string() },
            GeminiSafetySetting { category: "HARM_CATEGORY_DANGEROUS_CONTENT".to_string(), threshold: "BLOCK_MEDIUM_AND_ABOVE".to_string() },
        ]),
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let err = response.text().await?;
        return Err(anyhow::anyhow!("Gemini API error: {}", err));
    }

    let gemini_resp: GeminiResponse = response.json().await?;

    if let Some(candidate) = gemini_resp.candidates.first() {
        if let Some(part) = candidate.content.parts.first() {
            let result: ViralPredictionResult = serde_json::from_str(&part.text)?;
            return Ok(result);
        }
    }

    Err(anyhow::anyhow!("No valid response from Gemini"))
}

pub async fn optimize_content(
    api_key: &str,
    clip: &Clip,
    transcript: &Transcript,
    platform: &str,
) -> Result<ContentOptimizationResult> {
    let client = Client::new();
    let model = "gemini-1.5-pro";
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let clip_text = transcript.segments.iter()
        .filter(|s| s.start >= clip.start_s && s.end <= clip.end_s)
        .map(|s| s.text.clone())
        .collect::<Vec<_>>()
        .join(" ");

    let prompt = build_content_optimization_prompt(&clip_text, platform, clip.duration_s);

    let request = GeminiRequest {
        contents: vec![GeminiContent {
            role: "user".to_string(),
            parts: vec![GeminiPart { text: prompt }],
        }],
        generation_config: Some(GeminiGenerationConfig {
            temperature: Some(0.5),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(2048),
            response_mime_type: Some("application/json".to_string()),
        }),
        safety_settings: None,
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let err = response.text().await?;
        return Err(anyhow::anyhow!("Gemini API error: {}", err));
    }

    let gemini_resp: GeminiResponse = response.json().await?;

    if let Some(candidate) = gemini_resp.candidates.first() {
        if let Some(part) = candidate.content.parts.first() {
            let result: ContentOptimizationResult = serde_json::from_str(&part.text)?;
            return Ok(result);
        }
    }

    Err(anyhow::anyhow!("No valid response from Gemini"))
}

pub async fn generate_thumbnail_prompts(
    api_key: &str,
    clip: &Clip,
    transcript: &Transcript,
    style: &str,
) -> Result<Vec<String>> {
    let client = Client::new();
    let model = "gemini-1.5-pro";
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let clip_text = transcript.segments.iter()
        .filter(|s| s.start >= clip.start_s && s.end <= clip.end_s)
        .map(|s| s.text.clone())
        .collect::<Vec<_>>()
        .join(" ");

    let prompt = build_thumbnail_prompt(&clip_text, style, clip.duration_s);

    let request = GeminiRequest {
        contents: vec![GeminiContent {
            role: "user".to_string(),
            parts: vec![GeminiPart { text: prompt }],
        }],
        generation_config: Some(GeminiGenerationConfig {
            temperature: Some(0.7),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(1024),
            response_mime_type: Some("application/json".to_string()),
        }),
        safety_settings: None,
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let err = response.text().await?;
        return Err(anyhow::anyhow!("Gemini API error: {}", err));
    }

    let gemini_resp: GeminiResponse = response.json().await?;

    if let Some(candidate) = gemini_resp.candidates.first() {
        if let Some(part) = candidate.content.parts.first() {
            let result: Vec<String> = serde_json::from_str(&part.text)?;
            return Ok(result);
        }
    }

    Err(anyhow::anyhow!("No valid response from Gemini"))
}

fn build_viral_prediction_prompt(clip_text: &str, full_transcript: &str, platform: &str, duration: f64) -> String {
    format!(
        r#"You are a viral content analyst for {}. Analyze this short-form video content and predict its viral potential.

Platform: {}
Duration: {:.1}s
Full Video Context: {}
Clip Content: {}

Return ONLY valid JSON with this exact structure:
{{
  "predicted_views": number,
  "predicted_likes": number,
  "predicted_shares": number,
  "predicted_retention": number (0-1),
  "viral_score": number (0-100),
  "hook_analysis": "detailed analysis of the hook in first 3 seconds",
  "retention_curve": [
    {{"time_percent": 0, "retention_percent": 100}},
    {{"time_percent": 25, "retention_percent": 85}},
    {{"time_percent": 50, "retention_percent": 70}},
    {{"time_percent": 75, "retention_percent": 55}},
    {{"time_percent": 100, "retention_percent": 40}}
  ],
  "recommended_hooks": ["hook1", "hook2", "hook3"],
  "optimization_tips": ["tip1", "tip2", "tip3"]
}}

Consider: hook strength (first 3s), pacing, emotional arc, platform norms, completion rate factors, shareability, comment-worthiness."#,
        platform, platform, duration, full_transcript, clip_text
    )
}

fn build_content_optimization_prompt(clip_text: &str, platform: &str, duration: f64) -> String {
    format!(
        r#"You are a viral content optimizer for {}. Optimize this content for maximum engagement.

Platform: {}
Duration: {:.1}s
Content: {}

Return ONLY valid JSON:
{{
  "optimized_title": "catchy title under 100 chars",
  "optimized_caption": "engaging caption with CTA",
  "hashtags": ["hashtag1", "hashtag2", "hashtag3", "hashtag4", "hashtag5"],
  "best_posting_time": "HH:MM (24h format, timezone-aware)",
  "thumbnail_prompts": ["prompt1", "prompt2", "prompt3"]
}}

Platform-specific optimization:
- TikTok: trending sounds, challenges, duet/stitch hooks, 3-5 hashtags
- Instagram Reels: aesthetic, story-telling, 10-15 hashtags, carousel-friendly
- YouTube Shorts: search-optimized, high CTR thumbnail, 3-5 hashtags, series potential

Make it actionable and platform-native."#,
        platform, platform, duration, clip_text
    )
}

fn build_thumbnail_prompt(clip_text: &str, style: &str, duration: f64) -> String {
    format!(
        r#"Generate 5 thumbnail prompts for {} style thumbnail.

Video content: {}
Duration: {:.1}s
Style: {}

Return ONLY valid JSON array of 5 prompt strings:
["prompt1", "prompt2", "prompt3", "prompt4", "prompt5"]

Each prompt should be detailed for AI image generation (Midjourney/DALL-E style):
- Composition, lighting, colors
- Text placement suggestions
- Emotional tone
- Platform-optimized (9:16 for Shorts/Reels, 16:9 for YouTube)

Styles:
- clickbait: high CTR, bold text, arrows, shock, curiosity gap
- clean: minimal, professional, brand-consistent
- before_after: split screen, dramatic contrast
- emotional: expressive face, story moment
- educational: value-first, clear benefit"#,
        style, clip_text, duration, style
    )
}