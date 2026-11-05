use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};
use tokio::process::Command;

/// Generate a stunning AI thumbnail for a video using:
/// 1. Pollinations.ai (FLUX model) for AI image generation — 100% free, no key
/// 2. FFmpeg for cinematic color grading + bold text overlay
pub async fn generate_ai_thumbnail(
    title: &str,
    keywords: &[String],
    mood: &str,
    language: &str,
    output_path: &str,
) -> Result<()> {
    let ffmpeg = crate::render::ffmpeg::resolve_ffmpeg();

    // Build a cinematic prompt from the video context
    let visual_context = keywords.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    let prompt = build_thumbnail_prompt(title, &visual_context, mood);

    info!("[Thumbnail] Generating AI image via Pollinations.ai (FLUX)…");
    info!("[Thumbnail] Prompt: {}", prompt);

    // ── 1. Generate AI background via Pollinations.ai (free FLUX) ────────────
    let raw_image_path = format!("{}.raw.jpg", output_path);
    match generate_with_pollinations(&prompt, &raw_image_path).await {
        Ok(()) => {
            info!("[Thumbnail] AI image generated ✓");
            // ── 2. Apply cinematic grading + text overlay ─────────────────────
            apply_cinematic_overlay(
                &raw_image_path,
                title,
                language,
                output_path,
                &ffmpeg,
            ).await?;
            let _ = std::fs::remove_file(&raw_image_path);
        }
        Err(e) => {
            warn!("[Thumbnail] Pollinations failed: {} — falling back to gradient thumbnail", e);
            // Fallback: generate a pure FFmpeg gradient thumbnail (still looks great)
            generate_gradient_thumbnail(title, language, output_path, &ffmpeg).await?;
        }
    }

    info!("[Thumbnail] ✓ → {}", output_path);
    Ok(())
}

// ── Prompt Engineering ───────────────────────────────────────────────────────

pub fn build_thumbnail_prompt(title: &str, visual_context: &str, mood: &str) -> String {
    // Clean title for use in prompts (remove special chars)
    let clean_title = title
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>();

    let style = match mood {
        "intense" | "shocking" => "explosive cinematic scene, dramatic lighting, ultra high contrast, dark background, neon highlights, action",
        "calm" | "mysterious" => "ethereal atmosphere, soft bokeh, moody color grading, professional photo, dark aesthetic",
        "energetic" | "funny" => "vibrant pop art style, bold vivid colors, dynamic composition, bright and punchy",
        "emotional" | "inspiring" => "cinematic portrait, warm golden hour lighting, shallow depth of field, epic composition",
        _ => "cinematic, dramatic lighting, professional photography, vibrant colors, ultra detailed",
    };

    format!(
        "{}, {}, {}, \
        8k uhd, photorealistic, sharp focus, perfect composition, \
        YouTube thumbnail style, eye-catching, no text, no watermark, \
        masterpiece quality",
        clean_title,
        visual_context,
        style
    )
}

// ── Pollinations.ai (free FLUX model) ────────────────────────────────────────

pub async fn generate_with_pollinations(prompt: &str, output_path: &str) -> Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()?;

    // Pollinations.ai: free, no key, FLUX model, direct image URL
    let url = format!(
        "https://image.pollinations.ai/prompt/{}?width=1080&height=1920&model=flux&nologo=true&enhance=true&seed={}",
        urlencoding::encode(prompt),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() % 99999
    );

    info!("[Thumbnail] Calling Pollinations.ai…");
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "Pollinations returned {}", resp.status()
        ));
    }

    let bytes = resp.bytes().await?;
    if bytes.len() < 1000 {
        return Err(anyhow::anyhow!("Pollinations returned empty/invalid image"));
    }

    std::fs::write(output_path, &bytes)?;
    info!("[Thumbnail] Pollinations image: {} bytes", bytes.len());
    Ok(())
}

// ── Cinematic FFmpeg overlay ──────────────────────────────────────────────────

