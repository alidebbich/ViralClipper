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

#[derive(Debug, Deserialize)]
struct HighlightExtraction {
    pub start_time: f64,
    pub end_time: f64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub viral_score: f64,
    #[serde(default)]
    pub hook_score: f64,
    #[serde(default)]
    pub hook_analysis: String,
    #[serde(default)]
    pub strengths: String,
    #[serde(default)]
    pub weaknesses: String,
    #[serde(default)]
    pub how_to_improve: String,
    #[serde(default)]
    pub suggested_template: String,
    #[serde(default)]
    pub montage_segments: Option<Vec<Vec<f64>>>,
}

// Model information structures
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GeminiModel {
    pub name: String,
    pub base_model_id: Option<String>,
    pub version: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    pub input_token_limit: Option<i32>,
    pub output_token_limit: Option<i32>,
    pub supported_generation_methods: Vec<String>,
    pub temperature: Option<f32>,
    pub max_temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GeminiModelsResponse {
    pub models: Vec<GeminiModel>,
    pub next_page_token: Option<String>,
}

pub async fn find_viral_highlights(
    api_key: &str,
    transcript: &Transcript,
    max_clips: usize,
) -> Result<Vec<Clip>> {
    let client = Client::new();
    let model = "gemini-2.5-flash";
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    // Build full transcript text with timestamps
    let mut full_text = String::new();
    for seg in &transcript.segments {
        full_text.push_str(&format!("[{:.1}s - {:.1}s] {}\n", seg.start, seg.end, seg.text.trim()));
    }

    let prompt = format!(
        r#"You are an elite viral video strategist for TikTok and YouTube Shorts.
I am providing you the FULL transcript of a video, with timestamps.
Read the entire transcript, understand the actual concept/story, and extract the {} BEST viral moments.
Do NOT just pick random talking. Pick moments that have a clear hook, a buildup, and a payoff.
- Choose the parts to clip wisely. The part that can go viral, that can be cool. Think as a professional editor.
- Every video needs some creative design. Suggest a creative style/template for this specific clip ("tiktok", "hormozi", or "minimal"). Use variety across the clips!
- IMPORTANT: Make AT LEAST ONE of your clips a "Summary Montage". For this clip only, pick multiple key parts across the video and put them in `montage_segments` (e.g. `[[2.5, 10.0], [15.0, 25.0]]`).

Each clip must be between 10 and 90 seconds long.
CRITICAL: Do NOT extract the exact same timestamps for multiple clips. Every clip MUST cover a different segment or be a unique montage. If the video is too short to provide {} unique clips, just return as many unique clips as possible.
Do not let standard clips overlap.

Transcript:
{}

Return ONLY a valid JSON array of objects with this exact format:
[
  {{
    "start_time": 12.5,
    "end_time": 75.0,
    "title": "Short punchy title (ALL CAPS)",
    "viral_score": 8.5,
    "hook_score": 0.88,
    "hook_analysis": "Opens with an intriguing question that demands an answer.",
    "strengths": "Fast pacing and immediately relatable dilemma.",
    "weaknesses": "Lulls slightly around second 15.",
    "how_to_improve": "Use a zoom punch on the key revelation.",
    "suggested_template": "hormozi",
    "montage_segments": [[12.5, 22.0], [40.0, 50.0], [60.0, 75.0]]
  }}
] "#,
        max_clips, max_clips, full_text
    );

    let request = GeminiRequest {
        contents: vec![GeminiContent {
            role: "user".to_string(),
            parts: vec![GeminiPart { text: prompt }],
        }],
        generation_config: Some(GeminiGenerationConfig {
            temperature: Some(0.4),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(4096),
            response_mime_type: Some("application/json".to_string()),
        }),
        safety_settings: None,
    };

    let response = client.post(&url).json(&request).send().await?;

    if !response.status().is_success() {
        let err = response.text().await?;
        return Err(anyhow::anyhow!("Gemini API error: {}", err));
    }

    let gemini_resp: GeminiResponse = response.json().await?;
    let part_text = gemini_resp.candidates.first()
        .and_then(|c| c.content.parts.first())
        .map(|p| p.text.as_str())
        .ok_or_else(|| anyhow::anyhow!("Empty response from Gemini"))?;

    let mut clean_text = part_text.trim();
    if clean_text.starts_with("```json") {
        clean_text = clean_text.trim_start_matches("```json");
    } else if clean_text.starts_with("```") {
        clean_text = clean_text.trim_start_matches("```");
    }
    if clean_text.ends_with("```") {
        clean_text = clean_text.trim_end_matches("```");
    }
    clean_text = clean_text.trim();

    let extractions: Vec<HighlightExtraction> = serde_json::from_str(clean_text)
        .map_err(|e| anyhow::anyhow!("Failed to parse Gemini JSON: {} (text: {})", e, clean_text))?;

    let mut result_clips = Vec::new();
    for eval in extractions {
        let mut clip = Clip::new(
            "temp".to_string(), // Project ID filled later
            eval.start_time,
            eval.end_time,
            eval.title,
        );
        clip.viral_score = Some(eval.viral_score);
        clip.hook_score = Some(eval.hook_score);
        let feedback = format!(
            "🎯 Hook: {}\n🌟 Strengths: {}\n⚠️ Weaknesses: {}\n💡 Tips: {}",
            eval.hook_analysis.trim(),
            eval.strengths.trim(),
            eval.weaknesses.trim(),
            eval.how_to_improve.trim()
        );
        clip.llm_reason = Some(feedback);
        if !eval.suggested_template.trim().is_empty() {
            clip.template_id = Some(eval.suggested_template.trim().to_lowercase());
        }
        if let Some(segments) = eval.montage_segments {
            if segments.len() > 1 {
                clip.hook_segments = Some(serde_json::to_string(&segments).unwrap_or_default());
            }
        }
        result_clips.push(clip);
    }

    // Snap to nearest segment boundaries
    for clip in &mut result_clips {
        if let Some(closest_start) = transcript.segments.iter().min_by(|a, b| {
            (a.start - clip.start_s).abs().partial_cmp(&(b.start - clip.start_s).abs()).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            if (closest_start.start - clip.start_s).abs() < 5.0 {
                clip.start_s = closest_start.start;
            }
        }
        if let Some(closest_end) = transcript.segments.iter().min_by(|a, b| {
            (a.end - clip.end_s).abs().partial_cmp(&(b.end - clip.end_s).abs()).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            if (closest_end.end - clip.end_s).abs() < 5.0 {
                clip.end_s = closest_end.end;
            }
        }
        clip.duration_s = (clip.end_s - clip.start_s).max(1.0);
    }

    Ok(result_clips)
}

#[derive(Debug, Deserialize)]
struct ClipEvaluation {
    pub index: usize,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub viral_score: f64,
    #[serde(default)]
    pub hook_score: f64,
    #[serde(default)]
    pub hook_analysis: String,
    #[serde(default)]
    pub strengths: String,
    #[serde(default)]
    pub weaknesses: String,
    #[serde(default)]
    pub how_to_improve: String,
    #[serde(default)]
    pub suggested_template: String,
}

pub async fn evaluate_and_score_clips(
    api_key: &str,
    clips: &[Clip],
    transcript: &Transcript,
) -> Result<Vec<Clip>> {
    let client = Client::new();
    let model = "gemini-2.5-flash";
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let mut clips_summary = String::new();
    for (i, clip) in clips.iter().enumerate() {
        let clip_text = transcript.segments.iter()
            .filter(|s| s.start >= clip.start_s && s.end <= clip.end_s)
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join(" ");

        clips_summary.push_str(&format!(
            "Clip {}: [{:.1}s - {:.1}s] (Duration: {:.1}s)\nText: \"{}\"\n\n",
            i, clip.start_s, clip.end_s, clip.duration_s, clip_text
        ));
    }

    let prompt = format!(
        r#"You are an elite viral video strategist for TikTok, Instagram Reels, and YouTube Shorts.
Analyze these {} candidate clips from a video transcript.
For each clip, provide:
1. "index": the clip index (0 to {})
2. "title": a punchy, click-worthy hook title (max 60 chars)
3. "viral_score": score from 1.0 to 10.0 based on virality, retention likelihood, emotion, and rewatchability
4. "hook_score": score from 0.0 to 1.0 evaluating the opening 3 seconds
5. "hook_analysis": 1 concise sentence analyzing why the hook works or fails
6. "strengths": 1-2 concise sentences on key strengths
7. "weaknesses": 1-2 concise sentences on potential drop-off points
8. "how_to_improve": 1-2 actionable tips to maximize engagement
9. "suggested_template": suggest one of ["tiktok", "hormozi", "minimal"] that best fits the vibe. Be creative and vary the templates!

Candidate Clips:
{}

Return ONLY valid JSON array with this exact format:
[
  {{
    "index": 0,
    "title": "Short punchy title",
    "viral_score": 8.5,
    "hook_score": 0.88,
    "hook_analysis": "Opens with an intriguing question that demands an answer.",
    "strengths": "Fast pacing and immediately relatable dilemma.",
    "weaknesses": "Lulls slightly around second 15.",
    "how_to_improve": "Use a zoom punch on the key revelation and end with a curiosity loop.",
    "suggested_template": "minimal"
  }}
]"#,
        clips.len(),
        clips.len().saturating_sub(1),
        clips_summary
    );

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
        safety_settings: None,
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let err = response.text().await?;
        return Err(anyhow::anyhow!("Gemini API error ({}): {}", status, err));
    }

    let gemini_resp: GeminiResponse = response.json().await?;
    let part_text = gemini_resp.candidates.first()
        .and_then(|c| c.content.parts.first())
        .map(|p| p.text.as_str())
        .ok_or_else(|| anyhow::anyhow!("Empty response from Gemini"))?;

    let evaluations: Vec<ClipEvaluation> = serde_json::from_str(part_text)
        .map_err(|e| anyhow::anyhow!("Failed to parse Gemini clip evaluation JSON: {} (text: {})", e, part_text))?;

    let mut result_clips = clips.to_vec();
    for eval in evaluations {
        if let Some(clip) = result_clips.get_mut(eval.index) {
            if !eval.title.trim().is_empty() {
                clip.title = eval.title.trim().to_string();
            }
            clip.viral_score = Some((eval.viral_score * 10.0).round() / 10.0);
            clip.hook_score = Some((eval.hook_score * 100.0).round() / 100.0);

            // Format rich feedback
            let feedback = format!(
                "🎯 Hook: {}\n🌟 Strengths: {}\n⚠️ Weaknesses: {}\n💡 Tips: {}",
                eval.hook_analysis.trim(),
                eval.strengths.trim(),
                eval.weaknesses.trim(),
                eval.how_to_improve.trim()
            );
            clip.llm_reason = Some(feedback);
            if !eval.suggested_template.trim().is_empty() {
                clip.template_id = Some(eval.suggested_template.trim().to_lowercase());
            }
        }
    }

    // Snap boundaries to nearest segments and ensure strictly no overlapping clips
    for clip in &mut result_clips {
        if let Some(closest_start) = transcript.segments.iter().min_by(|a, b| {
            (a.start - clip.start_s).abs().partial_cmp(&(b.start - clip.start_s).abs()).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            if (closest_start.start - clip.start_s).abs() < 2.0 {
                clip.start_s = closest_start.start;
            }
        }
        if let Some(closest_end) = transcript.segments.iter().min_by(|a, b| {
            (a.end - clip.end_s).abs().partial_cmp(&(b.end - clip.end_s).abs()).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            if (closest_end.end - clip.end_s).abs() < 2.0 {
                clip.end_s = closest_end.end;
            }
        }
        clip.duration_s = ((clip.end_s - clip.start_s) * 100.0).round() / 100.0;
    }

    // Eliminate any overlaps by sorting chronologically and keeping non-overlapping
    result_clips.sort_by(|a, b| a.start_s.partial_cmp(&b.start_s).unwrap_or(std::cmp::Ordering::Equal));
    let mut deduped_clips: Vec<Clip> = Vec::new();
    for clip in result_clips {
        let has_overlap = deduped_clips.iter().any(|c| {
            !(clip.end_s <= c.start_s + 1.0 || clip.start_s >= c.end_s - 1.0)
        });
        if !has_overlap {
            deduped_clips.push(clip);
        }
    }

    Ok(deduped_clips)
}

pub async fn predict_viral_potential(
    api_key: &str,
    clip: &Clip,
    transcript: &Transcript,
    platform: &str,
) -> Result<ViralPredictionResult> {
    let client = Client::new();
    let model = "gemini-2.5-flash";
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
    let model = "gemini-2.5-flash";
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
    let model = "gemini-2.5-flash";
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

/// Fetch available models from Gemini
pub async fn fetch_gemini_models(key: &str) -> Result<Vec<GeminiModel>> {
    let client = Client::new();
    let resp = client
        .get(&format!("https://generativelanguage.googleapis.com/v1beta/models?key={}", key))
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow::anyhow!("Gemini models list failed ({}): {}", status, text));
    }

    let models_resp: GeminiModelsResponse = serde_json::from_str(&text)?;
    let generate_models: Vec<GeminiModel> = models_resp.models
        .into_iter()
        .filter(|m| m.supported_generation_methods.iter().any(|s| s == "generateContent"))
        .collect();
    Ok(generate_models)
}

/// Get preferred Gemini models for script generation
pub fn preferred_gemini_models(all_models: &[GeminiModel]) -> Vec<String> {
    let preferred = [
        "gemini-1.5-flash",
        "gemini-1.5-flash-8b",
        "gemini-2.0-flash",
        "gemini-2.5-flash",
        "gemini-1.5-pro",
    ];

    let available: std::collections::HashSet<&str> = all_models.iter().map(|m| {
        m.name.strip_prefix("models/").unwrap_or(&m.name)
    }).collect();

    preferred.iter()
        .filter(|m| available.contains(*m))
        .map(|s| s.to_string())
        .collect()
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
