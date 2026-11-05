use super::ScriptToVideoPlan;
use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

/// All available LLM keys. Any field can be None if not configured.
pub struct LlmKeys {
    pub groq_key: Option<String>,
    pub gemini_key: Option<String>,
}

// Model information structures re-exported from providers
pub use crate::providers::groq::{GroqModel, GroqModelsResponse, preferred_groq_models};
pub use crate::providers::gemini::{GeminiModel, GeminiModelsResponse, preferred_gemini_models};

/// Cached model lists to avoid repeated API calls
#[derive(Debug, Default)]
pub struct ModelCache {
    groq: Option<Vec<GroqModel>>,
    gemini: Option<Vec<GeminiModel>>,
    groq_fetched: bool,
    gemini_fetched: bool,
}

impl ModelCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_groq(&self) -> Option<&Vec<GroqModel>> {
        self.groq.as_ref()
    }

    pub fn set_groq(&mut self, models: Vec<GroqModel>) {
        self.groq = Some(models);
        self.groq_fetched = true;
    }

    pub fn get_gemini(&self) -> Option<&Vec<GeminiModel>> {
        self.gemini.as_ref()
    }

    pub fn set_gemini(&mut self, models: Vec<GeminiModel>) {
        self.gemini = Some(models);
        self.gemini_fetched = true;
    }

    pub fn is_groq_fetched(&self) -> bool {
        self.groq_fetched
    }

    pub fn is_gemini_fetched(&self) -> bool {
        self.gemini_fetched
    }
}

lazy_static::lazy_static! {
    static ref MODEL_CACHE: tokio::sync::Mutex<ModelCache> = tokio::sync::Mutex::new(ModelCache::new());
}

/// Fetch and cache available models from Groq
pub async fn fetch_groq_models(key: &str) -> Result<Vec<GroqModel>> {
    let mut cache = MODEL_CACHE.lock().await;
    if cache.is_groq_fetched() {
        if let Some(models) = cache.get_groq() {
            return Ok(models.clone());
        }
    }

    let models = crate::providers::groq::fetch_groq_models(key).await?;
    cache.set_groq(models.clone());
    Ok(models)
}

/// Fetch and cache available models from Gemini
pub async fn fetch_gemini_models(key: &str) -> Result<Vec<GeminiModel>> {
    let mut cache = MODEL_CACHE.lock().await;
    if cache.is_gemini_fetched() {
        if let Some(models) = cache.get_gemini() {
            return Ok(models.clone());
        }
    }

    let models = crate::providers::gemini::fetch_gemini_models(key).await?;
    cache.set_gemini(models.clone());
    Ok(models)
}

fn build_prompt(language: &str, raw_script: &str) -> String {
    format!(
        r#"You are a world-class viral short-form video editor (TikTok/YouTube Shorts/Instagram Reels).
You understand hook psychology, retention curves, and pattern interrupts.

MISSION: Split this script into scenes that will hook viewers in 0–3 s and keep them to the end.

EDITING RULES — follow strictly:
1. Scene 0 (HOOK): Must state a bold claim, question, or shocking fact. Keep text short (4–8 words max). Visual must be high-energy (camera: push_in, mood: intense or shocking).
2. Every scene is 3–8 seconds. Split long lines. Never one scene longer than 8 s.
3. Alternate camera moves — never two "static" in a row. Use push_in for emotional peaks, pan for reveals, static for dialogue.
4. Use "text_card" for key stats, quotes, or punchy one-liners. They perform well on mobile.
5. pacing_role: tag each scene with one of: hook | build | peak | payoff | cta
6. sfx_hint: one word for a sound effect that fits the mood (e.g. whoosh, ding, bass_drop, crowd, silence, clock_tick). Editor uses this — it is NOT rendered automatically.
7. Last scene: must be a clear CTA or a curiosity gap that keeps them watching more.
8. visual_keywords: be SPECIFIC — not "person thinking" but "focused entrepreneur at laptop late night neon".
9. Language of the "text" field: EXACTLY match the original script language ({language}).

OUTPUT FORMAT: valid JSON only. No markdown fences, no explanation, no comments.

Schema:
{{
  "scenes": [
    {{
      "text": "exact spoken text for this scene",
      "duration_hint": 4.0,
      "visual_keywords": ["specific keyword 1", "specific keyword 2"],
      "visual_type": "stock_video",
      "camera": "push_in",
      "mood": "intense",
      "pacing_role": "hook",
      "sfx_hint": "bass_drop"
    }}
  ]
}}

visual_type options: stock_video | stock_image | text_card
camera options: push_in | pan | static
mood options: intense | calm | mysterious | energetic | emotional | shocking | funny | inspiring
pacing_role options: hook | build | peak | payoff | cta

Script language: {language}

RAW SCRIPT:
{raw_script}
"#,
        language = language,
        raw_script = raw_script
    )
}

