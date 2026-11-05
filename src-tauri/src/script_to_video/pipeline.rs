use anyhow::Result;
use std::path::PathBuf;
use tracing::{info, warn};
use super::ScriptToVideoPlan;
use crate::common::{Clip, Transcript};
use crate::ffmpeg::resolve_ffmpeg;
use tokio::process::Command;

// ── Platform export presets ──────────────────────────────────────────────────

pub struct PlatformSpec {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub crf: u32,
    pub audio_bitrate: &'static str,
}

pub const TIKTOK: PlatformSpec = PlatformSpec {
    name: "tiktok",
    width: 1080, height: 1920, crf: 22, audio_bitrate: "192k",
};
pub const INSTAGRAM: PlatformSpec = PlatformSpec {
    name: "instagram",
    width: 1080, height: 1920, crf: 22, audio_bitrate: "192k",
};
pub const YOUTUBE_SHORTS: PlatformSpec = PlatformSpec {
    name: "youtube_shorts",
    width: 1080, height: 1920, crf: 20, audio_bitrate: "192k",
};

/// Encode a rendered MP4 to platform specs using FFmpeg.
pub async fn export_for_platform(
    input_path: &str,
    output_dir: &PathBuf,
    spec: &PlatformSpec,
) -> Result<String> {
    let out_path = output_dir.join(format!("export_{}.mp4", spec.name));
    let out_str = out_path.to_string_lossy().to_string();
    let ffmpeg = resolve_ffmpeg();

    let vf = format!(
        "scale={}:{}\
        :force_original_aspect_ratio=decrease,\
        pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,\
        setsar=1",
        spec.width, spec.height, spec.width, spec.height
    );

    let status = Command::new(&ffmpeg)
        .args([
            "-y",
            "-i", input_path,
            "-vf", &vf,
            "-c:v", "libx264",
            "-profile:v", "high",
            "-crf", &spec.crf.to_string(),
            "-preset", "slow",
            "-c:a", "aac",
            "-b:a", spec.audio_bitrate,
            "-movflags", "+faststart",
            "-pix_fmt", "yuv420p",
            &out_str,
        ])
        .status()
        .await?;

    if !status.success() {
        return Err(anyhow::anyhow!("Export encode failed for {}", spec.name));
    }

    info!("[Export] {} ✓ → {}", spec.name, out_str);
    Ok(out_str)
}

// ── Thumbnail generation ─────────────────────────────────────────────────────

/// Generate a thumbnail by picking the most impactful frame from the video
/// and burning a bold title overlay using FFmpeg.
pub async fn generate_thumbnail(
    video_path: &str,
    title: &str,
    output_path: &str,
    gemini_key: Option<&str>,
) -> Result<()> {
    let ffmpeg = resolve_ffmpeg();
    let duration = get_video_duration(video_path).await.unwrap_or(10.0);

    // Best frame candidates: 5%, 15%, 25% into video (hook area)
    let timestamps = [
        duration * 0.05,
        duration * 0.15,
        duration * 0.25,
    ];

    // Extract a few candidate frames
    let tmp_dir = std::path::Path::new(video_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);

    let mut best_frame = tmp_dir.join("thumb_candidate_0.jpg");

    // If Gemini key is available, extract 3 frames and let Gemini pick the best
    if let Some(key) = gemini_key {
        let mut frame_paths = Vec::new();
        for (i, ts) in timestamps.iter().enumerate() {
            let frame_path = tmp_dir.join(format!("thumb_candidate_{}.jpg", i));
            let _ = Command::new(&ffmpeg)
                .args([
                    "-y", "-ss", &ts.to_string(),
                    "-i", video_path,
                    "-vframes", "1",
                    "-q:v", "2",
                    frame_path.to_str().unwrap(),
                ])
                .output()
                .await;
            if frame_path.exists() {
                frame_paths.push(frame_path);
            }
        }

        if !frame_paths.is_empty() {
            best_frame = pick_best_frame_with_gemini(&frame_paths, key).await
                .unwrap_or_else(|_| frame_paths[0].clone());
        }
    } else {
        // No Gemini — just use the 15% frame
        let _ = Command::new(&ffmpeg)
            .args([
                "-y", "-ss", &(duration * 0.15).to_string(),
                "-i", video_path,
                "-vframes", "1",
                "-q:v", "2",
                best_frame.to_str().unwrap(),
            ])
            .output()
            .await;
    }

    // Burn title text over the frame with FFmpeg
    let safe_title = title.replace('\'', "").replace(':', "").chars().take(50).collect::<String>();
    let drawtext = format!(
        "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,\
        drawtext=text='{title}':\
        fontsize=72:fontcolor=white:\
        x=(w-text_w)/2:y=h*0.75:\
        shadowcolor=black@0.8:shadowx=3:shadowy=3:\
        box=1:boxcolor=black@0.5:boxborderw=20",
        title = safe_title
    );

    let out = Command::new(&ffmpeg)
        .args([
            "-y",
            "-i", best_frame.to_str().unwrap_or(video_path),
            "-vf", &drawtext,
            "-q:v", "1",
            output_path,
        ])
        .output()
        .await?;

    if !out.status.success() {
        // Fallback: just extract frame without overlay
        let _ = Command::new(&ffmpeg)
            .args([
                "-y", "-ss", &(duration * 0.15).to_string(),
                "-i", video_path, "-vframes", "1", "-q:v", "2", output_path,
            ])
            .output()
            .await;
    }

    Ok(())
}

