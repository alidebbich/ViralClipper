use anyhow::Result;
use crate::common::{Clip, TranscriptSegment};
use crate::templates::Template;

pub async fn render_clip(
    app_handle: tauri::AppHandle,
    video_path: &str,
    clip: &Clip,
    template: &Template,
    project_id: &str,
) -> Result<String> {
    let output_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("clipper")
        .join("renders")
        .join(project_id);
    std::fs::create_dir_all(&output_dir)?;

    let output_path = output_dir.join(format!("{}.mp4", clip.id));

    // Generate ASS subtitle file
    let transcript: Vec<TranscriptSegment> = vec![]; // Would be passed in
    generate_ass_file(clip, &transcript, template)?;

    // Build filter graph
    let filter_graph = build_filter_graph(clip, template)?;

    // Run FFmpeg with NVENC
    run_ffmpeg_nvenc(app_handle.clone(), video_path, &output_path.to_string_lossy(), &filter_graph, clip.start_s, clip.duration_s).await?;

    Ok(output_path.to_string_lossy().to_string())
}

fn build_filter_graph(clip: &Clip, template: &Template) -> Result<String> {
    let filter_graph = template.filter_graph().map_err(|e| anyhow::anyhow!(e))?;
    let theme = template.theme().map_err(|e| anyhow::anyhow!(e))?;
    let mut filters = Vec::new();

    // 1. Vertical crop (9:16) with face-aware reframe
    if filter_graph.face_crop {
        filters.push("crop=ih*9/16:ih".to_string());
    } else {
        filters.push("crop=ih*9/16:ih:(ow-iw)/2:(oh-ih)/2".to_string());
    }

    // 2. Scale to 1080x1920
    filters.push("scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2".to_string());

    // 3. Zoom punches on emphasis words
    if filter_graph.zoom_punch {
        filters.push("zoompan=z='if(lte(zoom,1.0),1.15,max(1.001,zoom-0.0015))':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=25*5:s=1080x1920".to_string());
    }

    // 4. Progress bar
    if filter_graph.progress_bar {
        let primary_color = theme.primary.trim_start_matches('#');
        filters.push(format!(
            "drawbox=x=0:y=ih-4:w=iw*min(1,t/{duration}):h=4:color={color}@1:t=fill",
            duration = clip.duration_s,
            color = primary_color
        ));
    }

    // Combine video filters
    let video_filter_chain = filters.join(",");

    // ASS subtitles for captions
    let ass_filter = format!(
        "ass='{}'",
        generate_ass_path(clip, template)
    );

    // Build full filter graph
    let full_filter = if video_filter_chain.is_empty() {
        ass_filter
    } else {
        format!("{},{}", video_filter_chain, ass_filter)
    };

    Ok(full_filter)
}

fn generate_ass_path(clip: &Clip, _template: &Template) -> String {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("clipper")
        .join("ass");
    std::fs::create_dir_all(&cache_dir).unwrap();
    cache_dir.join(format!("{}.ass", clip.id)).to_string_lossy().to_string()
}

