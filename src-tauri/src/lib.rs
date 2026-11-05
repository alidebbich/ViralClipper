// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod pipeline;
pub mod ffmpeg;
pub mod render;
pub mod assets;
pub mod templates;
pub mod storage;
pub mod common;
pub mod gemini;
pub mod ollama;
pub mod providers;
pub mod watchdog;
pub mod script_to_video;
pub mod types;
pub mod config;
pub mod error;

// Re-export common types (single source of truth)
pub use common::{Clip, Transcript, TranscriptSegment, Word, HeuristicScores, OllamaTranscript, ClipSegment};

// Re-export module-specific functionality
pub use pipeline::{PipelineStage, PipelineState, ClipInfo, StartPipelineRequest};
pub use ffmpeg::{render_clip, generate_ass_file};
pub use ollama::{rerank_clips, RerankResult};
pub use assets::{Asset, AssetSearchQuery};
pub use templates::{Template, AssStyle, FilterGraphConfig, Theme, default_templates};
pub use storage::{
    Project, Clip as DbClip, Asset as DbAsset, Template as DbTemplate, DbPool,
    init as init_db, get_projects, get_clips,
    SocialAccount, ScheduledPost, Thumbnail, ViralPrediction, ModelConfig,
    add_social_account, get_social_accounts, delete_social_account,
    schedule_post, get_scheduled_posts,
    create_thumbnail, get_thumbnails,
    create_viral_prediction, get_viral_predictions,
    get_model_configs, add_model_config, set_default_model,
    set_preference, get_preference,
    set_user_preference, get_user_preference,
};
pub use gemini::{predict_viral_potential, optimize_content, generate_thumbnail_prompts, ViralPredictionResult, ContentOptimizationResult};

use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Code};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_log::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            greet,
            pipeline::start_pipeline,
            pipeline::get_pipeline_status,
            pipeline::cancel_pipeline,
            assets::search_assets,
            assets::import_asset,
            assets::delete_asset,
            assets::sync_pexels_assets,
            templates::list_templates,
            templates::get_template,
            templates::save_template,
            templates::delete_template,
            templates::import_template,
            templates::export_template,
            storage::get_projects,
            storage::get_clips,
            storage::reset_database,
            pipeline::render::render_clip_cmd,
            pipeline::render::render_all_clips_cmd,
            pipeline::render::open_rendered_folder,
            pipeline::render::export_rendered_clip,
            pipeline::render::quick_export_clip,
            pipeline::render::generate_thumbnails,
            pipeline::render::delete_clip,
            pipeline::render::copy_file,
            pipeline::render::open_url,
            pipeline::render::get_clip_edl,
            pipeline::render::update_clip_edl,
            storage::get_user_preference,
            storage::set_user_preference,
            script_to_video::commands::generate_script_plan_cmd,
            script_to_video::commands::list_tts_voices_cmd,
            script_to_video::commands::list_tts_voices_by_language_cmd,
            script_to_video::commands::preview_tts_voice_cmd,
            script_to_video::commands::generate_scene_voice_cmd,
            script_to_video::commands::assemble_video_cmd,
            script_to_video::commands::refresh_models_cmd,
            script_to_video::commands::get_cached_models_cmd,
            script_to_video::commands::test_provider_cmd,
            script_to_video::commands::parse_script_cmd,
            script_to_video::commands::update_scene_cmd,
            debug_llm_keys,
        ])
        .setup(|app| {
            storage::init(app.handle().clone())?;
            watchdog::spawn_watchdog(Some(app.handle().clone()));

            app.handle().plugin(tauri_plugin_global_shortcut::Builder::new().build())?;

            let app_handle = app.handle().clone();
            let shortcut = tauri_plugin_global_shortcut::Shortcut::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyV
            );
            
            // Check if already registered and unregister if so
            if app.global_shortcut().is_registered(shortcut) {
                let _ = app.global_shortcut().unregister(shortcut);
            }
            
            app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, _event| {
                if let Some(window) = app_handle.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        window.hide().ok();
                    } else {
                        window.show().ok();
                        window.set_focus().ok();
                    }
                }
            })?;

            setup_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_tray(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItem::with_id(app_handle, "show", "Show Viral Clipper", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app_handle, "hide", "Hide Window", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app_handle, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app_handle, &[&show_item, &hide_item, &quit_item])?;

    let icon_data = include_bytes!("../icons/icon.png");
    let icon = tauri::image::Image::new_owned(icon_data.to_vec(), 32, 32);

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    window.show().ok();
                    window.set_focus().ok();
                }
            }
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide().ok();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app_handle)?;

    Ok(())
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to Viral Clipper", name)
}

/// Debug command — returns what LLM keys the app can currently find.
/// Call from browser console: window.__TAURI__.core.invoke('debug_llm_keys')
#[tauri::command]
fn debug_llm_keys(pool: tauri::State<'_, storage::DbPool>) -> String {
    let groq_env = std::env::var("GROQ_API_KEY").ok().filter(|k| !k.trim().is_empty());
    let groq_file = pipeline::get_key_from_env_file("GROQ_API_KEY");
    let groq_db = pool.lock().ok().and_then(|conn| {
        storage::get_user_preference_internal(&conn, "groq_api_key").ok().flatten()
            .map(|p| p.value_json.trim_matches('"').to_string())
            .filter(|k| !k.trim().is_empty())
    });

    let gemini_env = std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty());
    let gemini_file = pipeline::get_key_from_env_file("GEMINI_API_KEY");
    let gemini_db = pool.lock().ok().and_then(|conn| {
        storage::get_user_preference_internal(&conn, "gemini_api_key").ok().flatten()
            .map(|p| p.value_json.trim_matches('"').to_string())
            .filter(|k| !k.trim().is_empty())
    });

    let mask = |s: &Option<String>| s.as_deref().map(|k| format!("{}…", &k[..k.len().min(12)])).unwrap_or_else(|| "(not found)".to_string());

    format!(
        "GROQ: env={} | file={} | db={}\nGEMINI: env={} | file={} | db={}",
        mask(&groq_env), mask(&groq_file), mask(&groq_db),
        mask(&gemini_env), mask(&gemini_file), mask(&gemini_db),
    )
}
