use tauri::{command, State};
use super::{ScriptToVideoPlan, Scene};
use crate::storage::DbPool;
use std::path::PathBuf;

#[command]
pub async fn generate_script_plan_cmd(
    pool: State<'_, DbPool>,
    raw_script: String,
    language: String,
) -> Result<ScriptToVideoPlan, String> {
    // Load Groq key
    let groq_key = std::env::var("GROQ_API_KEY").ok().filter(|k| !k.trim().is_empty())
        .or_else(|| crate::pipeline::get_key_from_env_file("GROQ_API_KEY"))
        .or_else(|| {
            let conn = pool.lock().ok()?;
            crate::storage::get_user_preference_internal(&conn, "groq_api_key")
                .ok()
                .flatten()
                .map(|p| p.value_json.trim_matches('"').to_string())
                .filter(|k| !k.trim().is_empty())
        });

    // Load Gemini key
    let gemini_key = std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty())
        .or_else(|| crate::pipeline::get_key_from_env_file("GEMINI_API_KEY"))
        .or_else(|| {
            let conn = pool.lock().ok()?;
            crate::storage::get_user_preference_internal(&conn, "gemini_api_key")
                .ok()
                .flatten()
                .map(|p| p.value_json.trim_matches('"').to_string())
                .filter(|k| !k.trim().is_empty())
        });

    let keys = super::generator::LlmKeys { groq_key, gemini_key };
    super::generator::generate_script_plan(&keys, &raw_script, &language)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn list_tts_voices_cmd() -> Result<Vec<String>, String> {
    super::tts::list_voices().await.map_err(|e| e.to_string())
}

#[command]
pub async fn list_tts_voices_by_language_cmd(language: String) -> Result<Vec<super::tts::VoiceInfo>, String> {
    super::tts::list_voices_by_language(&language).await.map_err(|e| e.to_string())
}

#[command]
pub async fn preview_tts_voice_cmd(voice: String, text: String) -> Result<String, String> {
    let cache_base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    let out_dir = cache_base.join("clipper").join("tts").join("previews");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let preview_id = uuid::Uuid::new_v4().to_string();
    let out_path = out_dir.join(format!("{}.mp3", preview_id));
    let out_str = out_path.to_string_lossy().to_string();

    super::tts::generate_tts(&text, &voice, &out_str).await.map_err(|e| e.to_string())?;
    Ok(out_str)
}

#[command]
pub async fn generate_scene_voice_cmd(
    text: String,
    voice: String,
    scene_id: String,
) -> Result<String, String> {
    let cache_base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    let out_dir = cache_base.join("clipper").join("tts");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let out_path = out_dir.join(format!("{}.mp3", scene_id));
    let out_str = out_path.to_string_lossy().to_string();

    super::tts::generate_tts(&text, &voice, &out_str).await.map_err(|e| e.to_string())?;
    Ok(out_str)
}

#[command]
pub async fn assemble_video_cmd(plan: ScriptToVideoPlan) -> Result<String, String> {
    super::pipeline::assemble_video(plan).await.map_err(|e| e.to_string())
}

#[command]
pub async fn refresh_models_cmd(
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Load keys from settings
    let groq_key = std::env::var("GROQ_API_KEY").ok().filter(|k| !k.trim().is_empty())
        .or_else(|| crate::pipeline::get_key_from_env_file("GROQ_API_KEY"))
        .or_else(|| {
            let conn = pool.lock().ok()?;
            crate::storage::get_user_preference_internal(&conn, "groq_api_key")
                .ok()
                .flatten()
                .map(|p| p.value_json.trim_matches('"').to_string())
                .filter(|k| !k.trim().is_empty())
        });

    let gemini_key = std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty())
        .or_else(|| crate::pipeline::get_key_from_env_file("GEMINI_API_KEY"))
        .or_else(|| {
            let conn = pool.lock().ok()?;
            crate::storage::get_user_preference_internal(&conn, "gemini_api_key")
                .ok()
                .flatten()
                .map(|p| p.value_json.trim_matches('"').to_string())
                .filter(|k| !k.trim().is_empty())
        });

    super::generator::refresh_model_lists(groq_key.as_deref(), gemini_key.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn get_cached_models_cmd() -> Result<(Option<Vec<super::generator::GroqModel>>, Option<Vec<super::generator::GeminiModel>>), String> {
    let (groq, gemini) = super::generator::get_cached_models().await;
    Ok((groq, gemini))
}

#[command]
pub async fn test_provider_cmd(
    pool: State<'_, DbPool>,
    provider: String,
) -> Result<(), String> {
    let key = match provider.as_str() {
        "groq" => {
            std::env::var("GROQ_API_KEY").ok().filter(|k| !k.trim().is_empty())
                .or_else(|| crate::pipeline::get_key_from_env_file("GROQ_API_KEY"))
                .or_else(|| {
                    let conn = pool.lock().ok()?;
                    crate::storage::get_user_preference_internal(&conn, "groq_api_key")
                        .ok()
                        .flatten()
                        .map(|p| p.value_json.trim_matches('"').to_string())
                        .filter(|k| !k.trim().is_empty())
                })
        }
        "gemini" => {
            std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty())
                .or_else(|| crate::pipeline::get_key_from_env_file("GEMINI_API_KEY"))
                .or_else(|| {
                    let conn = pool.lock().ok()?;
                    crate::storage::get_user_preference_internal(&conn, "gemini_api_key")
                        .ok()
                        .flatten()
                        .map(|p| p.value_json.trim_matches('"').to_string())
                        .filter(|k| !k.trim().is_empty())
                })
        }
        "ollama" => {
            // Ollama doesn't need a key, just test the URL
            Some("test".to_string())
        }
        _ => return Err("Unknown provider".to_string()),
    };

    if key.is_none() {
        return Err(format!("{} key not configured", provider));
    }

    let key_str = key.unwrap();
    super::generator::test_provider_connection(&provider, &key_str)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn parse_script_cmd(
    raw_script: String,
    language: String,
) -> Result<Vec<Scene>, String> {
    super::tts::parse_script(&raw_script, &language).await.map_err(|e| e.to_string())
}

#[command]
pub async fn update_scene_cmd(
    plan: ScriptToVideoPlan,
    scene_index: usize,
    updated_scene: Scene,
) -> Result<ScriptToVideoPlan, String> {
    if scene_index >= plan.scenes.len() {
        return Err("Scene index out of bounds".to_string());
    }
    
    let mut new_plan = plan;
    new_plan.scenes[scene_index] = updated_scene;
    Ok(new_plan)
}