pub fn generate_ass_file(
    clip: &Clip,
    transcript: &[TranscriptSegment],
    template: &Template,
) -> Result<String> {
    let ass_path = generate_ass_path(clip, template);
    let mut ass_content = String::new();

    // ASS Header
    let ass_style = template.ass_style().map_err(|e| anyhow::anyhow!(e))?;
    ass_content.push_str(&format!(
        r#"[Script Info]
Title: {}
ScriptType: v4.00+
WrapStyle: 0
ScaledBorderAndShadow: yes
YCbCr Matrix: TV.709
PlayResX: 1080
PlayResY: 1920

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,{},{},&H{}&,{},{},{},-1,0,0,0,100,100,0,0,1,{},{},{},{},0,0,{},1
Style: Karaoke,{},{},&H{}&,{},{},{},-1,0,0,0,100,100,0,0,1,{},{},{},{},0,0,{},1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text

"#,
        clip.title,
        ass_style.font,
        ass_style.font_size,
        ass_style.primary_color.trim_start_matches("&H"),
        ass_style.secondary_color.trim_start_matches("&H"),
        ass_style.outline_color.trim_start_matches("&H"),
        ass_style.background.trim_start_matches("&H"),
        ass_style.outline_width,
        ass_style.shadow_offset,
        ass_style.alignment,
        ass_style.margin_v,
        ass_style.margin_v,
        ass_style.font,
        ass_style.font_size,
        ass_style.primary_color.trim_start_matches("&H"),
        ass_style.secondary_color.trim_start_matches("&H"),
        ass_style.outline_color.trim_start_matches("&H"),
        ass_style.background.trim_start_matches("&H"),
        ass_style.outline_width,
        ass_style.shadow_offset,
        ass_style.alignment,
        ass_style.margin_v,
        ass_style.margin_v,
    ));

    // Generate karaoke events for each word
    for segment in transcript {
        for word in &segment.words {
            let start_hms = format_time_ass(word.start);
            let end_hms = format_time_ass(word.end);

            let effect = match ass_style.karaoke_style.as_str() {
                "pop_in" => r"{\k0\t(\fscx150\fscy150)\t(100,\fscx100\fscy100)}",
                "highlight" => r"{\k0\t(\c&HFFFF00&)\t(100,\c&HFFFFFF&)}",
                "fade" => r"{\k0\fad(100,100)}",
                _ => "",
            };

            ass_content.push_str(&format!(
                "Dialogue: 0,{},{},Karaoke,,0,0,0,{} {}\n",
                start_hms, end_hms, effect, word.text
            ));
        }
    }

    std::fs::write(&ass_path, ass_content)?;

    Ok(ass_path)
}

fn format_time_ass(seconds: f64) -> String {
    let hours = (seconds / 3600.0).floor() as i64;
    let minutes = ((seconds % 3600.0) / 60.0).floor() as i64;
    let secs = (seconds % 60.0).floor() as i64;
    let centiseconds = ((seconds.fract() * 100.0).round() as i64).clamp(0, 99);

    format!("{:01}:{:02}:{:02}.{:02}", hours, minutes, secs, centiseconds)
}

