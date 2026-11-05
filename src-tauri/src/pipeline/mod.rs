use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, State, Emitter};
use tokio::sync::Mutex as TokioMutex;

pub mod download;
pub mod transcribe;
pub mod analyze;
pub mod edit;
pub mod render;
pub use transcribe::find_python;
use tracing::{info, warn, error};

use crate::common::{Clip, Transcript, OllamaTranscript};
use crate::templates::Template;
use crate::storage::{DbPool, Clip as DbClip};
use crate::pipeline::render::{build_montage_video, remap_montage_transcript};

// ============================================================
// PIPELINE TYPES
// ============================================================

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub enum PipelineStage {
    Downloading,
    Transcribing,
    Analyzing,
    Scoring,
    Rendering,
    Completed,
    Failed,
}

impl std::fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineStage::Downloading => write!(f, "Downloading"),
            PipelineStage::Transcribing => write!(f, "Transcribing"),
            PipelineStage::Analyzing => write!(f, "Analyzing"),
            PipelineStage::Scoring => write!(f, "Scoring"),
            PipelineStage::Rendering => write!(f, "Rendering"),
            PipelineStage::Completed => write!(f, "Completed"),
            PipelineStage::Failed => write!(f, "Failed"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PipelineState {
    pub project_id: String,
    pub current_stage: PipelineStage,
    pub progress: f32,
    pub message: String,
    pub clips: Vec<ClipInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClipInfo {
    pub id: String,
    pub start_s: f64,
    pub end_s: f64,
    pub title: String,
    pub viral_score: Option<f64>,
    pub render_path: Option<String>,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StartPipelineRequest {
    pub source_url: String,
    pub template_id: Option<String>,
    pub max_clips: Option<usize>,
    pub language: Option<String>,
}

type PipelineMap = Arc<TokioMutex<HashMap<String, PipelineState>>>;

lazy_static::lazy_static! {
    pub(crate) static ref PIPELINES: PipelineMap = Arc::new(TokioMutex::new(HashMap::new()));
}

// Stage timeouts in seconds
const DOWNLOAD_TIMEOUT_SECS: u64 = 1800;  // 30 minutes for long/high-res videos
pub const TRANSCRIBE_TIMEOUT_SECS: u64 = 300;  // 5 minutes
const ANALYZE_TIMEOUT_SECS: u64 = 120;      // 2 minutes
const SCORE_TIMEOUT_SECS: u64 = 120;        // 2 minutes
pub const RENDER_TIMEOUT_SECS: u64 = 300;      // 5 minutes per clip

#[tauri::command]
pub async fn start_pipeline(
    app_handle: AppHandle,
    pool: State<'_, DbPool>,
    request: StartPipelineRequest,
) -> Result<String, String> {
    info!("Starting pipeline for URL: {}", request.source_url);

    // Create project
    let project_id = uuid::Uuid::new_v4().to_string();
    let return_project_id = project_id.clone();
    let project = crate::storage::Project {
        id: project_id.clone(),
        source_url: request.source_url.clone(),
        source_path: None,
        title: "New Project".to_string(),
        created_at: Utc::now(),
        status: "downloading".to_string(),
    };
    
    {
        let conn = pool.lock().map_err(|e| format!("Database lock error: {}", e))?;
        crate::storage::create_project_internal(&conn, &project)
            .map_err(|e| format!("Failed to create project: {}", e))?;
    }

    // Initialize pipeline state
    {
        let mut pipelines = PIPELINES.lock().await;
        pipelines.insert(project_id.clone(), PipelineState {
            project_id: project_id.clone(),
            current_stage: PipelineStage::Downloading,
            progress: 0.0,
            message: "Starting download...".to_string(),
            clips: Vec::new(),
            error: None,
        });
    }

    // Spawn background task — clone the pool Arc so the error handler can use it
    let app_handle_clone = app_handle.clone();
    let pool_clone = pool.inner().clone();
    let pool_for_error = pool.inner().clone();
    let template_id = request.template_id.unwrap_or_else(|| "tiktok".to_string());
    let max_clips = request.max_clips.unwrap_or(3);
    let language = request.language.unwrap_or_else(|| "auto".to_string());
    let project_id_for_closure = project_id.clone();
    let project_id_for_error = project_id.clone();

    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_pipeline(
            app_handle_clone.clone(),
            pool_clone,
            project_id_for_closure.clone(),
            request.source_url,
            template_id,
            max_clips,
            language,
        ).await {
            error!("Pipeline failed for project {}: {}", project_id_for_error, e);

            // Update pipeline state with error
            let mut pipelines = PIPELINES.lock().await;
            if let Some(state) = pipelines.get_mut(&project_id_for_error) {
                state.current_stage = PipelineStage::Failed;
                state.error = Some(e.to_string());
                state.message = format!("Failed: {}", e);
            }

            // Emit failure event to frontend
            let _ = app_handle_clone.emit("pipeline-update", serde_json::json!({
                "project_id": project_id_for_error,
                "current_stage": "Failed",
                "progress": 0.0,
                "message": format!("Failed: {}", e),
                "clips": [],
                "error": e.to_string()
            }));

            // Update project status in the REAL database
            if let Ok(conn) = pool_for_error.lock() {
                crate::storage::update_project_status_internal(&conn, &project_id_for_error, "failed").ok();
            }
        }
    });

    Ok(return_project_id)
}

// Helper to update state and emit event
pub(crate) async fn update_state(
    app_handle: &AppHandle,
    project_id: &str,
    stage: PipelineStage,
    progress: f32,
    message: &str,
    clips: Option<&[ClipInfo]>,
) {
    let stage_name = stage.to_string();
    info!("[Pipeline {}] Stage: {} | Progress: {:.0}% | {}", project_id, stage_name, progress * 100.0, message);

    let clips_vec = clips.unwrap_or(&[]).to_vec();

    {
        let mut pipelines = PIPELINES.lock().await;
        if let Some(state) = pipelines.get_mut(project_id) {
            state.current_stage = stage.clone();
            state.progress = progress;
            state.message = message.to_string();
            if !clips_vec.is_empty() {
                state.clips = clips_vec.clone();
            }
        }
    }

    // Emit event with string stage name (matching frontend expectations)
    let _ = app_handle.emit("pipeline-update", serde_json::json!({
        "project_id": project_id,
        "current_stage": stage_name,
        "progress": progress,
        "message": message,
        "clips": clips_vec
    }));
}

// Helper to check if pipeline was cancelled
async fn is_cancelled(project_id: &str) -> bool {
    let pipelines = PIPELINES.lock().await;
    if let Some(state) = pipelines.get(project_id) {
        state.current_stage == PipelineStage::Failed
            && state.error.as_deref() == Some("Cancelled by user")
    } else {
        false
    }
}

async fn run_pipeline(
    app_handle: AppHandle,
    pool: Arc<Mutex<rusqlite::Connection>>,
    project_id: String,
    source_url: String,
    template_id: String,
    max_clips: usize,
    language: String,
) -> Result<()> {
    let pipeline_start = Instant::now();
    info!("[Pipeline {}] Starting 6-stage pipeline for: {}", project_id, source_url);



    // ============================================================
    // STAGE 1: Download
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Downloading, 0.05, "Downloading video...", None).await;
    let stage_start = Instant::now();

    let video_path = tokio::time::timeout(
        std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS),
        download::download_video(&app_handle, &project_id, &source_url)
    )
    .await
    .map_err(|_| anyhow::anyhow!("Download timed out after {} minutes", DOWNLOAD_TIMEOUT_SECS / 60))?
    .map_err(|e| anyhow::anyhow!("Download failed: {}", e))?;

    info!("[Pipeline {}] Download completed in {:.1}s: {}", project_id, stage_start.elapsed().as_secs_f64(), video_path);

    if is_cancelled(&project_id).await {
        return Err(anyhow::anyhow!("Pipeline cancelled by user"));
    }

    // Update project with source path
    {
        let conn = pool.lock().map_err(|e| anyhow::anyhow!("DB lock error: {}", e))?;
        crate::storage::update_project_source_path_internal(&conn, &project_id, &video_path)
            .map_err(|e| anyhow::anyhow!("Failed to update project source path: {}", e))?;
        crate::storage::update_project_status_internal(&conn, &project_id, "transcribing")
            .map_err(|e| anyhow::anyhow!("Failed to update project status: {}", e))?;
    }

    // ============================================================
    // STAGE 2: Transcribe
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Transcribing, 0.20, "Extracting audio & transcribing...", None).await;
    let stage_start = Instant::now();

    let transcript = transcribe::transcribe_video(&app_handle, &video_path, &project_id, &pool, &language)
        .await
        .map_err(|e| anyhow::anyhow!("Transcription failed: {}", e))?;

    info!("[Pipeline {}] Transcription completed in {:.1}s: {} segments, {:.1}s duration, language={}",
        project_id, stage_start.elapsed().as_secs_f64(),
        transcript.segments.len(), transcript.duration, transcript.language);

    // Save transcript to disk for rendering / re-rendering
    let _ = save_project_transcript(&project_id, &transcript);

    if is_cancelled(&project_id).await {
        return Err(anyhow::anyhow!("Pipeline cancelled by user"));
    }

    // ============================================================
    // STAGE 3 & 4: Analyze & Score Content (AI or Heuristic)
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Scoring, 0.40, "AI is watching the full video to find viral concepts...", None).await;
    let stage_start = Instant::now();

    // Check for GEMINI_API_KEY from environment, .env file, or user preferences
    let gemini_key = std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty())
        .or_else(|| get_key_from_env_file("GEMINI_API_KEY"))
        .or_else(|| {
            let conn = pool.lock().ok()?;
            crate::storage::get_user_preference_internal(&conn, "gemini_api_key")
                .ok()
                .flatten()
                .map(|p| p.value_json.trim_matches('"').to_string())
                .filter(|k| !k.trim().is_empty())
        });

    let mut scored_clips = if let Some(ref key) = gemini_key {
        info!("[Pipeline {}] Using Clip Finder V2 (Whole Video + Extra Signals)...", project_id);
        
        let info_json_path = video_path.replace(".mp4", ".info.json");
        let json_opt = if std::path::Path::new(&info_json_path).exists() { Some(info_json_path.as_str()) } else { None };
        let signals = crate::pipeline::analyze::signals::extract_signals(&video_path, json_opt).await.unwrap_or_default();
        
        match tokio::time::timeout(
            std::time::Duration::from_secs(SCORE_TIMEOUT_SECS * 5), // 10 minutes timeout for long videos
            crate::pipeline::analyze::clip_finder::run_clip_finder(key, &transcript, &signals, max_clips)
        ).await {
            Ok(Ok(mut ai_clips)) => {
                info!("[Pipeline {}] Clip Finder found {} clips in {:.1}s", 
                    project_id, ai_clips.len(), stage_start.elapsed().as_secs_f64());
                // Set project_id on all clips
                for c in &mut ai_clips {
                    c.project_id = project_id.clone();
                }
                ai_clips
            }
            Ok(Err(e)) => {
                warn!("[Pipeline {}] Clip Finder failed (falling back to heuristic): {}", project_id, e);
                Vec::new() // triggers fallback
            }
            Err(_) => {
                warn!("[Pipeline {}] Clip Finder timed out (falling back to heuristic)", project_id);
                Vec::new() // triggers fallback
            }
        }
    } else {
        Vec::new()
    };

    // Fallback: If Gemini failed, or no key, use heuristic sliding window + Ollama rerank
    if scored_clips.is_empty() {
        update_state(&app_handle, &project_id, PipelineStage::Analyzing, 0.45, "Using fallback heuristic analysis...", None).await;
        
        let mut clips = tokio::time::timeout(
            std::time::Duration::from_secs(ANALYZE_TIMEOUT_SECS),
            crate::pipeline::analyze::analyze_and_score(&transcript, max_clips)
        )
        .await
        .map_err(|_| anyhow::anyhow!("Analysis timed out"))?
        .map_err(|e| anyhow::anyhow!("Analysis failed: {}", e))?;

        if clips.is_empty() {
            return Err(anyhow::anyhow!("No suitable clip candidates found in the video."));
        }

        for c in &mut clips {
            c.project_id = project_id.clone();
        }

        update_state(&app_handle, &project_id, PipelineStage::Scoring, 0.50, "Evaluating clips...", None).await;

        let ollama_transcript = OllamaTranscript {
            text: transcript.segments.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join(" "),
            language: transcript.language.clone(),
            duration: transcript.duration,
        };

        scored_clips = match tokio::time::timeout(
            std::time::Duration::from_secs(SCORE_TIMEOUT_SECS),
            crate::ollama::rerank_clips(&clips, &ollama_transcript)
        ).await {
            Ok(Ok(reranked)) => {
                info!("[Pipeline {}] Ollama reranking completed in {:.1}s", project_id, stage_start.elapsed().as_secs_f64());
                reranked
            }
            Ok(Err(e)) => {
                warn!("[Pipeline {}] Ollama reranking failed (continuing without AI scoring): {}", project_id, e);
                update_state(&app_handle, &project_id, PipelineStage::Scoring, 0.55,
                    "AI scoring unavailable — using heuristic scores", None).await;
                clips.clone()
            }
            Err(_) => {
                warn!("[Pipeline {}] Ollama reranking timed out (continuing without AI scoring)", project_id);
                update_state(&app_handle, &project_id, PipelineStage::Scoring, 0.55,
                    "AI scoring timed out — using heuristic scores", None).await;
                clips.clone()
            }
        }
    };

    // Save clips to database
    {
        let conn = pool.lock().map_err(|e| anyhow::anyhow!("DB lock error: {}", e))?;
        for clip in &scored_clips {
            // Create a database clip from the common clip
            let db_clip = DbClip {
                id: clip.id.clone(),
                project_id: clip.project_id.clone(),
                start_s: clip.start_s,
                end_s: clip.end_s,
                duration_s: clip.duration_s,
                title: clip.title.clone(),
                hook_score: clip.hook_score,
                viral_score: clip.viral_score,
                heuristic_json: clip.heuristic_json.clone(),
                llm_reason: clip.llm_reason.clone(),
                template_id: clip.template_id.clone(),
                render_path: clip.render_path.clone(),
                thumbnail_path: None,
                status: "analyzed".to_string(),
                timeline_data: clip.timeline_data.clone(),
                hook_segments: None,
                montage_order: None,
                platform_exports: None,
                created_at: Utc::now(),
            };
            crate::storage::create_clip_internal(&conn, &db_clip)
                .map_err(|e| anyhow::anyhow!("Failed to save clip: {}", e))?;
        }
    }

    if is_cancelled(&project_id).await {
        return Err(anyhow::anyhow!("Pipeline cancelled by user"));
    }

    // ============================================================
    // STAGE 5: Render clips (automatically after analysis, max 2 in parallel)
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Rendering, 0.60, "Rendering clips (max 2 parallel)...", None).await;
    let stage_start = Instant::now();

    let db_template = {
        let conn = pool.lock().map_err(|e| anyhow::anyhow!("DB lock error: {}", e))?;
        crate::storage::get_template_internal(&conn, &template_id)
            .map_err(|e| anyhow::anyhow!("Failed to load template: {}", e))?
            .or_else(|| crate::storage::get_template_internal(&conn, "tiktok").ok().flatten())
            .ok_or_else(|| anyhow::anyhow!("Default template 'tiktok' not found."))?
    };
    
    // Convert database Template to templates::Template
    let template = Template {
        id: db_template.id,
        name: db_template.name,
        ass_style_json: db_template.ass_style_json,
        filter_graph_json: db_template.filter_graph_json,
        theme_json: db_template.theme_json,
    };

    // ── Quality filter: only render clips with viral_score >= 6.0 ──────────
    const MIN_VIRAL_SCORE: f64 = 6.0;
    let mut render_candidates: Vec<Clip> = scored_clips.iter()
        .filter(|c| c.viral_score.unwrap_or(0.0) >= MIN_VIRAL_SCORE)
        .cloned()
        .collect();

    let dropped = scored_clips.len() - render_candidates.len();
    if dropped > 0 {
        info!("[Pipeline {}] Score filter: dropped {} clip(s) with viral_score < {} (keeping {} clip(s) for render)",
            project_id, dropped, MIN_VIRAL_SCORE, render_candidates.len());
    }

    // Guarantee at least 1 clip always renders (keep the highest-scored one)
    if render_candidates.is_empty() {
        if let Some(best) = scored_clips.iter()
            .max_by(|a, b| a.viral_score.unwrap_or(0.0).partial_cmp(&b.viral_score.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
        {
            warn!("[Pipeline {}] All clips scored below {}; keeping top clip '{}' ({:.1}/10) for render",
                project_id, MIN_VIRAL_SCORE, best.title, best.viral_score.unwrap_or(0.0));
            render_candidates.push(best.clone());
        }
    }

    let total_clips = render_candidates.len();
    
    // ── Generate EDL for each candidate using the Editor Brain ──────────
    let key_ref = gemini_key.as_deref().unwrap_or("");
    let info_json_path = video_path.replace(".mp4", ".info.json");
    let json_opt = if std::path::Path::new(&info_json_path).exists() { Some(info_json_path.as_str()) } else { None };
    let signals = crate::pipeline::analyze::signals::extract_signals(&video_path, json_opt).await.unwrap_or_default();
    
    for (i, clip) in render_candidates.iter_mut().enumerate() {
        update_state(&app_handle, &project_id, PipelineStage::Rendering, 0.65, &format!("Generating Editor Brain (EDL) for clip {}/{}...", i + 1, total_clips), None).await;
        if let Ok(edl) = crate::pipeline::edit::build_edl_for_clip(key_ref, clip, &transcript, &signals).await {
            if let Ok(json) = serde_json::to_string(&edl) {
                clip.timeline_data = Some(json.clone());
                clip.heuristic_json = Some(json.clone());
                if let Ok(conn) = pool.lock() {
                    let _ = crate::storage::update_clip_timeline_internal(&conn, &clip.id, &json);
                }
            }
        } else {
            warn!("[Pipeline {}] Failed to generate EDL for clip {}", project_id, clip.id);
        }
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(2));
    let mut render_tasks = Vec::new();

    for (i, clip) in render_candidates.iter().enumerate() {
        let sem = semaphore.clone();
        let app = app_handle.clone();
        let v_path = video_path.clone();
        let c = clip.clone();
        
        let specific_template_id = c.template_id.as_deref().unwrap_or(&template_id);
        let db_template_specific = if let Ok(conn) = pool.lock() {
            crate::storage::get_template_internal(&conn, specific_template_id)
                .unwrap_or(None)
        } else {
            None
        };
        
        let tmpl = if let Some(db_t) = db_template_specific {
            Template {
                id: db_t.id,
                name: db_t.name,
                ass_style_json: db_t.ass_style_json,
                filter_graph_json: db_t.filter_graph_json,
                theme_json: db_t.theme_json,
            }
        } else {
            template.clone()
        };

        let p_id = project_id.clone();
        let t = transcript.clone();
        let p_pool = pool.clone();

        // Mark clip as rendering in DB & UI
        {
            if let Ok(conn) = p_pool.lock() {
                let _ = crate::storage::update_clip_status_internal(&conn, &c.id, "rendering");
            }
        }
        let _ = app.emit("clip-update", serde_json::json!({
            "clip_id": c.id,
            "status": "rendering",
            "render_path": null
        }));

        render_tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.ok();
            info!("[Pipeline {}] Starting render for clip {} ({}/{}): {} ({:.1}s - {:.1}s)",
                p_id, c.id, i + 1, total_clips, c.title, c.start_s, c.end_s);

            let mut final_v_path = v_path.clone();
            let mut final_c = c.clone();
            let mut final_t = t.clone();

            if let Some(ref hooks_json) = final_c.hook_segments {
                if let Ok(segments) = serde_json::from_str::<Vec<[f64; 2]>>(hooks_json) {
                    if segments.len() > 1 {
                        let cache_base = std::env::var("LOCALAPPDATA")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|_| std::env::temp_dir());
                        let pre_montage_path = cache_base.join("clipper").join(format!("{}_montage.mp4", final_c.id));
                        let pre_montage_str = pre_montage_path.to_string_lossy().to_string();

                        tracing::info!("[Montage {}] Generating intermediate summary video...", final_c.id);
                        let ffmpeg_exe = crate::ffmpeg::resolve_ffmpeg();
                        if let Ok(()) = build_montage_video(&ffmpeg_exe, &v_path, &pre_montage_str, &segments).await {
                            final_t = remap_montage_transcript(&t, &segments);
                            
                            let mut total_duration = 0.0;
                            for seg in &segments {
                                total_duration += seg[1] - seg[0];
                            }
                            final_c.start_s = 0.0;
                            final_c.duration_s = total_duration;
                            final_c.end_s = total_duration;
                            final_v_path = pre_montage_str;
                        } else {
                            tracing::warn!("[Montage {}] Failed to generate intermediate video. Falling back to default render.", final_c.id);
                        }
                    }
                }
            }

            let render_result = tokio::time::timeout(
                std::time::Duration::from_secs(RENDER_TIMEOUT_SECS),
                crate::ffmpeg::render_clip(&final_v_path, &final_c, &tmpl, &p_id, &final_t)
            ).await;

            let (status, render_path_opt) = match render_result {
                Ok(Ok((render_path, thumb_path, caption_warning))) => {
                    info!("[Pipeline {}] Clip {} rendered successfully: {}", p_id, c.id, render_path);
                    if let Ok(conn) = p_pool.lock() {
                        let _ = crate::storage::update_clip_render_path_internal(&conn, &c.id, &render_path);
                        let _ = crate::storage::update_clip_thumbnail_internal(&conn, &c.id, &thumb_path);
                        let _ = crate::storage::update_clip_status_internal(&conn, &c.id, "rendered");
                    }
                    let _ = app.emit("clip-update", serde_json::json!({
                        "clip_id": c.id,
                        "status": "rendered",
                        "render_path": render_path,
                        "thumbnail_path": thumb_path,
                        "caption_warning": caption_warning,
                    }));
                    ("rendered".to_string(), Some(render_path))
                }
                Ok(Err(e)) => {
                    let err_msg = format!("failed: {}", e);
                    error!("[Pipeline {}] Failed to render clip {}: {}", p_id, c.id, e);
                    if let Ok(conn) = p_pool.lock() {
                        let _ = crate::storage::update_clip_status_internal(&conn, &c.id, &err_msg);
                    }
                    let _ = app.emit("clip-update", serde_json::json!({
                        "clip_id": c.id,
                        "status": err_msg.clone(),
                        "render_path": null
                    }));
                    (err_msg, None)
                }
                Err(_) => {
                    let err_msg = "failed: render timed out (5m)".to_string();
                    error!("[Pipeline {}] Render timed out for clip {}", p_id, c.id);
                    if let Ok(conn) = p_pool.lock() {
                        let _ = crate::storage::update_clip_status_internal(&conn, &c.id, &err_msg);
                    }
                    let _ = app.emit("clip-update", serde_json::json!({
                        "clip_id": c.id,
                        "status": err_msg.clone(),
                        "render_path": null
                    }));
                    (err_msg, None)
                }
            };

            ClipInfo {
                id: c.id,
                start_s: c.start_s,
                end_s: c.end_s,
                title: c.title,
                viral_score: c.viral_score,
                render_path: render_path_opt,
                status,
            }
        }));
    }

    let mut rendered_clips = Vec::new();
    for (idx, task) in render_tasks.into_iter().enumerate() {
        if let Ok(clip_info) = task.await {
            rendered_clips.push(clip_info);
            let prog = 0.60 + (0.35 * ((idx + 1) as f32) / (total_clips as f32));
            let msg = format!("Rendered {}/{} clips", idx + 1, total_clips);
            update_state(&app_handle, &project_id, PipelineStage::Rendering, prog, &msg, None).await;
        }
    }

    info!("[Pipeline {}] Rendering completed in {:.1}s: {}/{} clips rendered successfully",
        project_id, stage_start.elapsed().as_secs_f64(),
        rendered_clips.iter().filter(|c| c.render_path.is_some()).count(), total_clips);

    // Update pipeline state with final clips
    {
        let mut pipelines = PIPELINES.lock().await;
        if let Some(state) = pipelines.get_mut(&project_id) {
            state.clips = rendered_clips.clone();
        }
    }

    // ============================================================
    // STAGE 6: Completed
    // ============================================================
    let total_time = pipeline_start.elapsed().as_secs_f64();
    let success_count = rendered_clips.iter().filter(|c| c.render_path.is_some()).count();
    let msg = if success_count > 0 {
        format!("Done! {}/{} clips ready in {:.0}s", success_count, total_clips, total_time)
    } else {
        format!("Completed with render errors: 0/{} clips rendered", total_clips)
    };

    update_state(&app_handle, &project_id, PipelineStage::Completed, 1.0, &msg, Some(&rendered_clips)).await;

    // Update project status
    {
        let conn = pool.lock().map_err(|e| anyhow::anyhow!("DB lock error: {}", e))?;
        crate::storage::update_project_status_internal(&conn, &project_id, "completed")
            .map_err(|e| anyhow::anyhow!("Failed to update project status: {}", e))?;
    }

    info!("[Pipeline {}] Pipeline completed successfully in {:.1}s total", project_id, total_time);

    Ok(())
}