fn parse_plan(text: &str, language: &str) -> Result<ScriptToVideoPlan> {
    let clean = text
        .trim()
        .trim_start_matches("```json")
        .trim_end_matches("```")
        .trim();
    let mut plan: ScriptToVideoPlan =
        serde_json::from_str(clean).map_err(|e| anyhow::anyhow!("JSON Parse Error: {}", e))?;
    plan.language = language.to_string();
    Ok(plan)
}

// ── Provider 1: Groq — tries models in order of availability ─────────────────
async fn try_groq(key: &str, prompt: &str) -> Result<String> {
    let client = Client::new();
    
    // First, fetch available models if not cached
    let models = fetch_groq_models(key).await.unwrap_or_default();
    let preferred = preferred_groq_models(&models);
    
    if preferred.is_empty() {
        // Fallback to known models if fetch failed
        let fallback = [
            "llama-3.1-8b-instant",
            "llama-3.2-3b-preview",
            "qwen-2.5-72b-instruct",
            "gemma2-9b-it",
            "openai/gpt-oss-20b",
            "openai/gpt-oss-120b",
            "mixtral-8x7b-32768",
        ];
        
        for model in &fallback {
            if let Ok(result) = try_groq_model(&client, key, prompt, model).await {
                return Ok(result);
            }
        }
        return Err(anyhow::anyhow!("All Groq models failed. Check your key at https://console.groq.com/keys"));
    }

    for model in &preferred {
        if let Ok(result) = try_groq_model(&client, key, prompt, model).await {
            return Ok(result);
        }
    }

    Err(anyhow::anyhow!("All preferred Groq models failed. Check your key at https://console.groq.com/keys"))
}

async fn try_groq_model(client: &Client, key: &str, prompt: &str, model: &str) -> Result<String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.4,
        "max_tokens": 4096,
        "response_format": {"type": "json_object"}
    });

    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .bearer_auth(key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if status.as_u16() == 404 {
        let _ = resp.text().await;
        warn!("[ScriptGen] Groq model {} not found (404), trying next…", model);
        return Err(anyhow::anyhow!("Model not found"));
    }
    if status.as_u16() == 401 {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Groq authentication failed (401): {}", text));
    }
    if status.as_u16() == 429 {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Groq rate limited (429): {}", text));
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Groq API error ({} {}): {}", status.as_u16(), model, text));
    }

    let json: serde_json::Value = resp.json().await?;
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Empty response from Groq ({})", model))?
        .to_string();
    info!("[ScriptGen] Groq model {} ✓", model);
    Ok(content)
}

