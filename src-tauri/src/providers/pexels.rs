use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

/// Download stock video from Pexels (primary) → Pixabay (fallback) → color placeholder (last resort)
pub async fn download_stock_video(
    keyword: &str,
    output_path: &str,
    duration: f64,
    pexels_api_key: Option<&str>,
) -> Result<()> {
    // 1. Try Pexels
    if let Some(key) = pexels_api_key {
        if !key.trim().is_empty() {
            match download_from_pexels(keyword, output_path, duration, key).await {
                Ok(()) => {
                    info!("[Visuals] Pexels ✓ for '{}'", keyword);
                    return Ok(());
                }
                Err(e) => warn!("[Visuals] Pexels failed for '{}': {}", keyword, e),
            }
        }
    }

    // 2. Try Pixabay (free, no key required for basic use)
    match download_from_pixabay(keyword, output_path, duration).await {
        Ok(()) => {
            info!("[Visuals] Pixabay ✓ for '{}'", keyword);
            return Ok(());
        }
        Err(e) => warn!("[Visuals] Pixabay failed for '{}': {}", keyword, e),
    }

    // 3. Generate branded placeholder (better than a plain blue screen)
    warn!("[Visuals] No stock footage found for '{}', using branded placeholder", keyword);
    generate_placeholder(keyword, output_path, duration).await
}

async fn download_from_pexels(
    keyword: &str,
    output_path: &str,
    duration: f64,
    api_key: &str,
) -> Result<()> {
    let client = Client::new();
    let url = format!(
        "https://api.pexels.com/videos/search?query={}&per_page=5&orientation=portrait&size=medium",
        urlencoding::encode(keyword)
    );

    let resp = client
        .get(&url)
        .header("Authorization", api_key)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Pexels error: {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await?;
    let videos = json["videos"].as_array()
        .ok_or_else(|| anyhow::anyhow!("No videos in Pexels response"))?;

    if videos.is_empty() {
        return Err(anyhow::anyhow!("No Pexels results for '{}'", keyword));
    }

    // Pick the best 9:16 video file — prefer portrait, highest res ≤ 1080p
    for video in videos.iter().take(3) {
        let video_files = match video["video_files"].as_array() {
            Some(f) => f,
            None => continue,
        };

        // Find the best portrait file
        let best = video_files.iter()
            .filter(|f| {
                let w = f["width"].as_i64().unwrap_or(0);
                let h = f["height"].as_i64().unwrap_or(0);
                h >= w && w <= 1080 // portrait or square, not too large
            })
            .max_by_key(|f| f["width"].as_i64().unwrap_or(0));

        // Fall back to any file if no portrait found
        let file = best.or_else(|| {
            video_files.iter()
                .filter(|f| f["width"].as_i64().unwrap_or(0) <= 1280)
                .max_by_key(|f| f["width"].as_i64().unwrap_or(0))
        });

        if let Some(file) = file {
            if let Some(link) = file["link"].as_str() {
                return download_and_trim(link, output_path, duration).await;
            }
        }
    }

    Err(anyhow::anyhow!("No suitable Pexels video file found for '{}'", keyword))
}

async fn download_from_pixabay(
    keyword: &str,
    output_path: &str,
    duration: f64,
) -> Result<()> {
    // Pixabay allows anonymous API calls with a public key
    let client = Client::new();
    let url = format!(
        "https://pixabay.com/api/videos/?key=47391282-e6a11c4aba1455591de0f621b&q={}&video_type=film&per_page=5",
        urlencoding::encode(keyword)
    );

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Pixabay error: {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await?;
    let hits = json["hits"].as_array()
        .ok_or_else(|| anyhow::anyhow!("No hits in Pixabay response"))?;

    if hits.is_empty() {
        return Err(anyhow::anyhow!("No Pixabay results for '{}'", keyword));
    }

    // Try medium quality first (smaller download)
    for hit in hits.iter().take(3) {
        let medium = hit["videos"]["medium"]["url"].as_str()
            .or_else(|| hit["videos"]["small"]["url"].as_str());

        if let Some(url) = medium {
            match download_and_trim(url, output_path, duration).await {
                Ok(()) => return Ok(()),
                Err(e) => warn!("[Visuals] Pixabay download failed: {}", e),
            }
        }
    }

    Err(anyhow::anyhow!("All Pixabay downloads failed for '{}'", keyword))
}

async fn download_and_trim(url: &str, output_path: &str, duration: f64) -> Result<()> {
    // Download to a temp file then trim with FFmpeg
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Download failed: {}", resp.status()));
    }

    let tmp_path = format!("{}.tmp.mp4", output_path);
    let bytes = resp.bytes().await?;
    std::fs::write(&tmp_path, &bytes)?;

    // Trim + scale to 1080x1920 (9:16 portrait)
    let ffmpeg = crate::render::ffmpeg::resolve_ffmpeg();
    let output = tokio::process::Command::new(&ffmpeg)
        .args([
            "-y",
            "-i", &tmp_path,
            "-t", &duration.to_string(),
            "-vf", "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,setsar=1",
            "-c:v", "libx264",
            "-preset", "fast",
            "-crf", "23",
            "-an", // no audio — TTS replaces it
            output_path,
        ])
        .output()
        .await?;

    let _ = std::fs::remove_file(&tmp_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("FFmpeg trim failed: {}", stderr));
    }

    Ok(())
}

/// Branded placeholder: dark gradient + keyword text (much better than a blue screen)
async fn generate_placeholder(keyword: &str, output_path: &str, duration: f64) -> Result<()> {
    let ffmpeg = crate::render::ffmpeg::resolve_ffmpeg();

    // Dark gradient background + centered keyword text
    let drawtext = format!(
        "drawtext=fontfile='C\\:/Windows/Fonts/arial.ttf':text='{}':fontsize=48:fontcolor=white:x=(w-text_w)/2:y=(h-text_h)/2:\
        shadowcolor=black:shadowx=2:shadowy=2",
        keyword.replace('\'', "\\'")
    );

    let output = tokio::process::Command::new(&ffmpeg)
        .args([
            "-y",
            "-f", "lavfi",
            "-i", &format!("color=c=0x0a0a0f:s=1080x1920:d={}", duration),
            "-vf", &drawtext,
            "-c:v", "libx264",
            "-preset", "ultrafast",
            "-crf", "28",
            output_path,
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Placeholder gen failed: {}", stderr));
    }

    Ok(())
}
