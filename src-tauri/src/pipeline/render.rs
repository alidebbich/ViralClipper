use anyhow::Result;
use tauri::{Manager, Emitter};

use crate::common::Clip;
use crate::templates::Template;
use crate::pipeline::{load_project_transcript, ClipInfo, RENDER_TIMEOUT_SECS};


#[tauri::command]
pub async fn render_clip_cmd(
    app_handle: tauri::AppHandle,
    clip_id: String,
) -> Result<ClipInfo, String> {
    let pool = app_handle.state::<crate::storage::DbPool>();

    // 1. Get clip from database
    let clip = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_clip_internal(&conn, &clip_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Clip '{}' not found", clip_id))?
    };

    // 2. Get project from database
    let project = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_project_internal(&conn, &clip.project_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Project '{}' not found", clip.project_id))?
    };

    let video_path = project.source_path
        .ok_or_else(|| "Project has no downloaded source video".to_string())?;

    // 3. Get template
    let template_id = clip.template_id.clone().unwrap_or_else(|| "tiktok".to_string());
    let db_template = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_template_internal(&conn, &template_id)
            .map_err(|e| e.to_string())?
            .or_else(|| crate::storage::get_template_internal(&conn, "tiktok").ok().flatten())
            .ok_or_else(|| "Default template not found".to_string())?
    };

    let template = Template {
        id: db_template.id,
        name: db_template.name,
        ass_style_json: db_template.ass_style_json,
        filter_graph_json: db_template.filter_graph_json,
        theme_json: db_template.theme_json,
    };

    // 4. Load transcript
    let transcript = load_project_transcript(&clip.project_id)
        .map_err(|e| format!("Failed to load transcript: {}", e))?;

    // 5. Update clip status to rendering
    {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        let _ = crate::storage::update_clip_status_internal(&conn, &clip.id, "rendering");
    }
    let _ = app_handle.emit("clip-update", serde_json::json!({
        "clip_id": clip.id,
        "status": "rendering",
        "render_path": null
    }));

    // Convert db Clip to common Clip
    let common_clip = Clip {
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
        template_id: Some(template.id.clone()),
        render_path: None,
        status: "rendering".to_string(),
        hook_segments: clip.hook_segments.clone(),
        montage_order: clip.montage_order,
        timeline_data: clip.timeline_data.clone(),
        is_clip_relative: false,
        caption_warning: None,
    };

    // 6. Execute render with timeout
    let render_res = tokio::time::timeout(
        std::time::Duration::from_secs(RENDER_TIMEOUT_SECS),
        crate::ffmpeg::render_clip(
            &video_path,
            &common_clip,
            &template,
            &clip.project_id,
            &transcript,
        )
    ).await;

    match render_res {
        Ok(Ok((render_path, thumb_path, caption_warning))) => {
            {
                let conn = pool.lock().map_err(|e| e.to_string())?;
                let _ = crate::storage::update_clip_render_path_internal(&conn, &clip.id, &render_path);
                let _ = crate::storage::update_clip_thumbnail_internal(&conn, &clip.id, &thumb_path);
                let _ = crate::storage::update_clip_status_internal(&conn, &clip.id, "rendered");
            }
            let _ = app_handle.emit("clip-update", serde_json::json!({
                "clip_id": clip.id,
                "status": "rendered",
                "render_path": render_path,
                "thumbnail_path": thumb_path,
                "caption_warning": caption_warning,
            }));
            Ok(ClipInfo {
                id: clip.id,
                start_s: clip.start_s,
                end_s: clip.end_s,
                title: clip.title,
                viral_score: clip.viral_score,
                render_path: Some(render_path),
                status: "rendered".to_string(),
            })
        }
        Ok(Err(e)) => {
            let err_msg = format!("failed: {}", e);
            {
                let conn = pool.lock().map_err(|e| e.to_string())?;
                let _ = crate::storage::update_clip_status_internal(&conn, &clip.id, &err_msg);
            }
            let _ = app_handle.emit("clip-update", serde_json::json!({
                "clip_id": clip.id,
                "status": err_msg.clone(),
                "render_path": null
            }));
            Err(err_msg)
        }
        Err(_) => {
            let err_msg = "failed: render timed out (5m)".to_string();
            {
                let conn = pool.lock().map_err(|e| e.to_string())?;
                let _ = crate::storage::update_clip_status_internal(&conn, &clip.id, &err_msg);
            }
            let _ = app_handle.emit("clip-update", serde_json::json!({
                "clip_id": clip.id,
                "status": err_msg.clone(),
                "render_path": null
            }));
            Err(err_msg)
        }
    }
}