// ── Provider 2: Gemini — tries available models in order ─────────────────────
async fn try_gemini(key: &str, prompt: &str) -> Result<String> {
    let client = Client::new();
    let req = crate::gemini::GeminiRequest {
        contents: vec![crate::gemini::GeminiContent {
            role: "user".to_string(),
            parts: vec![crate::gemini::GeminiPart {
                text: prompt.to_string(),
            }],
        }],
        generation_config: Some(crate::gemini::GeminiGenerationConfig {
            temperature: Some(0.4),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(4096),
            response_mime_type: Some("application/json".to_string()),
        }),
        safety_settings: None,
    };

    let models = fetch_gemini_models(key).await.unwrap_or_default();
    let preferred = preferred_gemini_models(&models);

    for model in &preferred {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model, key
        );
        let resp = client.post(&url).json(&req).send().await?;
        let status = resp.status();
        if status.as_u16() == 429 {
            let _ = resp.text().await;
            warn!("[ScriptGen] Gemini {} quota exceeded (429), trying next model…", model);
            continue;
        }
        if status.as_u16() == 404 {
            let _ = resp.text().await;
            warn!("[ScriptGen] Gemini {} not found (404), trying next model…", model);
            continue;
        }
        if status.as_u16() == 401 {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Gemini authentication failed (401): {}", text));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Gemini API error ({}) on {}: {}",
                status, model, text
            ));
        }
        let gemini_resp: crate::gemini::GeminiResponse = resp.json().await?;
        let part_text = gemini_resp
            .candidates
            .first()
            .and_then(|c| c.content.parts.first())
            .map(|p| p.text.as_str())
            .ok_or_else(|| anyhow::anyhow!("Empty response from Gemini ({})", model))?
            .to_string();
        info!("[ScriptGen] Gemini model {} ✓", model);
        return Ok(part_text);
    }

    // Fallback to known models if fetch failed
    let fallback = ["gemini-1.5-flash", "gemini-1.5-flash-8b", "gemini-2.5-flash"];
    for model in &fallback {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model, key
        );
        let resp = client.post(&url).json(&req).send().await?;
        let status = resp.status();
        if status.as_u16() == 429 {
            let _ = resp.text().await;
            continue;
        }
        if status.as_u16() == 404 {
            let _ = resp.text().await;
            continue;
        }
        if !status.is_success() {
            continue;
        }
        let gemini_resp: crate::gemini::GeminiResponse = resp.json().await?;
        let part_text = gemini_resp
            .candidates
            .first()
            .and_then(|c| c.content.parts.first())
            .map(|p| p.text.as_str())
            .ok_or_else(|| anyhow::anyhow!("Empty response from Gemini ({})", model))?
            .to_string();
        info!("[ScriptGen] Gemini model {} ✓", model);
        return Ok(part_text);
    }

    Err(anyhow::anyhow!(
        "Gemini quota exceeded or all models unavailable. Add Groq key at https://console.groq.com/keys"
    ))
}