async fn pick_best_frame_with_gemini(
    frames: &[PathBuf],
    _api_key: &str,
) -> Result<PathBuf> {
    // For now return frame index 1 (15%) — Gemini vision integration is Phase B
    // TODO: send frames as base64 to Gemini and ask it which is most striking
    Ok(frames.get(1).or_else(|| frames.first()).cloned()
        .ok_or_else(|| anyhow::anyhow!("No frames"))?)
}

async fn get_video_duration(path: &str) -> Result<f64> {
    let ffmpeg = resolve_ffmpeg().replace("ffmpeg", "ffprobe");
    let out = Command::new(&ffmpeg)
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            path,
        ])
        .output()
        .await?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let dur = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(10.0);
    Ok(dur)
}

// ── Main assembly pipeline ───────────────────────────────────────────────────

pub async fn assemble_video(plan: ScriptToVideoPlan) -> Result<String> {
    let cache_base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    let project_id = format!("script_{}", chrono::Utc::now().timestamp());
    let work_dir = cache_base.join("clipper").join("script_renders").join(&project_id);
    std::fs::create_dir_all(&work_dir)?;

    // Load Pexels key from env
    let pexels_key = crate::pipeline::get_key_from_env_file("PEXELS_API_KEY")
        .or_else(|| std::env::var("PEXELS_API_KEY").ok().filter(|k| !k.trim().is_empty()));

    let mut rendered_scenes: Vec<String> = Vec::new();

    for (i, scene) in plan.scenes.iter().enumerate() {
        info!("[ScriptPipeline] Scene {}/{}: {:?}", i + 1, plan.scenes.len(), &scene.text[..scene.text.len().min(40)]);

        let scene_base = work_dir.join(format!("scene_{}", i));
        let mp3_path = scene_base.with_extension("mp3");
        let mp3_str = mp3_path.to_string_lossy().to_string();

        // ── 1. TTS voiceover ─────────────────────────────────────────────────
        // Use language-appropriate default voice if none selected
        let voice = if !plan.voice.trim().is_empty() {
            plan.voice.clone()
        } else {
            default_voice_for_language(&plan.language)
        };

        match crate::script_to_video::tts::generate_tts(&scene.text, &voice, &mp3_str).await {
            Ok(()) => info!("[ScriptPipeline] TTS ✓ scene {}", i),
            Err(e) => {
                warn!("[ScriptPipeline] TTS failed scene {}: {} — using silence", i, e);
                generate_silence(&mp3_str, scene.duration_hint.unwrap_or(4.0)).await?;
            }
        }

        // Get actual audio duration
        let audio_duration = get_audio_duration(&mp3_str).await
            .unwrap_or(scene.duration_hint.unwrap_or(4.0));

        // ── 2. Stock footage ─────────────────────────────────────────────────
        let visual_path = scene_base.with_extension("mp4");
        let visual_str = visual_path.to_string_lossy().to_string();
        let keyword = scene.visual_keywords.first()
            .map(|s| s.as_str())
            .unwrap_or("nature landscape");

        crate::script_to_video::visuals::download_stock_video(
            keyword,
            &visual_str,
            audio_duration,
            pexels_key.as_deref(),
        ).await?;

        // ── 3. Animated captions (ASS subtitles) ────────────────────────────
        let transcript = Transcript {
            language: plan.language.clone(),
            duration: audio_duration,
            segments: vec![crate::common::TranscriptSegment {
                start: 0.0,
                end: audio_duration,
                text: scene.text.clone(),
                words: vec![],
            }],
            is_clip_relative: true,
        };


        let clip = {
            let mut c = Clip::new(project_id.clone(), 0.0, audio_duration, scene.text.clone());
            c.id = format!("scene_{}", i);
            c
        };
        let template = crate::templates::default_templates().into_iter().next().unwrap();
        let ass_path = crate::ffmpeg::generate_ass_file(&clip, &transcript.segments, &template, true)?;

        // ── 4. Render scene: visual + audio + captions ───────────────────────
        let rendered_path = work_dir.join(format!("scene_{}_rendered.mp4", i));
        let rendered_str = rendered_path.to_string_lossy().to_string();

        let escaped_ass = ass_path.replace('\\', "/").replace(':', "\\:");
        let camera_filter = camera_move_filter(&scene.camera, audio_duration);
        let vf = format!(
            "[0:v]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,setsar=1,{},ass='{}'[v]",
            camera_filter, escaped_ass
        );

        let ffmpeg = resolve_ffmpeg();
        let render_out = Command::new(&ffmpeg)
            .args([
                "-y",
                "-i", &visual_str,
                "-i", &mp3_str,
                "-filter_complex", &vf,
                "-map", "[v]",
                "-map", "1:a",
                "-c:v", "libx264",
                "-preset", "fast",
                "-crf", "22",
                "-c:a", "aac",
                "-b:a", "192k",
                "-shortest",
                &rendered_str,
            ])
            .output()
            .await?;

        if !render_out.status.success() {
            let stderr = String::from_utf8_lossy(&render_out.stderr);
            return Err(anyhow::anyhow!("FFmpeg scene {} render failed: {}", i, stderr));
        }

        rendered_scenes.push(rendered_str);
    }

    if rendered_scenes.is_empty() {
        return Err(anyhow::anyhow!("No scenes generated"));
    }

    // ── 5. Concatenate scenes ────────────────────────────────────────────────
    let concat_txt = work_dir.join("concat.txt");
    let concat_content: String = rendered_scenes.iter()
        .map(|p| format!("file '{}'\n", p.replace('\\', "/")))
        .collect();
    std::fs::write(&concat_txt, concat_content)?;

    let final_mp4 = work_dir.join("final_rendered.mp4");
    let final_str = final_mp4.to_string_lossy().to_string();
    let ffmpeg = resolve_ffmpeg();

    let concat_out = Command::new(&ffmpeg)
        .args([
            "-y", "-f", "concat", "-safe", "0",
            "-i", concat_txt.to_str().unwrap(),
            "-c", "copy",
            &final_str,
        ])
        .output()
        .await?;

    if !concat_out.status.success() {
        let stderr = String::from_utf8_lossy(&concat_out.stderr);
        return Err(anyhow::anyhow!("FFmpeg concat failed: {}", stderr));
    }

    // ── 6. Platform exports ──────────────────────────────────────────────────
    info!("[ScriptPipeline] Exporting for TikTok / Instagram / YouTube Shorts…");
    for spec in &[&TIKTOK, &INSTAGRAM, &YOUTUBE_SHORTS] {
        if let Err(e) = export_for_platform(&final_str, &work_dir, spec).await {
            warn!("[ScriptPipeline] Export for {} failed: {}", spec.name, e);
        }
    }

    // ── 7. Thumbnail ─────────────────────────────────────────────────────────
    let thumb_path = work_dir.join("thumbnail.jpg");
    let title = plan.title.as_deref().unwrap_or("Watch this!");
    let gemini_key = crate::pipeline::get_key_from_env_file("GEMINI_API_KEY")
        .or_else(|| std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty()));

    if let Err(e) = generate_thumbnail(
        &final_str,
        title,
        thumb_path.to_str().unwrap_or(""),
        gemini_key.as_deref(),
    ).await {
        warn!("[ScriptPipeline] Thumbnail gen failed: {}", e);
    } else {
        info!("[ScriptPipeline] Thumbnail ✓ → {}", thumb_path.display());
    }

    info!("[ScriptPipeline] Done! → {}", final_str);
    Ok(final_str)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn default_voice_for_language(lang: &str) -> String {
    let lang = lang.to_lowercase();
    if lang.contains("ar") {
        "ar-SA-HamedNeural".to_string()
    } else if lang.contains("fr") {
        "fr-FR-DeniseNeural".to_string()
    } else {
        "en-US-ChristopherNeural".to_string()
    }
}