// ── Small helpers ─────────────────────────────────────────────────────────────



pub fn get_key_from_env_file(key_name: &str) -> Option<String> {
    // Build candidate list: CWD relatives + walk up from the binary's own directory
    let mut candidates: Vec<std::path::PathBuf> = vec![
        std::path::PathBuf::from(".env"),
        std::path::PathBuf::from("../.env"),
        std::path::PathBuf::from("../../.env"),
        std::path::PathBuf::from("../../../.env"),
    ];

    // Also walk up from the executable location (handles tauri dev target/debug/ layout)
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        for _ in 0..5 {
            if let Some(d) = dir {
                candidates.push(d.join(".env"));
                dir = d.parent().map(|p| p.to_path_buf());
            } else {
                break;
            }
        }
    }

    let parse_value = |content: &str| -> Option<String> {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key_name {
                    let clean = v.trim().trim_matches('"').trim_matches('\'').trim().to_string();
                    if !clean.is_empty() {
                        return Some(clean);
                    }
                }
            }
        }
        None
    };

    for env_path in &candidates {
        if let Ok(content) = std::fs::read_to_string(env_path) {
            if let Some(v) = parse_value(&content) {
                return Some(v);
            }
        }
    }
    None
}








#[tauri::command]
pub async fn get_pipeline_status(project_id: String) -> Result<Option<PipelineState>, String> {
    let pipelines = PIPELINES.lock().await;
    Ok(pipelines.get(&project_id).cloned())
}

