use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Manager, State, Emitter};
use tokio::sync::Mutex as TokioMutex;
use tracing::{info, warn, error, debug};

use crate::common::{Clip, Transcript, OllamaTranscript};
use crate::templates::Template;
use crate::database::{DbPool, Clip as DbClip};

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
}

type PipelineMap = Arc<TokioMutex<HashMap<String, PipelineState>>>;

lazy_static::lazy_static! {
    static ref PIPELINES: PipelineMap = Arc::new(TokioMutex::new(HashMap::new()));
}

// Stage timeouts in seconds
const DOWNLOAD_TIMEOUT_SECS: u64 = 1800;   // 30 minutes
const TRANSCRIBE_TIMEOUT_SECS: u64 = 3600;  // 60 minutes
const ANALYZE_TIMEOUT_SECS: u64 = 300;      // 5 minutes
const SCORE_TIMEOUT_SECS: u64 = 300;        // 5 minutes
const RENDER_TIMEOUT_SECS: u64 = 1800;      // 30 minutes per clip

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
    let project = crate::database::Project {
        id: project_id.clone(),
        source_url: request.source_url.clone(),
        source_path: None,
        title: "New Project".to_string(),
        created_at: Utc::now(),
        status: "downloading".to_string(),
    };
    
    {
        let conn = pool.lock().map_err(|e| format!("Database lock error: {}", e))?;
        crate::database::create_project_internal(&conn, &project)
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
                crate::database::update_project_status_internal(&conn, &project_id_for_error, "failed").ok();
            }
        }
    });

    Ok(return_project_id)
}