pub async fn apply_cinematic_overlay(
    bg_image: &str,
    title: &str,
    _language: &str,
    output_path: &str,
    ffmpeg: &str,
) -> Result<()> {
    // Sanitise title for FFmpeg drawtext (escape single quotes and colons)
    let safe_title = sanitise_for_drawtext(title);

    // Split title into two lines if long
    let (line1, line2) = split_title(&safe_title);

    // ── FFmpeg filter chain:
    // 1. Scale + force 9:16
    // 2. Cinematic color grade (high contrast, vibrant, warm highlights)
    // 3. Vignette (dark edges for depth)
    // 4. Bottom gradient overlay (so text is always readable)
    // 5. Bold title text with thick stroke + shadow
    // 6. Optional hook/subtitle line
    let grade = "\
        curves=r='0/0 0.12/0.05 0.6/0.65 1/1':\
              g='0/0 0.12/0.05 0.6/0.62 1/0.96':\
              b='0/0 0.12/0.08 0.6/0.55 1/0.88',\
        eq=contrast=1.15:saturation=1.4:brightness=0.02:gamma=0.92,\
        vignette=angle=PI/4:mode=backward";

    let gradient =
        "drawbox=x=0:y=ih*0.45:w=iw:h=ih*0.55:\
         color=black@0.75:t=fill";

    let text1 = if !line2.is_empty() {
        format!(
            "drawtext=fontfile='C\\:/Windows/Fonts/arialbd.ttf':text='{line1}':\
            fontsize=88:fontcolor=white:\
            x=(w-text_w)/2:y=h*0.70:\
            shadowcolor=black@0.9:shadowx=4:shadowy=4:\
            borderw=4:bordercolor=black@0.7",
            line1 = line1
        )
    } else {
        format!(
            "drawtext=fontfile='C\\:/Windows/Fonts/arialbd.ttf':text='{line1}':\
            fontsize=94:fontcolor=white:\
            x=(w-text_w)/2:y=h*0.72:\
            shadowcolor=black@0.9:shadowx=4:shadowy=4:\
            borderw=4:bordercolor=black@0.7",
            line1 = line1
        )
    };

    let text2 = if !line2.is_empty() {
        format!(
            ",drawtext=fontfile='C\\:/Windows/Fonts/arialbd.ttf':text='{line2}':\
            fontsize=88:fontcolor=white:\
            x=(w-text_w)/2:y=h*0.78:\
            shadowcolor=black@0.9:shadowx=4:shadowy=4:\
            borderw=4:bordercolor=black@0.7",
            line2 = line2
        )
    } else {
        String::new()
    };

    // Accent bar: a thin colored line above title
    let accent = "drawbox=x=iw*0.1:y=h*0.675:w=iw*0.8:h=6:color=#FF4757@0.9:t=fill";

    // Compose the full VF string
    let vf = format!(
        "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,setsar=1,\
        {grade},\
        {gradient},\
        {accent},\
        {text1}{text2}",
        grade = grade,
        gradient = gradient,
        accent = accent,
        text1 = text1,
        text2 = text2,
    );

    let out = Command::new(ffmpeg)
        .args([
            "-y",
            "-i", bg_image,
            "-vf", &vf,
            "-frames:v", "1",
            "-q:v", "1",
            output_path,
        ])
        .output()
        .await?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow::anyhow!("FFmpeg overlay failed: {}", stderr));
    }

    Ok(())
}

// ── Pure FFmpeg gradient thumbnail (fallback) ────────────────────────────────

