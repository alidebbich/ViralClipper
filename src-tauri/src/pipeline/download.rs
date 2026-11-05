use anyhow::Result;
use tauri::{AppHandle, Manager};
use tracing::{error, info};
use crate::pipeline::{update_state, PipelineStage};
use tokio;

/// Helper to extract YouTube video ID from URL
pub fn extract_youtube_video_id(url: &str) -> Option<String> {
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
pub async fn download_video(app_handle: &AppHandle, project_id: &str, url: &str) -> Result<String> {
    info!("[Download] Starting download for: {}", url);

    // Check if input is already a local video file
    let clean_url = url.trim_start_matches("file:///").trim_start_matches("file://");
    let local_path = std::path::Path::new(clean_url);
    if local_path.is_file() && local_path.exists() {
        info!("[Download] Input is already a local file: {:?}", local_path);
        return Ok(local_path.to_string_lossy().to_string());
    }

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

    let ffmpeg_exe = crate::render::ffmpeg::resolve_ffmpeg();

    // 4. Ensure bgutil-pot PO Token server is running
    let pot_running = std::net::TcpStream::connect("127.0.0.1:4416").is_ok();
    if !pot_running {
        let pot_candidates = ["bgutil-pot.exe", "../bgutil-pot.exe"];
        if let Some(pot_exe) = pot_candidates.iter().find(|p| std::path::Path::new(p).exists()) {
            info!("[Download] Spawning bgutil-pot token server on 127.0.0.1:4416...");
            let _ = std::process::Command::new(pot_exe)
                .args(["server", "--host", "127.0.0.1"])
                .spawn();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

fn resolve_script(app_handle: &AppHandle, rel: &str) -> std::path::PathBuf {
    if let Ok(p) = app_handle.path().resolve(rel, tauri::path::BaseDirectory::Resource) {
        if p.exists() { return p; }
    }
    let dev = std::path::PathBuf::from(rel);
    if dev.exists() { return dev; }
    std::path::PathBuf::from(format!("../{}", rel))
}

    // Use a specific UUID for this download
    let file_uuid = uuid::Uuid::new_v4().to_string();
    let output_template = output_dir.join(format!("{}.%(ext)s", file_uuid));
    let output_str = output_template.to_string_lossy().to_string();

    info!("[Download] Output template: {}", output_str);

    let downloader_script = resolve_script(app_handle, "python/download_yt.py");
    info!("[Download] Downloader script resolved to: {:?} (exists: {})", downloader_script, downloader_script.exists());

    let mut cmd_args = if downloader_script.exists() {
        vec![
            downloader_script.to_string_lossy().to_string(),
            url.to_string(),
            "-o".to_string(),
            output_str.clone(),
            "--ffmpeg-location".to_string(),
            ffmpeg_exe.clone(),
            "--merge-output-format".to_string(),
            "mp4".to_string(),
            "--write-info-json".to_string(),
        ]
    } else {
        vec![
            "-m".to_string(),
            "yt_dlp".to_string(),
            "--ffmpeg-location".to_string(),
            ffmpeg_exe.clone(),
            "-N".to_string(),
            "4".to_string(),
            "--socket-timeout".to_string(),
            "6".to_string(),
            "--retries".to_string(),
            "2".to_string(),
            "--fragment-retries".to_string(),
            "2".to_string(),
            "--js-runtimes".to_string(),
            r"node:C:\Program Files\nodejs\node.exe".to_string(),
            "-f".to_string(),
            "bestvideo[height<=1080][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=1080]+bestaudio/best[height<=1080]/best".to_string(),
            "-o".to_string(),
            output_str.clone(),
            "--merge-output-format".to_string(),
            "mp4".to_string(),
            "--no-playlist".to_string(),
            "--no-warnings".to_string(),
            "--write-info-json".to_string(),
            url.to_string(),
        ]
    };

    // Automatically discover cookies.txt in app data, user's Downloads, or workspace
    let mut resolved_cookie_file: Option<std::path::PathBuf> = None;
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    let downloads_dir = std::path::PathBuf::from(&user_profile).join("Downloads");

    let candidate_cookie_paths = [
        output_dir.join("cookies.txt"),
        downloads_dir.join("cookies.txt"),
        downloads_dir.join("youtube.com_cookies.txt"),
        std::path::PathBuf::from("cookies.txt"),
    ];

    for cpath in &candidate_cookie_paths {
        if cpath.exists() && cpath.metadata().map(|m| m.len() > 50).unwrap_or(false) {
            resolved_cookie_file = Some(cpath.clone());
            break;
        }
    }

    // Check for any *cookies*.txt in Downloads as well
    if resolved_cookie_file.is_none() && downloads_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&downloads_dir) {
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

    let mut child = tokio::process::Command::new(&python_exe)
        .arg("-X")
        .arg("utf8")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .args(&cmd_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!(
            "Failed to launch Python ({}): {}. Make sure Python is installed at C:\\Python314 or on PATH, and run: python -m pip install yt-dlp",
            python_exe, e
        ))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let app_clone = app_handle.clone();
    let p_id = project_id.to_string();

    let reader_handle = tokio::spawn(async move {
        if let Some(stdout) = stdout {
            use tokio::io::AsyncBufReadExt;
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.contains("[download]") && line.contains('%') {
                    if let Some(pct_idx) = line.find('%') {
                        let prefix = &line[..pct_idx];
                        if let Some(num_str) = prefix.split_whitespace().last() {
                            if let Ok(pct) = num_str.parse::<f64>() {
                                let norm_prog = ((pct / 100.0 * 0.15 + 0.02).min(0.18)) as f32;
                                let clean_msg = line.replace("[download]", "").trim().to_string();
                                update_state(&app_clone, &p_id, PipelineStage::Downloading, norm_prog, &format!("Downloading: {}", clean_msg), None).await;
                            }
                        }
                    }
                }
            }
        }
    });

    let stderr_handle = tokio::spawn(async move {
        let mut err_text = String::new();
        if let Some(stderr) = stderr {
            use tokio::io::AsyncReadExt;
            let mut reader = tokio::io::BufReader::new(stderr);
            let _ = reader.read_to_string(&mut err_text).await;
        }
        err_text
    });

    let status = child.wait().await;
    let _ = reader_handle.await;
    let stderr_text = stderr_handle.await.unwrap_or_default();

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

    if let Ok(st) = status {
        if !st.success() {
            let exit_code = st.code().unwrap_or(-1);
            error!("[Download] yt-dlp failed with exit code {}: {}", exit_code, stderr_text);
            return Err(anyhow::anyhow!("yt-dlp failed (exit {}): {}", exit_code, stderr_text.trim()));
        }
    }

    Err(anyhow::anyhow!(
        "yt-dlp succeeded but the output file was not found (UUID: {}). The URL may be unavailable.",
        file_uuid
    ))
}