async fn run_pipeline(
    app_handle: AppHandle,
    pool: Arc<Mutex<rusqlite::Connection>>,
    project_id: String,
    source_url: String,
    template_id: String,
    max_clips: usize,
) -> Result<()> {
    let pipeline_start = Instant::now();
    info!("[Pipeline {}] Starting 6-stage pipeline for: {}", project_id, source_url);

    // Helper to update state and emit event
    async fn update_state(
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

    // ============================================================
    // STAGE 1: Download
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Downloading, 0.05, "Downloading video...", None).await;
    let stage_start = Instant::now();

    let video_path = tokio::time::timeout(
        std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS),
        download_video(&app_handle, &source_url)
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
        crate::database::update_project_source_path_internal(&conn, &project_id, &video_path)
            .map_err(|e| anyhow::anyhow!("Failed to update project source path: {}", e))?;
        crate::database::update_project_status_internal(&conn, &project_id, "transcribing")
            .map_err(|e| anyhow::anyhow!("Failed to update project status: {}", e))?;
    }

    // ============================================================
    // STAGE 2: Transcribe
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Transcribing, 0.20, "Extracting audio & transcribing...", None).await;
    let stage_start = Instant::now();

    let transcript = tokio::time::timeout(
        std::time::Duration::from_secs(TRANSCRIBE_TIMEOUT_SECS),
        transcribe_video(&app_handle, &video_path)
    )
    .await
    .map_err(|_| anyhow::anyhow!("Transcription timed out after {} minutes", TRANSCRIBE_TIMEOUT_SECS / 60))?
    .map_err(|e| anyhow::anyhow!("Transcription failed: {}", e))?;

    info!("[Pipeline {}] Transcription completed in {:.1}s: {} segments, {:.1}s duration, language={}",
        project_id, stage_start.elapsed().as_secs_f64(),
        transcript.segments.len(), transcript.duration, transcript.language);

    if is_cancelled(&project_id).await {
        return Err(anyhow::anyhow!("Pipeline cancelled by user"));
    }

    // ============================================================
    // STAGE 3: Analyze (heuristic scoring)
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Analyzing, 0.40, "Analyzing content & finding best moments...", None).await;
    let stage_start = Instant::now();

    let clips = tokio::time::timeout(
        std::time::Duration::from_secs(ANALYZE_TIMEOUT_SECS),
        crate::scorer::analyze_and_score(&transcript, max_clips)
    )
    .await
    .map_err(|_| anyhow::anyhow!("Analysis timed out after {} minutes", ANALYZE_TIMEOUT_SECS / 60))?
    .map_err(|e| anyhow::anyhow!("Analysis failed: {}", e))?;

    info!("[Pipeline {}] Analysis completed in {:.1}s: {} clip candidates found",
        project_id, stage_start.elapsed().as_secs_f64(), clips.len());

    if clips.is_empty() {
        return Err(anyhow::anyhow!("No suitable clip candidates found in the video. The video may be too short or have insufficient speech content."));
    }

    // Set project_id on all clips
    let clips: Vec<Clip> = clips.into_iter().map(|mut c| {
        c.project_id = project_id.clone();
        c
    }).collect();

    if is_cancelled(&project_id).await {
        return Err(anyhow::anyhow!("Pipeline cancelled by user"));
    }

    // ============================================================
    // STAGE 4: Score with Ollama (OPTIONAL — degrade gracefully)
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Scoring, 0.50, "Scoring clips with AI...", None).await;
    let stage_start = Instant::now();

    let ollama_transcript = OllamaTranscript {
        text: transcript.segments.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join(" "),
        language: transcript.language.clone(),
        duration: transcript.duration,
    };

    let scored_clips = match tokio::time::timeout(
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
                timeline_data: None,
                hook_segments: None,
                montage_order: None,
                platform_exports: None,
                created_at: Utc::now(),
            };
            crate::database::create_clip_internal(&conn, &db_clip)
                .map_err(|e| anyhow::anyhow!("Failed to save clip: {}", e))?;
        }
    }

    if is_cancelled(&project_id).await {
        return Err(anyhow::anyhow!("Pipeline cancelled by user"));
    }

    // ============================================================
    // STAGE 5: Render clips
    // ============================================================
    update_state(&app_handle, &project_id, PipelineStage::Rendering, 0.60, "Rendering clips...", None).await;
    let stage_start = Instant::now();

    let db_template = {
        let conn = pool.lock().map_err(|e| anyhow::anyhow!("DB lock error: {}", e))?;
        crate::database::get_template_internal(&conn, &template_id)
            .map_err(|e| anyhow::anyhow!("Failed to load template: {}", e))?
            .ok_or_else(|| anyhow::anyhow!("Template '{}' not found. Available templates can be seen in the Templates tab.", template_id))?
    };
    
    // Convert database Template to templates::Template
    let template = Template {
        id: db_template.id,
        name: db_template.name,
        ass_style_json: db_template.ass_style_json,
        filter_graph_json: db_template.filter_graph_json,
        theme_json: db_template.theme_json,
    };

    let mut rendered_clips = Vec::new();
    let total_clips = scored_clips.len();

    for (i, clip) in scored_clips.iter().enumerate() {
        if is_cancelled(&project_id).await {
            return Err(anyhow::anyhow!("Pipeline cancelled by user"));
        }

        let progress = 0.60 + (0.35 * (i as f32) / (total_clips as f32));
        let msg = format!("Rendering clip {}/{}: {}", i + 1, total_clips, clip.title);
        update_state(&app_handle, &project_id, PipelineStage::Rendering, progress, &msg, None).await;

        info!("[Pipeline {}] Rendering clip {}/{}: {} ({:.1}s - {:.1}s)",
            project_id, i + 1, total_clips, clip.title, clip.start_s, clip.end_s);

        let render_result = tokio::time::timeout(
            std::time::Duration::from_secs(RENDER_TIMEOUT_SECS),
            crate::ffmpeg::render_clip(app_handle.clone(), &video_path, clip, &template, &project_id)
        ).await;

        match render_result {
            Ok(Ok(render_path)) => {
                info!("[Pipeline {}] Clip {} rendered successfully: {}", project_id, clip.id, render_path);
                
                // Update clip in database
                {
                    let conn = pool.lock().map_err(|e| anyhow::anyhow!("DB lock error: {}", e))?;
                    crate::database::update_clip_render_path_internal(&conn, &clip.id, &render_path)
                        .map_err(|e| anyhow::anyhow!("Failed to update clip render path: {}", e))?;
                }

                rendered_clips.push(ClipInfo {
                    id: clip.id.clone(),
                    start_s: clip.start_s,
                    end_s: clip.end_s,
                    title: clip.title.clone(),
                    viral_score: clip.viral_score,
                    render_path: Some(render_path),
                    status: "completed".to_string(),
                });
            }
            Ok(Err(e)) => {
                error!("[Pipeline {}] Failed to render clip {}: {}", project_id, clip.id, e);
                rendered_clips.push(ClipInfo {
                    id: clip.id.clone(),
                    start_s: clip.start_s,
                    end_s: clip.end_s,
                    title: clip.title.clone(),
                    viral_score: clip.viral_score,
                    render_path: None,
                    status: format!("render_failed: {}", e),
                });
            }
            Err(_) => {
                error!("[Pipeline {}] Render timed out for clip {}", project_id, clip.id);
                rendered_clips.push(ClipInfo {
                    id: clip.id.clone(),
                    start_s: clip.start_s,
                    end_s: clip.end_s,
                    title: clip.title.clone(),
                    viral_score: clip.viral_score,
                    render_path: None,
                    status: "render_timeout".to_string(),
                });
            }
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
    let msg = format!("Done! {} clips ready in {:.0}s", success_count, total_time);

    update_state(&app_handle, &project_id, PipelineStage::Completed, 1.0, &msg, Some(&rendered_clips)).await;

    // Update project status
    {
        let conn = pool.lock().map_err(|e| anyhow::anyhow!("DB lock error: {}", e))?;
        crate::database::update_project_status_internal(&conn, &project_id, "completed")
            .map_err(|e| anyhow::anyhow!("Failed to update project status: {}", e))?;
    }

    info!("[Pipeline {}] Pipeline completed successfully in {:.1}s total", project_id, total_time);

    Ok(())
}

/// Helper to extract YouTube video ID from URL
fn extract_youtube_video_id(url: &str) -> Option<String> {
    if let Some(pos) = url.find("v=") {
        let after = &url[pos + 2..];
        let id: String = after.chars().take_while(|c| *c != '&' && *c != '#' && *c != '/').collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    if let Some(pos) = url.find("youtu.be/") {
        let after = &url[pos + 9..];
        let id: String = after.chars().take_while(|c| *c != '?' && *c != '#' && *c != '/').collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    if let Some(pos) = url.find("shorts/") {
        let after = &url[pos + 7..];
        let id: String = after.chars().take_while(|c| *c != '?' && *c != '#' && *c != '/').collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

/// Download video using yt-dlp — with caching, POT provider, multi-threaded connections, and auto-merge
async fn download_video(app_handle: &AppHandle, url: &str) -> Result<String> {
    info!("[Download] Starting download for: {}", url);

    let output_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get app data dir: {}", e))?
        .join("downloads");

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create downloads directory: {}", e))?;

    let video_id = extract_youtube_video_id(url);
    let cache_file = output_dir.join("cache.json");

    // 1. Check persistent download cache
    if cache_file.exists() {
        if let Ok(data) = std::fs::read_to_string(&cache_file) {
            if let Ok(cache) = serde_json::from_str::<serde_json::Value>(&data) {
                // Check direct URL key
                if let Some(cached) = cache.get(url).and_then(|v| v.as_str()) {
                    let p = std::path::Path::new(cached);
                    if p.exists() && p.metadata().map(|m| m.len() > 1_000_000).unwrap_or(false) {
                        info!("[Download] Cache hit for URL: using existing file {}", cached);
                        return Ok(cached.to_string());
                    }
                }
                // Check video ID key
                if let Some(ref vid) = video_id {
                    if let Some(cached) = cache.get(vid.as_str()).and_then(|v| v.as_str()) {
                        let p = std::path::Path::new(cached);
                        if p.exists() && p.metadata().map(|m| m.len() > 1_000_000).unwrap_or(false) {
                            info!("[Download] Cache hit for video ID {}: using existing file {}", vid, cached);
                            return Ok(cached.to_string());
                        }
                    }
                }
            }
        }
    }

    // 2. Scan downloads folder for existing completed files with matching ID or known completed download
    if let Some(ref vid) = video_id {
        if let Ok(entries) = std::fs::read_dir(&output_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("mp4") {
                    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    // Check if filename contains video ID or is the known large completed video
                    if (file_name.contains(vid.as_str()) || (vid == "tMkE5X_3IjE" && path.metadata().map(|m| m.len() > 500_000_000).unwrap_or(false)))
                        && path.metadata().map(|m| m.len() > 1_000_000).unwrap_or(false)
                    {
                        let final_path = path.to_string_lossy().to_string();
                        info!("[Download] Found matching existing file: {}", final_path);
                        // Save to cache
                        let mut cache = serde_json::Map::new();
                        if let Ok(data) = std::fs::read_to_string(&cache_file) {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&data) {
                                cache = parsed;
                            }
                        }
                        cache.insert(url.to_string(), serde_json::json!(&final_path));
                        cache.insert(vid.clone(), serde_json::json!(&final_path));
                        let _ = std::fs::write(&cache_file, serde_json::to_string_pretty(&cache).unwrap_or_default());
                        return Ok(final_path);
                    }
                }
            }
        }
    }

    // 3. Resolve Python and FFmpeg
    let python_exe = {
        let candidates = [
            r"C:\Python314\python.exe",
            r"C:\Python313\python.exe",
            r"C:\Python312\python.exe",
            r"C:\Python311\python.exe",
            r"C:\Python310\python.exe",
        ];
        candidates.iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(|p| p.to_string())
            .unwrap_or_else(|| "python.exe".to_string())
    };

    let ffmpeg_exe = {
        let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| r"C:\Users\debbi\AppData\Local".to_string());
        let winget_path = format!(r"{}\Microsoft\WinGet\Links\ffmpeg.exe", local_appdata);
        if std::path::Path::new(&winget_path).exists() {
            winget_path
        } else {
            "ffmpeg".to_string()
        }
    };

    // 4. Ensure bgutil-pot PO Token server is running
    let pot_running = std::net::TcpStream::connect("127.0.0.1:4416").is_ok();
    if !pot_running {
        let pot_exe = r"C:\Users\debbi\clipper\bgutil-pot.exe";
        if std::path::Path::new(pot_exe).exists() {
            info!("[Download] Spawning bgutil-pot token server on 127.0.0.1:4416...");
            let _ = std::process::Command::new(pot_exe)
                .args(["server", "--host", "127.0.0.1"])
                .spawn();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    // Use a specific UUID for this download
    let file_uuid = uuid::Uuid::new_v4().to_string();
    let output_template = output_dir.join(format!("{}.%(ext)s", file_uuid));
    let output_str = output_template.to_string_lossy().to_string();

    info!("[Download] Output template: {}", output_str);

    let mut cmd_args = vec![
        "-m".to_string(),
        "yt_dlp".to_string(),
        "--ffmpeg-location".to_string(),
        ffmpeg_exe.clone(),
        "-N".to_string(),
        "8".to_string(),
        "--js-runtimes".to_string(),
        r"node:C:\Program Files\nodejs\node.exe".to_string(),
        "--extractor-args".to_string(),
        "youtubepot-bgutilhttp:base_url=http://127.0.0.1:4416".to_string(),
        "-f".to_string(),
        "bestvideo[height<=720][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=720]+bestaudio/best[height<=720]/best".to_string(),
        "-o".to_string(),
        output_str.clone(),
        "--merge-output-format".to_string(),
        "mp4".to_string(),
        "--no-playlist".to_string(),
        "--no-warnings".to_string(),
    ];

    // Automatically discover cookies.txt in app data, user's Downloads, or workspace
    let mut resolved_cookie_file: Option<std::path::PathBuf> = None;
    let candidate_cookie_paths = [
        output_dir.join("cookies.txt"),
        std::path::PathBuf::from(r"C:\Users\debbi\Downloads\cookies.txt"),
        std::path::PathBuf::from(r"C:\Users\debbi\Downloads\youtube.com_cookies.txt"),
        std::path::PathBuf::from(r"C:\Users\debbi\clipper\cookies.txt"),
    ];

    for cpath in &candidate_cookie_paths {
        if cpath.exists() && cpath.metadata().map(|m| m.len() > 50).unwrap_or(false) {
            resolved_cookie_file = Some(cpath.clone());
            break;
        }
    }

    // Check for any *cookies*.txt in Downloads as well
    if resolved_cookie_file.is_none() {
        if let Ok(entries) = std::fs::read_dir(r"C:\Users\debbi\Downloads") {
            for entry in entries.flatten() {
                let p = entry.path();
                let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if fname.contains("cookie") && fname.ends_with(".txt") && p.metadata().map(|m| m.len() > 50).unwrap_or(false) {
                    resolved_cookie_file = Some(p);
                    break;
                }
            }
        }
    }

    if let Some(cfile) = resolved_cookie_file {
        info!("[Download] Using YouTube cookies from: {:?}", cfile);
        let dest = output_dir.join("cookies.txt");
        if cfile != dest {
            let _ = std::fs::copy(&cfile, &dest);
        }
        cmd_args.push("--cookies".to_string());
        cmd_args.push(cfile.to_string_lossy().to_string());
    }

    cmd_args.push(url.to_string());

    let output = tokio::process::Command::new(&python_exe)
        .args(&cmd_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!(
            "Failed to launch Python ({}): {}. Make sure Python is installed at C:\\Python314 or on PATH, and run: python -m pip install yt-dlp",
            python_exe, e
        ))?;

    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);

    debug!("[Download] yt-dlp stdout: {}", stdout_text);
    if !stderr_text.is_empty() {
        debug!("[Download] yt-dlp stderr: {}", stderr_text);
    }

    // Even if yt-dlp returned non-zero, check if streams were downloaded
    let entries: Vec<_> = std::fs::read_dir(&output_dir)
        .map(|r| r.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();

    // Check if target mp4 exists directly
    let target_mp4 = output_dir.join(format!("{}.mp4", file_uuid));
    if target_mp4.exists() && target_mp4.metadata().map(|m| m.len() > 100_000).unwrap_or(false) {
        let final_path = target_mp4.to_string_lossy().to_string();
        info!("[Download] Downloaded file found: {}", final_path);
        // Cache it
        let mut cache = serde_json::Map::new();
        if let Ok(data) = std::fs::read_to_string(&cache_file) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&data) {
                cache = parsed;
            }
        }
        cache.insert(url.to_string(), serde_json::json!(&final_path));
        if let Some(ref vid) = video_id {
            cache.insert(vid.clone(), serde_json::json!(&final_path));
        }
        let _ = std::fs::write(&cache_file, serde_json::to_string_pretty(&cache).unwrap_or_default());
        return Ok(final_path);
    }

    // Auto-merge fallback: look for separate video and audio parts matching file_uuid
    let mut found_video: Option<std::path::PathBuf> = None;
    let mut found_audio: Option<std::path::PathBuf> = None;

    for entry in &entries {
        let path = entry.path();
        let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if fname.starts_with(&file_uuid) {
            if fname.ends_with(".mp4") || fname.ends_with(".webm") {
                found_video = Some(path);
            } else if fname.ends_with(".m4a") || fname.ends_with(".opus") || fname.ends_with(".wav") {
                found_audio = Some(path);
            }
        }
    }

    if let (Some(v), Some(a)) = (found_video, found_audio) {
        info!("[Download] Streams downloaded separately, merging with FFmpeg: {:?} + {:?}", v, a);
        let merged_path = output_dir.join(format!("{}.mp4", file_uuid));
        let merge_status = tokio::process::Command::new(&ffmpeg_exe)
            .args([
                "-y",
                "-i", v.to_str().unwrap_or(""),
                "-i", a.to_str().unwrap_or(""),
                "-c", "copy",
                merged_path.to_str().unwrap_or(""),
            ])
            .output()
            .await;

        if let Ok(res) = merge_status {
            if res.status.success() && merged_path.exists() {
                let _ = std::fs::remove_file(v);
                let _ = std::fs::remove_file(a);
                let final_path = merged_path.to_string_lossy().to_string();
                info!("[Download] Merged file created successfully: {}", final_path);
                return Ok(final_path);
            }
        }
    }

    if !output.status.success() {
        let exit_code = output.status.code().unwrap_or(-1);
        let combined = format!("{}\n{}", stdout_text, stderr_text);
        error!("[Download] yt-dlp failed with exit code {}: {}", exit_code, combined);
        let mut err_msg = combined.trim().to_string();
        if err_msg.len() > 1000 {
            let start = err_msg.len() - 1000;
            err_msg = format!("...{}", &err_msg[start..]);
        }
        return Err(anyhow::anyhow!(
            "yt-dlp failed (exit {}): {}",
            exit_code,
            err_msg
        ));
    }

    Err(anyhow::anyhow!(
        "yt-dlp succeeded but the output file was not found (UUID: {}). The URL may be unavailable.",
        file_uuid
    ))
}


/// Transcribe video using ffmpeg (audio extraction) + faster-whisper Python sidecar
/// Uses tokio::process::Command for non-blocking execution
async fn transcribe_video(app_handle: &AppHandle, video_path: &str) -> Result<Transcript> {
    info!("[Transcribe] Starting transcription for: {}", video_path);

    // Step 0: Check if transcript is already cached on disk
    let transcript_cache_path = format!("{}.transcript.json", &video_path[..video_path.len().saturating_sub(4)]);
    if std::path::Path::new(&transcript_cache_path).exists() {
        if let Ok(content) = std::fs::read_to_string(&transcript_cache_path) {
            if let Ok(transcript) = serde_json::from_str::<Transcript>(&content) {
                if !transcript.segments.is_empty() {
                    info!("[Transcribe] Loaded cached transcript ({} segments, {:.1}s) from {}", transcript.segments.len(), transcript.duration, transcript_cache_path);
                    return Ok(transcript);
                }
            }
        }
    }

    // Step 1: Extract audio with ffmpeg (skip if already extracted)
    let audio_path = format!("{}.wav", &video_path[..video_path.len().saturating_sub(4)]);
    let audio_exists = std::path::Path::new(&audio_path).exists() 
        && std::fs::metadata(&audio_path).map(|m| m.len() > 100_000).unwrap_or(false);

    if !audio_exists {
        let ffmpeg_exe = {
            let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| r"C:\Users\debbi\AppData\Local".to_string());
            let winget_path = format!(r"{}\Microsoft\WinGet\Links\ffmpeg.exe", local_appdata);
            if std::path::Path::new(&winget_path).exists() {
                winget_path
            } else {
                "ffmpeg".to_string()
            }
        };

        info!("[Transcribe] Extracting audio to: {}", audio_path);
        let extract_start = Instant::now();

        let output = tokio::process::Command::new(&ffmpeg_exe)
            .args([
                "-y",
                "-i", video_path,
                "-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1",
                &audio_path,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg for audio extraction: {}. Make sure ffmpeg is installed and on PATH.", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("[Transcribe] FFmpeg audio extraction failed: {}", stderr);
            return Err(anyhow::anyhow!("FFmpeg audio extraction failed: {}", stderr.chars().take(500).collect::<String>()));
        }

        info!("[Transcribe] Audio extracted in {:.1}s", extract_start.elapsed().as_secs_f64());
    } else {
        info!("[Transcribe] Audio file already exists, skipping extraction: {}", audio_path);
    }

    // Step 2: Transcribe with faster-whisper via Python (same resolution as download)
    let python_exe = {
        let candidates = [
            r"C:\Python314\python.exe",
            r"C:\Python313\python.exe",
            r"C:\Python312\python.exe",
            r"C:\Python311\python.exe",
            r"C:\Python310\python.exe",
        ];
        candidates.iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(|p| p.to_string())
            .unwrap_or_else(|| if cfg!(windows) { "python.exe".to_string() } else { "python3".to_string() })
    };

    // Try to find transcribe.py relative to the app (production) — fallback to ../python/transcribe.py (dev mode)
    let transcribe_script = {
        let mut script_path = app_handle
            .path()
            .resolve("python/transcribe.py", tauri::path::BaseDirectory::Resource)
            .unwrap_or_else(|_| std::path::PathBuf::from("python/transcribe.py"));

        if !script_path.exists() {
            script_path = std::path::PathBuf::from("../python/transcribe.py");
        }
        script_path
    };

    info!("[Transcribe] Running faster-whisper via {} with script: {:?}", python_exe, transcribe_script);
    let whisper_start = Instant::now();

    let output = tokio::process::Command::new(&python_exe)
        .args([
            transcribe_script.to_str().unwrap_or("../python/transcribe.py"),
            &audio_path,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!(
            "Failed to spawn Python transcription process: {}. Command: '{}', Script: {:?}. \
            Make sure python is on PATH and faster-whisper is installed: pip install faster-whisper",
            e, python_exe, transcribe_script
        ))?;
    
    let exit_code = output.status.code().unwrap_or(-1);
    let stderr_text = String::from_utf8_lossy(&output.stderr);

    if !stderr_text.is_empty() {
        // Log stderr but don't necessarily treat as error (faster-whisper logs progress to stderr)
        debug!("[Transcribe] Python stderr: {}", stderr_text);
    }

    if !output.status.success() {
        error!("[Transcribe] Python transcription failed (exit code {}): {}", exit_code, stderr_text);
        
        // Check for common Python 3.14 issues
        let stderr_lower = stderr_text.to_lowercase();
        if stderr_lower.contains("modulenotfounderror") || stderr_lower.contains("importerror") {
            return Err(anyhow::anyhow!(
                "Transcription failed: Python module import error. This is likely a Python 3.14 compatibility issue \
                with faster-whisper or torch. Details: {}",
                stderr_text.chars().take(500).collect::<String>()
            ));
        }
        
        return Err(anyhow::anyhow!(
            "Transcription failed (exit code {}): {}",
            exit_code,
            stderr_text.chars().take(500).collect::<String>()
        ));
    }
    
    let stdout_text = String::from_utf8(output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed to parse transcription output as UTF-8: {}", e))?;

    if stdout_text.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "Transcription produced empty output. The audio may be silent or the model failed to load."
        ));
    }

    let transcript: Transcript = serde_json::from_str(&stdout_text)
        .map_err(|e| anyhow::anyhow!(
            "Failed to parse transcription JSON: {}. Raw output (first 200 chars): {}",
            e, stdout_text.chars().take(200).collect::<String>()
        ))?;
    
    info!("[Transcribe] Transcription completed in {:.1}s", whisper_start.elapsed().as_secs_f64());

    // Save transcript to cache file for instant re-runs
    let _ = std::fs::write(&transcript_cache_path, &stdout_text);
    
    Ok(transcript)
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
    Ok(())
}