/// Creates a stunning dark gradient thumbnail with no AI — looks great as a fallback.
pub async fn generate_gradient_thumbnail(
    title: &str,
    _language: &str,
    output_path: &str,
    ffmpeg: &str,
) -> Result<()> {
    let safe_title = sanitise_for_drawtext(title);
    let (line1, line2) = split_title(&safe_title);

    // Dynamic gradient color based on hash of title
    let hash = title.bytes().fold(0u64, |acc, b| acc.wrapping_add(b as u64));
    let hue = (hash % 360) as u32;
    let (r1, g1, b1) = hsl_to_rgb(hue, 80, 15);
    let (r2, g2, b2) = hsl_to_rgb((hue + 30) % 360, 70, 8);

    let bg_filter = format!(
        "color=c=#{r1:02x}{g1:02x}{b1:02x}:s=1080x1920[c1];\
        color=c=#{r2:02x}{g2:02x}{b2:02x}:s=1080x1920[c2];\
        [c1][c2]blend=all_expr='A*(1-Y/H)+B*(Y/H)'[grad];\
        [grad]vignette=angle=PI/3:mode=backward[bg]",
        r1=r1, g1=g1, b1=b1, r2=r2, g2=g2, b2=b2
    );

    // Geometric accent lines for depth
    let accents = format!(
        "drawbox=x=0:y=ih*0.55:w=iw:h=4:color=#{h:02x}{h2:02x}ff@0.6:t=fill,\
        drawbox=x=iw*0.05:y=ih*0.56:w=iw*0.9:h=2:color=white@0.15:t=fill",
        h = ((hue * 255 / 360) as u8), h2 = 100u8
    );

    let text1 = format!(
        "drawtext=fontfile='C\\:/Windows/Fonts/arialbd.ttf':text='{line1}':\
        fontsize=96:fontcolor=white:\
        x=(w-text_w)/2:y=h*0.60:\
        shadowcolor=black@0.8:shadowx=3:shadowy=3:\
        borderw=3:bordercolor=black@0.5",
        line1 = line1
    );

    let text2 = if !line2.is_empty() {
        format!(
            ",drawtext=fontfile='C\\:/Windows/Fonts/arialbd.ttf':text='{line2}':\
            fontsize=96:fontcolor=white:\
            x=(w-text_w)/2:y=h*0.69:\
            shadowcolor=black@0.8:shadowx=3:shadowy=3:\
            borderw=3:bordercolor=black@0.5",
            line2 = line2
        )
    } else {
        String::new()
    };

    let vf = format!("{accents},{text1}{text2}", accents = accents, text1 = text1, text2 = text2);

    let out = Command::new(ffmpeg)
        .args([
            "-y",
            "-filter_complex", &format!("{}", bg_filter),
            "-map", "[bg]",
            "-vf", &vf,
            "-frames:v", "1",
            "-q:v", "1",
            output_path,
        ])
        .output()
        .await?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow::anyhow!("Gradient thumbnail failed: {}", stderr));
    }

    Ok(())
}

// ── Utilities ────────────────────────────────────────────────────────────────

pub fn sanitise_for_drawtext(s: &str) -> String {
    s.replace('\'', "")
     .replace(':', " ")
     .replace('\\', "")
     .replace('[', "")
     .replace(']', "")
     .chars()
     .take(80)
     .collect()
}

pub fn split_title(title: &str) -> (String, String) {
    let words: Vec<&str> = title.split_whitespace().collect();
    if words.len() <= 4 || title.len() <= 28 {
        return (title.to_string(), String::new());
    }
    // Try to split near the middle
    let mid = words.len() / 2;
    let line1 = words[..mid].join(" ");
    let line2 = words[mid..].join(" ");
    (line1, line2)
}

/// Convert HSL (hue 0-360, saturation 0-100, lightness 0-100) to RGB (0-255)
pub fn hsl_to_rgb(h: u32, s: u32, l: u32) -> (u8, u8, u8) {
    let h = h as f64 / 360.0;
    let s = s as f64 / 100.0;
    let l = l as f64 / 100.0;

    if s == 0.0 {
        let v = (l * 255.0) as u8;
        return (v, v, v);
    }

    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;

    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);

    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 { t += 1.0; }
    if t > 1.0 { t -= 1.0; }
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 1.0 / 2.0 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    p
}

