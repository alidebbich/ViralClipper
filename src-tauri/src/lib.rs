// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod pipeline;
pub mod ffmpeg;
pub mod whisper;
pub mod ollama;
pub mod scorer;
pub mod assets;
pub mod templates;
pub mod thermal;
pub mod database;
pub mod common;
pub mod gemini;

// Re-export common types (single source of truth)
pub use common::{Clip, Transcript, TranscriptSegment, Word, HeuristicScores, OllamaTranscript, ClipSegment};

// Re-export module-specific functionality (functions only, no duplicate types)
pub use pipeline::{PipelineStage, PipelineState, ClipInfo, StartPipelineRequest};
pub use ffmpeg::{render_clip, generate_ass_file};
pub use whisper::{transcribe};
pub use ollama::{rerank_clips, RerankResult};
pub use scorer::{analyze_and_score};
pub use assets::{Asset, AssetSearchQuery};
pub use templates::{Template, AssStyle, FilterGraphConfig, Theme, default_templates};
pub use thermal::{ThermalStatus, ThermalConfig};
pub use database::{Project, Clip as DbClip, Asset as DbAsset, Template as DbTemplate, DbPool, init as init_db, get_projects, get_clips, SocialAccount, ScheduledPost, Thumbnail, ViralPrediction, ModelConfig, UserPreferences, add_social_account, get_social_accounts, delete_social_account, schedule_post, get_scheduled_posts, create_thumbnail, get_thumbnails, create_viral_prediction, get_viral_predictions, get_model_configs, add_model_config, set_default_model, set_preference, get_preference, set_user_preference, get_user_preference};
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
            database::get_projects,
            database::get_clips,
            database::reset_database,
            thermal::get_thermal_status,
            thermal::get_thermal_config,
            thermal::set_thermal_config,
            thermal::is_render_paused,
            thermal::wait_for_thermal_clear,
        ])
        .setup(|app| {
            database::init(app.handle().clone())?;
            thermal::start_monitor(app.handle().clone());
            
            app.handle().plugin(tauri_plugin_global_shortcut::Builder::new().build())?;
            
            let app_handle = app.handle().clone();
            let shortcut = tauri_plugin_global_shortcut::Shortcut::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT), 
                Code::KeyV
            );
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