#[tauri::command]
pub async fn cancel_pipeline(project_id: String) -> Result<(), String> {
    info!("[Pipeline {}] Cancellation requested", project_id);
    let mut pipelines = PIPELINES.lock().await;
    if let Some(state) = pipelines.get_mut(&project_id) {
        state.current_stage = PipelineStage::Failed;
        state.error = Some("Cancelled by user".to_string());
        state.message = "Cancelled by user".to_string();
    }
    // Also kill all child processes to prevent orphans
    let sys = sysinfo::System::new_all();
    if let Ok(pid) = sysinfo::get_current_pid() {
        crate::watchdog::kill_child_processes(&sys, pid);
    }
    Ok(())
}

pub async fn abort_all_pipelines(app: &tauri::AppHandle) {
    let mut pipelines = PIPELINES.lock().await;
    for state in pipelines.values_mut() {
        state.current_stage = PipelineStage::Failed;
        state.error = Some("Stopped to protect your PC: RAM High".to_string());
        state.message = "Stopped to protect your PC: RAM High".to_string();
        let _ = app.emit("pipeline-update", state.clone());
    }
}

pub fn save_project_transcript(project_id: &str, transcript: &Transcript) -> Result<()> {
    let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let dir = std::path::PathBuf::from(local_appdata).join("clipper").join("transcripts");
    std::fs::create_dir_all(&dir)?;
    let file_path = dir.join(format!("{}.json", project_id));
    let content = serde_json::to_string(transcript)?;
    std::fs::write(file_path, content)?;
    Ok(())
}

pub fn load_project_transcript(project_id: &str) -> Result<Transcript> {
    let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let file_path = std::path::PathBuf::from(&local_appdata).join("clipper").join("transcripts").join(format!("{}.json", project_id));
    if file_path.exists() {
        let content = std::fs::read_to_string(&file_path)?;
        return Ok(serde_json::from_str(&content)?);
    }
    // Fallback: check transcript_cache
    let cache_dir = std::path::PathBuf::from(&local_appdata).join("clipper").join("transcript_cache");
    if cache_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(cache_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        if let Ok(t) = serde_json::from_str::<Transcript>(&content) {
                            if t.segments.len() > 10 {
                                return Ok(t);
                            }
                        }
                    }
                }
            }
        }
    }
    Err(anyhow::anyhow!("No transcript found for project {}", project_id))
}