/// Local heuristic thumbnail generator that uses zero external APIs or network calls.
/// Selects an optimal frame timestamp based on face detection or duration,
/// applies cinematic color contrast + vignette grading, and overlays title typography via ASS.
pub async fn generate_local_heuristic_thumbnail(
    video_path: &str,
    title: &str,
    output_path: &str,
    preferred_timestamp: Option<f64>,
    face_data_json: Option<&str>,
) -> Result<()> {
    let ffmpeg = crate::render::ffmpeg::resolve_ffmpeg();
    let ffprobe = crate::render::ffmpeg::resolve_ffprobe();
    
    // 1. Probe duration if timestamp not given
    let mut ts = preferred_timestamp.unwrap_or(0.0);
    if ts <= 0.0 {
        // Try to pick from face data if available
        let mut face_ts = None;
        if let Some(json_str) = face_data_json {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(samples) = v["samples"].as_array() {
                    let best = samples.iter().max_by(|a, b| {
                        let conf_a = a["conf"].as_f64().unwrap_or(0.0);
                        let conf_b = b["conf"].as_f64().unwrap_or(0.0);
                        conf_a.partial_cmp(&conf_b).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    if let Some(b) = best {
                        if let Some(t) = b["t"].as_f64() {
                            face_ts = Some(t);
                        }
                    }
                }
            }
        }
        
        if let Some(fts) = face_ts {
            ts = fts;
        } else {
            let dur_output = Command::new(&ffprobe)
                .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", video_path])
                .output()
                .await;
            let dur = dur_output.ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
                .unwrap_or(5.0);
            ts = (dur * 0.35).clamp(0.5, dur - 0.5);
        }
    }
    
    let is_ar = crate::render::captions::contains_arabic(title);
    let font_name = if is_ar { "Tajawal ExtraBold" } else { "Poppins ExtraBold" };
    let clean_title = crate::render::captions::clean_ass_text(title, !is_ar);
    
    // Resolve fonts dir
    let assets = crate::render::ffmpeg::resolve_assets();
    let fonts_dir = assets.fonts_dir.replace('\\', "/");
    
    // Build temporary ASS file
    let temp_ass = std::env::temp_dir().join(format!("thumb_{}.ass", uuid::Uuid::new_v4().simple()));
    let ass_content = format!(
        "[Script Info]\nTitle: Thumbnail\nScriptType: v4.00+\nWrapStyle: 0\nPlayResX: 1080\nPlayResY: 1920\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Title,{font},96,&H0000FFFF,&H00FFFFFF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,16,6,5,80,80,0,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:00.00,0:00:10.00,Title,,0,0,0,,{{\\an5\\pos(540,1450)}}{t}\n",
        font = font_name,
        t = clean_title
    );
    std::fs::write(&temp_ass, &ass_content)?;
    
    let ass_esc = temp_ass.to_string_lossy().replace('\\', "/").replace(':', "\\:");
    let vf = format!(
        "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,setsar=1,eq=contrast=1.12:saturation=1.2:brightness=0.02,vignette=angle=PI/4:mode=backward,subtitles='{ass}':fontsdir='{fonts}'",
        ass = ass_esc,
        fonts = fonts_dir
    );
    
    let res = Command::new(&ffmpeg)
        .args(["-y", "-ss", &format!("{:.2}", ts), "-i", video_path, "-vf", &vf, "-frames:v", "1", "-q:v", "2", output_path])
        .output()
        .await;
        
    let _ = std::fs::remove_file(&temp_ass);
    
    match res {
        Ok(out) if out.status.success() && std::path::Path::new(output_path).exists() => Ok(()),
        Ok(out) => Err(anyhow::anyhow!("FFmpeg thumbnail failed: {}", String::from_utf8_lossy(&out.stderr))),
        Err(e) => Err(anyhow::anyhow!("FFmpeg execution failed: {}", e)),
    }
}