/// Convert camera move name to FFmpeg zoompan/crop expression
fn camera_move_filter(camera: &str, duration: f64) -> String {
    match camera {
        "push_in" => {
            // Slow zoom in over the full scene duration
            let frames = (duration * 30.0) as i64; // assume 30fps
            format!("zoompan=z='min(zoom+0.0015,1.5)':d={}:s=1080x1920:fps=30", frames)
        }
        "pan" => {
            // Horizontal pan
            let frames = (duration * 30.0) as i64;
            format!("zoompan=z=1.1:x='iw/2-(iw/zoom/2)+t/{dur}*iw*0.03':d={frames}:s=1080x1920:fps=30",
                dur = duration, frames = frames)
        }
        _ => "setpts=PTS".to_string(), // static — no-op
    }
}

async fn get_audio_duration(mp3_path: &str) -> Result<f64> {
    let ffprobe = resolve_ffmpeg().replace("ffmpeg", "ffprobe");
    let out = Command::new(&ffprobe)
        .args(["-v", "quiet", "-print_format", "json", "-show_format", mp3_path])
        .output()
        .await?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let dur = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(4.0);
    Ok(dur)
}

async fn generate_silence(output_path: &str, duration: f64) -> Result<()> {
    let ffmpeg = resolve_ffmpeg();
    Command::new(&ffmpeg)
        .args([
            "-y", "-f", "lavfi",
            "-i", &format!("anullsrc=r=44100:cl=stereo:d={}", duration),
            "-c:a", "aac", "-b:a", "128k",
            output_path,
        ])
        .output()
        .await?;
    Ok(())
}