// ── Provider 3: Ollama — local, last resort ───────────────────────────────────
async fn try_ollama(prompt: &str) -> Result<String> {
    let client = Client::new();
    let ollama_url =
        std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());

    // Quick reachability check (3 s timeout)
    let reachable = client
        .get(format!("{}/api/tags", ollama_url))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    if !reachable {
        // Not an error, just not available
        return Err(anyhow::anyhow!("Ollama not running (skipped)"));
    }

    let request = serde_json::json!({
        "model": "qwen2.5:7b-instruct-q4_K_M",
        "prompt": prompt,
        "stream": false,
        "format": "json",
        "options": {"temperature": 0.4, "top_p": 0.9, "top_k": 40, "num_predict": 4096}
    });

    let resp = client
        .post(format!("{}/api/generate", ollama_url))
        .json(&request)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Ollama error ({}): {}", status, text));
    }

    let json: serde_json::Value = resp.json().await?;
    let content = json["response"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Empty response from Ollama"))?
        .to_string();
    Ok(content)
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Generate a scene plan. Fallback order: Groq → Gemini → Ollama → Error
pub async fn generate_script_plan(
    keys: &LlmKeys,
    raw_script: &str,
    language: &str,
) -> Result<ScriptToVideoPlan> {
    let prompt = build_prompt(language, raw_script);
    let mut errors: Vec<String> = Vec::new();

    // 1. Groq — generous free tier, very unlikely to 429 on personal use
    if let Some(ref key) = keys.groq_key {
        info!("[ScriptGen] Trying Groq (key: {}…)", &key[..key.len().min(8)]);
        match try_groq(key, &prompt).await {
            Ok(text) => {
                info!("[ScriptGen] Used Groq ✓");
                return parse_plan(&text, language);
            }
            Err(e) => {
                let msg = format!("Groq: {}", e);
                warn!("[ScriptGen] {}", msg);
                errors.push(msg);
            }
        }
    } else {
        errors.push("Groq: no key configured".to_string());
    }

    // 2. Gemini
    if let Some(ref key) = keys.gemini_key {
        info!("[ScriptGen] Trying Gemini (key: {}…)", &key[..key.len().min(8)]);
        match try_gemini(key, &prompt).await {
            Ok(text) => {
                info!("[ScriptGen] Used Gemini ✓");
                return parse_plan(&text, language);
            }
            Err(e) => {
                let msg = format!("Gemini: {}", e);
                warn!("[ScriptGen] {}", msg);
                errors.push(msg);
            }
        }
    } else {
        errors.push("Gemini: no key configured".to_string());
    }

    // 3. Ollama — local, slow but always free (not an error if not running)
    info!("[ScriptGen] Trying Ollama…");
    match try_ollama(&prompt).await {
        Ok(text) => {
            info!("[ScriptGen] Used Ollama ✓");
            return parse_plan(&text, language);
        }
        Err(e) => {
            let msg = format!("Ollama: {}", e);
            warn!("[ScriptGen] {}", msg);
            // Don't add to errors if it's just not running
            if !msg.contains("not running") {
                errors.push(msg);
            }
        }
    }

    // Build a helpful error message
    let error_details = errors.join("\n");
    Err(anyhow::anyhow!(
        "All LLM providers failed:\n{}\n\nFixes:\n- Groq: Add a free key at https://console.groq.com/keys (model 404 = model retired, not key issue)\n- Gemini: Add a key at https://aistudio.google.com/apikey (quota 429 = wait or use Groq)\n- Ollama: Start with `ollama serve` (optional, local fallback)",
        error_details
    ))
}

/// Refresh model lists from all providers (called from Settings)
pub async fn refresh_model_lists(
    groq_key: Option<&str>,
    gemini_key: Option<&str>,
) -> Result<()> {
    let mut cache = MODEL_CACHE.lock().await;
    
    if let Some(key) = groq_key {
        if let Ok(models) = fetch_groq_models(key).await {
            cache.set_groq(models);
            info!("[ScriptGen] Refreshed Groq models: {} available", cache.get_groq().map(|m| m.len()).unwrap_or(0));
        }
    }
    
    if let Some(key) = gemini_key {
        if let Ok(models) = fetch_gemini_models(key).await {
            cache.set_gemini(models);
            info!("[ScriptGen] Refreshed Gemini models: {} available", cache.get_gemini().map(|m| m.len()).unwrap_or(0));
        }
    }
    
    Ok(())
}

/// Get cached model lists for UI display
pub async fn get_cached_models() -> (Option<Vec<GroqModel>>, Option<Vec<GeminiModel>>) {
    let cache = MODEL_CACHE.lock().await;
    (cache.get_groq().cloned(), cache.get_gemini().cloned())
}

/// Test a provider connection
pub async fn test_provider_connection(
    provider: &str,
    key: &str,
) -> Result<()> {
    match provider {
        "groq" => {
            let client = Client::new();
            let resp = client
                .get("https://api.groq.com/openai/v1/models")
                .bearer_auth(key)
                .send()
                .await?;
            if resp.status().as_u16() == 401 {
                return Err(anyhow::anyhow!("Invalid Groq API key"));
            }
            if !resp.status().is_success() {
                return Err(anyhow::anyhow!("Groq connection test failed: {}", resp.status()));
            }
            Ok(())
        }
        "gemini" => {
            let client = Client::new();
            let resp = client
                .get(&format!("https://generativelanguage.googleapis.com/v1beta/models?key={}", key))
                .send()
                .await?;
            if resp.status().as_u16() == 401 {
                return Err(anyhow::anyhow!("Invalid Gemini API key"));
            }
            if !resp.status().is_success() {
                return Err(anyhow::anyhow!("Gemini connection test failed: {}", resp.status()));
            }
            Ok(())
        }
        "ollama" => {
            let ollama_url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());
            let client = Client::new();
            let resp = client
                .get(format!("{}/api/tags", ollama_url))
                .timeout(std::time::Duration::from_secs(3))
                .send()
                .await?;
            if !resp.status().is_success() {
                return Err(anyhow::anyhow!("Ollama not responding"));
            }
            Ok(())
        }
        _ => Err(anyhow::anyhow!("Unknown provider: {}", provider)),
    }
}