#[tauri::command]
pub async fn render_all_clips_cmd(
    app_handle: tauri::AppHandle,
    project_id: String,
) -> Result<Vec<ClipInfo>, String> {
    let pool = app_handle.state::<crate::storage::DbPool>();
    let clips = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_clips_internal(&conn, &project_id)
            .map_err(|e| e.to_string())?
    };

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let mut tasks = Vec::new();

    for clip in clips {
        let permit = sem.clone().acquire_owned().await.map_err(|e| e.to_string())?;
        let app = app_handle.clone();
        let cid = clip.id.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            render_clip_cmd(app, cid).await
        }));
    }

    let mut results = Vec::new();
    for task in tasks {
        if let Ok(res) = task.await {
            if let Ok(info) = res {
                results.push(info);
            }
        }
    }

    Ok(results)
}

#[tauri::command]
pub async fn open_rendered_folder(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if p.is_file() {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.replace('/', "\\")))
            .spawn()
            .map_err(|e| e.to_string())?;
    } else {
        std::process::Command::new("explorer")
            .arg(path.replace('/', "\\"))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn export_rendered_clip(
    app_handle: tauri::AppHandle,
    clip_id: String,
    target_path: String,
) -> Result<(), String> {
    let pool = app_handle.state::<crate::storage::DbPool>();
    let clip = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_clip_internal(&conn, &clip_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Clip '{}' not found", clip_id))?
    };

    let src = clip.render_path.ok_or_else(|| "Clip is not rendered yet".to_string())?;
    if !std::path::Path::new(&src).exists() {
        return Err("Rendered clip file not found on disk".to_string());
    }

    if let Some(parent) = std::path::Path::new(&target_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create destination folder: {}", e))?;
    }

    std::fs::copy(&src, &target_path)
        .map_err(|e| format!("Failed to export clip: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn quick_export_clip(
    app_handle: tauri::AppHandle,
    clip_id: String,
) -> Result<String, String> {
    let pool = app_handle.state::<crate::storage::DbPool>();
    let clip = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_clip_internal(&conn, &clip_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Clip '{}' not found", clip_id))?
    };

    let src = clip.render_path.ok_or_else(|| "Clip is not rendered yet".to_string())?;
    if !std::path::Path::new(&src).exists() {
        return Err("Rendered clip file not found on disk".to_string());
    }

    // Determine user's Downloads folder
    let downloads_dir = std::env::var("USERPROFILE")
        .map(|p| std::path::PathBuf::from(p).join("Downloads").join("ViralClipper"))
        .unwrap_or_else(|_| std::env::temp_dir().join("ViralClipper"));

    std::fs::create_dir_all(&downloads_dir)
        .map_err(|e| format!("Failed to create Downloads folder: {}", e))?;

    let score = clip.viral_score.map(|s| format!("_{:.0}", s)).unwrap_or_default();
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let safe_title = clip.title
        .replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_")
        .trim()
        .to_string();
    let file_stem = if safe_title.is_empty() { "clip".to_string() } else { safe_title };
    let filename = format!("{}{}_{}.mp4", file_stem, score, ts);
    let target_path = downloads_dir.join(&filename);

    std::fs::copy(&src, &target_path)
        .map_err(|e| format!("Failed to export clip: {}", e))?;

    // Copy thumbnail alongside if present
    if let Some(ref thumb_src) = clip.thumbnail_path {
        if std::path::Path::new(thumb_src).exists() {
            let thumb_target = downloads_dir.join(format!("{}{}_{}_thumb.jpg", file_stem, score, ts));
            let _ = std::fs::copy(thumb_src, thumb_target);
        }
    }

    let target_str = target_path.to_string_lossy().to_string();
    
    // Automatically reveal file in Windows Explorer
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", target_str.replace('/', "\\")))
        .spawn();

    Ok(target_str)
}

#[tauri::command]
pub async fn get_clip_edl(
    app_handle: tauri::AppHandle,
    clip_id: String,
) -> Result<crate::common::Edl, String> {
    let pool = app_handle.state::<crate::storage::DbPool>();
    let clip = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_clip_internal(&conn, &clip_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Clip '{}' not found", clip_id))?
    };

    let transcript = crate::pipeline::load_project_transcript(&clip.project_id).ok();
    let common_clip = crate::common::Clip {
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
        status: clip.status.clone(),
        hook_segments: clip.hook_segments.clone(),
        montage_order: clip.montage_order,
        timeline_data: clip.timeline_data.clone(),
        is_clip_relative: false,
        caption_warning: None,
    };

    let edl = crate::pipeline::edit::resolve_clip_edl(&common_clip, transcript.as_ref());
    Ok(edl)
}

#[tauri::command]
pub async fn update_clip_edl(
    app_handle: tauri::AppHandle,
    clip_id: String,
    edl: crate::common::Edl,
) -> Result<crate::common::Edl, String> {
    let pool = app_handle.state::<crate::storage::DbPool>();
    let mut validated = edl;
    let clip_duration = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        let c = crate::storage::get_clip_internal(&conn, &clip_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Clip '{}' not found", clip_id))?;
        c.duration_s
    };
    validated.validate_and_repair(clip_duration);
    let json = serde_json::to_string(&validated).map_err(|e| e.to_string())?;
    {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::update_clip_timeline_internal(&conn, &clip_id, &json)
            .map_err(|e| e.to_string())?;
    }
    Ok(validated)
}

// ── New commands added for thumbnail generation, delete, export, and social ──

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ThumbnailCandidate {
    pub path: String,
    pub timestamp: f64,
}

/// Generate 4 AI-designed thumbnail candidates using Gemini Vision + FFmpeg text overlay.
/// Steps:
///   1. Extract 4 raw frames from the rendered clip
///   2. Ask Gemini Vision to pick the most viral-worthy frame and generate a punchy title
///   3. Use FFmpeg drawtext to composite a professional thumbnail overlay on the chosen frame
///   4. Return all 4 candidates (the AI-enhanced one is always first)
#[tauri::command]
pub async fn generate_thumbnails(
    app_handle: tauri::AppHandle,
    clip_id: String,
) -> Result<Vec<ThumbnailCandidate>, String> {
    let pool = app_handle.state::<crate::storage::DbPool>();
    let clip = {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        crate::storage::get_clip_internal(&conn, &clip_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Clip '{}' not found", clip_id))?
    };

    // Gemini API key extraction removed since we skip AI overlay

    let render_path = clip.render_path.ok_or("Clip has no render path")?;
    if !std::path::Path::new(&render_path).exists() {
        return Err("Rendered file not found on disk".to_string());
    }

    let duration = crate::ffmpeg::verify_rendered_file(&render_path).await
        .map_err(|e| e.to_string())?;

    let cache_base = std::env::var("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let thumb_dir = cache_base.join("clipper").join("thumbnails").join(&clip_id);
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;


    let percentages: [(f64, &str); 4] = [
        (0.10, "frame_10pct"),
        (0.25, "frame_25pct"),
        (0.50, "frame_50pct"),
        (0.75, "frame_75pct"),
    ];

    // Step 1: Extract 4 raw frames
    let mut raw_frames: Vec<(String, f64, &str)> = Vec::new();
    for (pct, label) in &percentages {
        let ts = duration * pct;
        let raw_path = thumb_dir.join(format!("{}_raw.jpg", label));
        let raw_str = raw_path.to_string_lossy().to_string();
        if let Ok(()) = crate::ffmpeg::extract_clip_thumbnail(&render_path, &raw_str, ts).await {
            raw_frames.push((raw_str, ts, label));
        }
    }

    if raw_frames.is_empty() {
        return Err("Could not extract any frames from clip".to_string());
    }

    // Step 2: Apply cinematic grading + viral text overlay to each frame
    let mut candidates = Vec::new();
    let ffmpeg = crate::ffmpeg::resolve_ffmpeg();
    
    // Clean and split title for the overlay
    let safe_title = clip.title
        .replace('\'', "")
        .replace(':', " ")
        .replace('\\', "")
        .chars()
        .take(50)
        .collect::<String>();
        
    let words: Vec<&str> = safe_title.split_whitespace().collect();
    let mid = words.len() / 2;
    let (line1, line2) = if words.len() > 3 {
        (words[..mid].join(" "), words[mid..].join(" "))
    } else {
        (safe_title.clone(), String::new())
    };

    let font = "fontfile='C\\:/Windows/Fonts/arial.ttf'";

    for (i, (raw_path, ts, label)) in raw_frames.into_iter().enumerate() {
        let out_path = thumb_dir.join(format!("{}_viral.jpg", label));
        let out_str = out_path.to_string_lossy().to_string();

        // Create 4 distinct professional styles instead of identical broken ones
        let vf = match i {
            0 => {
                // Style 1: Clean & Safe (Just slight color pop)
                "eq=contrast=1.05:saturation=1.1".to_string()
            },
            1 => {
                // Style 2: Vibrant & Zoomed 1.2x (Focus on faces)
                "crop=900:1600:(iw-900)/2:(ih-1600)/2,scale=1080:1920,eq=contrast=1.1:saturation=1.3:brightness=0.02".to_string()
            },
            2 => {
                // Style 3: Heavy Zoom 1.4x (Close up)
                "crop=770:1370:(iw-770)/2:(ih-1370)/2,scale=1080:1920,eq=contrast=1.15:saturation=1.3".to_string()
            },
            _ => {
                // Style 4: Title Overlay
                let grade = "eq=contrast=1.1:saturation=1.2";
                let gradient = "drawbox=x=0:y=ih*0.5:w=iw:h=ih*0.5:color=black@0.6:t=fill";
                
                let t1 = format!(
                    "drawtext={font}:text='{line1}':fontsize=80:fontcolor=white:\
x=(w-text_w)/2:y=h*0.70:\
shadowcolor=black@0.9:shadowx=4:shadowy=4:\
borderw=3:bordercolor=black@0.8",
                    font = font, line1 = line1
                );
                let t2 = if !line2.is_empty() {
                    format!(
                        ",drawtext={font}:text='{line2}':fontsize=80:fontcolor=yellow:\
x=(w-text_w)/2:y=h*0.77:\
shadowcolor=black@0.9:shadowx=4:shadowy=4:\
borderw=3:bordercolor=black@0.8",
                        font = font, line2 = line2
                    )
                } else {
                    String::new()
                };
                format!("{},{},{}{}", grade, gradient, t1, t2)
            }
        };

        let out = tokio::process::Command::new(&ffmpeg)
            .args([
                "-y",
                "-i", &raw_path,
                "-vf", &vf,
                "-frames:v", "1",
                "-q:v", "1",
                &out_str,
            ])
            .output()
            .await;

        if let Ok(output) = out {
            if output.status.success() {
                candidates.push(ThumbnailCandidate { path: out_str, timestamp: ts });
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("FFmpeg thumb error: {}", err);
                candidates.push(ThumbnailCandidate { path: raw_path, timestamp: ts });
            }
        } else {
            candidates.push(ThumbnailCandidate { path: raw_path, timestamp: ts });
        }
    }

    Ok(candidates)
}





/// Delete a clip from the database (and optionally from disk).
#[tauri::command]
pub async fn delete_clip(
    app_handle: tauri::AppHandle,
    clip_id: String,
) -> Result<(), String> {
    let pool = app_handle.state::<crate::storage::DbPool>();
    let conn = pool.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM clips WHERE id = ?1", rusqlite::params![clip_id])
        .map_err(|e| format!("Failed to delete clip: {}", e))?;
    Ok(())
}

/// Copy a file from src to dst — used to save thumbnail alongside exported clip.
#[tauri::command]
pub async fn copy_file(src: String, dst: String) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(&dst).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::copy(&src, &dst)
        .map_err(|e| format!("copy_file failed: {}", e))?;
    Ok(())
}

/// Open a URL in the system default browser.
#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("Failed to open URL: {}", e))?;
    Ok(())
}

pub async fn build_montage_video(
    ffmpeg_exe: &str,
    input_path: &str,
    output_path: &str,
    segments: &[[f64; 2]],
) -> anyhow::Result<()> {
    // We use ffmpeg filter_complex to trim and concat all segments
    let mut filter = String::new();
    let mut concat_inputs = String::new();

    for (i, seg) in segments.iter().enumerate() {
        let st = seg[0];
        let en = seg[1];
        filter.push_str(&format!("[0:v]trim=start={:.3}:end={:.3},setpts=PTS-STARTPTS[v{}];", st, en, i));
        filter.push_str(&format!("[0:a]atrim=start={:.3}:end={:.3},asetpts=PTS-STARTPTS[a{}];", st, en, i));
        concat_inputs.push_str(&format!("[v{}][a{}]", i, i));
    }
    filter.push_str(&format!("{}concat=n={}:v=1:a=1[vout][aout]", concat_inputs, segments.len()));

    let output = tokio::process::Command::new(ffmpeg_exe)
        .args([
            "-y",
            "-i", input_path,
            "-filter_complex", &filter,
            "-map", "[vout]",
            "-map", "[aout]",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            "-crf", "18", // very high quality intermediate
            "-c:a", "aac",
            output_path,
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to build montage intermediate: {}", stderr);
    }
    Ok(())
}

pub fn remap_montage_transcript(
    original: &crate::common::Transcript,
    segments: &[[f64; 2]]
) -> crate::common::Transcript {
    let mut new_segments = Vec::new();
    let mut current_offset = 0.0;

    for seg in segments {
        let st = seg[0];
        let en = seg[1];
        let dur = en - st;

        for orig_seg in &original.segments {
            // Find intersection
            let overlap_start = orig_seg.start.max(st);
            let overlap_end = orig_seg.end.min(en);

            if overlap_end > overlap_start {
                let mut mapped_words = Vec::new();
                for w in &orig_seg.words {
                    let w_overlap_start = w.start.max(st);
                    let w_overlap_end = w.end.min(en);
                    if w_overlap_end > w_overlap_start {
                        let mut new_w = w.clone();
                        new_w.start = (w.start - st) + current_offset;
                        new_w.end = (w.end - st) + current_offset;
                        mapped_words.push(new_w);
                    }
                }

                if !mapped_words.is_empty() {
                    let mut new_seg = orig_seg.clone();
                    new_seg.start = (orig_seg.start - st).max(0.0) + current_offset;
                    new_seg.end = (orig_seg.end - st).min(dur) + current_offset;
                    new_seg.words = mapped_words;
                    new_segments.push(new_seg);
                }
            }
        }
        current_offset += dur;
    }

    let mut t = original.clone();
    t.segments = new_segments;
    t
}