async fn run_ffmpeg_nvenc(
    _app_handle: tauri::AppHandle,
    input_path: &str,
    output_path: &str,
    filter_graph: &str,
    start_time: f64,
    duration: f64,
) -> Result<()> {
    use tracing::{info, warn};

    // Resolve FFmpeg explicitly to bypass stale PATH environments in long-running terminals
    let ffmpeg_exe = {
        let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| r"C:\Users\debbi\AppData\Local".to_string());
        let winget_path = format!(r"{}\Microsoft\WinGet\Links\ffmpeg.exe", local_appdata);
        if std::path::Path::new(&winget_path).exists() {
            winget_path
        } else {
            "ffmpeg".to_string()
        }
    };

    info!("[FFmpeg] Attempting NVENC render: {} -> {}", input_path, output_path);

    // Try NVENC (GPU) first
    let nvenc_result = tokio::process::Command::new(&ffmpeg_exe)
        .args([
            "-y",
            "-hwaccel", "cuda",
            "-hwaccel_output_format", "cuda",
            "-ss", &start_time.to_string(),
            "-t", &duration.to_string(),
            "-i", input_path,
            "-vf", filter_graph,
            "-c:v", "h264_nvenc",
            "-preset", "p4",
            "-profile:v", "high",
            "-level:v", "4.1",
            "-rc:v", "vbr",
            "-cq:v", "23",
            "-b:v", "5M",
            "-maxrate:v", "8M",
            "-bufsize:v", "10M",
            "-c:a", "aac",
            "-b:a", "128k",
            "-ar", "48000",
            "-af", "anull",
            output_path,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match nvenc_result {
        Ok(output) if output.status.success() => {
            info!("[FFmpeg] NVENC render completed successfully");
            return Ok(());
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("[FFmpeg] NVENC failed, falling back to CPU (libx264): {}", 
                stderr.chars().take(200).collect::<String>());
        }
        Err(e) => {
            warn!("[FFmpeg] NVENC unavailable, falling back to CPU (libx264): {}", e);
        }
    }

    // Fallback to libx264 (CPU)
    info!("[FFmpeg] Using CPU fallback (libx264) for render");
    let cpu_output = tokio::process::Command::new(&ffmpeg_exe)
        .args([
            "-y",
            "-ss", &start_time.to_string(),
            "-t", &duration.to_string(),
            "-i", input_path,
            "-vf", filter_graph,
            "-c:v", "libx264",
            "-preset", "medium",
            "-profile:v", "high",
            "-level:v", "4.1",
            "-crf", "23",
            "-b:v", "5M",
            "-maxrate:v", "8M",
            "-bufsize:v", "10M",
            "-c:a", "aac",
            "-b:a", "128k",
            "-ar", "48000",
            "-af", "anull",
            output_path,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg (CPU fallback): {}", e))?;

    if !cpu_output.status.success() {
        let stderr = String::from_utf8_lossy(&cpu_output.stderr);
        return Err(anyhow::anyhow!("FFmpeg render failed (both NVENC and CPU): {}", 
            stderr.chars().take(500).collect::<String>()));
    }

    info!("[FFmpeg] CPU (libx264) render completed successfully");
    Ok(())
}

pub fn build_jump_cut_filter(silence_regions: &[(f64, f64)], duration: f64) -> String {
    // Build select filter to remove silence regions
    let mut select_parts = Vec::new();
    let mut last_end = 0.0;

    for (start, end) in silence_regions {
        if *start > last_end {
            select_parts.push(format!("between(t,{},{})", last_end, start));
        }
        last_end = *end;
    }

    if last_end < duration {
        select_parts.push(format!("between(t,{},{})", last_end, duration));
    }

    let select_expr = select_parts.join("+");
    format!("select='{}',setpts=N/FRAME_RATE/TB", select_expr)
}

pub fn build_zoom_punch_keyframes(emphasis_times: &[f64], duration: f64) -> String {
    // Generate zoompan keyframes for emphasis moments
    let mut keyframes = Vec::new();

    for &t in emphasis_times {
        if t > 0.0 && t < duration {
            keyframes.push(format!("zoompan=z='if(between(t,{},{}),1.3,1.0)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s=1080x1920", t - 0.1, t + 0.3));
        }
    }

    keyframes.join(",")
}

pub fn build_face_crop_filter(_face_positions: &[(f64, f64, f64, f64)], video_w: i32, video_h: i32) -> String {
    // face_positions: (x, y, w, h) normalized 0-1 for each keyframe time
    // This would generate a dynamic crop filter using sendcmd or keyframe expressions
    // For now return a static center crop
    let crop_w = (video_h as f32 * 9.0 / 16.0) as i32;
    let crop_h = video_h;
    let x = (video_w - crop_w) / 2;
    let y = 0;
    format!("crop={}:{}:{}:{}", crop_w, crop_h, x, y)
}

pub fn build_music_duck_filter() -> String {
    // Sidechain compressor for music ducking
    // -18dB duck, 50ms attack, 200ms release
    "sidechaincompress=threshold=-18dB:ratio=4:attack=50:release=200:makeup=1".to_string()
}

pub fn build_broll_overlay_filter(broll_path: &str, start: f64, end: f64, opacity: f64) -> String {
    format!(
        "movie='{}',scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2,format=rgba,colorchannelmixer=aa={}[broll];[0:v][broll]overlay=enable='between(t,{},{})':alpha={}",
        broll_path, opacity, start, end, opacity
    )
}

pub fn build_sfx_filter(sfx_path: &str, trigger_time: f64, volume: f64) -> String {
    format!(
        "amovie='{}',adelay={}|{}[sfx];[0:a][sfx]amix=inputs=2:weights=1 {}",
        sfx_path,
        (trigger_time * 1000.0) as i64,
        (trigger_time * 1000.0) as i64,
        volume
    )
}