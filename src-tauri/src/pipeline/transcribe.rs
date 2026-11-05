use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Manager, Emitter};
use crate::pipeline::TRANSCRIBE_TIMEOUT_SECS;
use tracing::{info, debug};

use crate::common::Transcript;
use crate::pipeline::PIPELINES;
use crate::pipeline::get_key_from_env_file;

/// Transcribe video - ffmpeg audio extraction + Python transcribe.py (streaming JSON lines)
pub async fn transcribe_video(
    app_handle: &AppHandle,
    video_path: &str,
    project_id: &str,
    pool: &Arc<std::sync::Mutex<rusqlite::Connection>>,
    language: &str,
) -> Result<Transcript> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;
    use std::process::Stdio;

    info!("[Transcribe] Starting transcription for: {}", video_path);
    let t_start = Instant::now();

    // ── Detect ffmpeg ─────────────────────────────────────────────────────
    let ffmpeg_exe = {
        let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let winget = format!(r"{}\Microsoft\WinGet\Links\ffmpeg.exe", local_appdata);
        if std::path::Path::new(&winget).exists() { winget } else { "ffmpeg".to_string() }
    };

    // ── Step 1: Extract audio (16 kHz mono WAV) ───────────────────────────
    let stripped = video_path
        .trim_end_matches(".mp4")
        .trim_end_matches(".webm")
        .trim_end_matches(".mkv")
        .trim_end_matches(".m4v");
    let audio_path = format!("{}.audio.wav", stripped);
    let audio_exists = std::path::Path::new(&audio_path).exists()
        && std::fs::metadata(&audio_path).map(|m| m.len() > 100_000).unwrap_or(false);

    if !audio_exists {
        info!("[Transcribe] Extracting 16kHz mono WAV from {}", video_path);
        emit_progress(app_handle, project_id, 0.06, "Extracting audio...").await;

        let extract_out = Command::new(&ffmpeg_exe)
            .args(["-y", "-i", video_path, "-vn",
                   "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1",
                   &audio_path])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("ffmpeg spawn failed: {}. Is ffmpeg installed?", e))?;

        if !extract_out.status.success() {
            return Err(anyhow::anyhow!(
                "ffmpeg audio extraction failed: {}",
                String::from_utf8_lossy(&extract_out.stderr).chars().take(400).collect::<String>()
            ));
        }
        info!("[Transcribe] Audio extracted in {:.1}s", t_start.elapsed().as_secs_f64());
    } else {
        info!("[Transcribe] Reusing cached audio: {}", audio_path);
    }

    // ── Step 2: Transcribe via Python (streaming JSON lines) ──────────────
    let python_exe = find_python();
    let transcribe_script = resolve_script(app_handle, "python/transcribe.py");

    let groq_key = std::env::var("GROQ_API_KEY").ok().filter(|k| !k.trim().is_empty())
        .or_else(|| get_key_from_env_file("GROQ_API_KEY"))
        .or_else(|| {
            let conn = pool.lock().ok()?;
            crate::storage::get_user_preference_internal(&conn, "groq_api_key")
                .ok()
                .flatten()
                .map(|p| p.value_json.trim_matches('"').to_string())
                .filter(|k| !k.trim().is_empty())
        }).unwrap_or_default();

    let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let cache_dir = format!(r"{}\clipper\transcript_cache", local_appdata);

    // Stable models directory in APPDATA
    let models_dir = {
        let appdata = std::env::var("APPDATA")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
        let dir = std::path::PathBuf::from(appdata).join("ViralClipper").join("models");
        std::fs::create_dir_all(&dir).ok();
        dir.to_string_lossy().into_owned()
    };

    info!("[Transcribe] python={} | groq={} | models_dir={}",
        python_exe,
        if groq_key.is_empty() { "no key (local fallback)" } else { "key configured" },
        models_dir);

    let mut cmd = Command::new(&python_exe);
    cmd.arg(transcribe_script.to_str().unwrap_or("python/transcribe.py"))
       .arg(&audio_path)
       .arg("--download-root").arg(&models_dir)
       .arg("--models-dir").arg(&models_dir)
       .arg("--cache-dir").arg(&cache_dir)
       .arg("--lang").arg(language)
       .env("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")
       .env("PYTHONUNBUFFERED", "1");

    if !groq_key.is_empty() {
        cmd.env("GROQ_API_KEY", &groq_key);
    }

    cmd.stdout(Stdio::piped())
       .stderr(Stdio::piped())
       .kill_on_drop(true);

    let mut child = cmd.spawn()
        .map_err(|e| anyhow::anyhow!(
            "Failed to spawn Python ({}): {}. Make sure python is on PATH.", python_exe, e
        ))?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::debug!("[py stderr] {}", line);
        }
    });

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut final_transcript: Option<Transcript> = None;
    let mut last_error: Option<String> = None;

    let overall_deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(TRANSCRIBE_TIMEOUT_SECS);

    loop {
        let now = tokio::time::Instant::now();
        if now >= overall_deadline {
            let _ = child.kill().await;
            return Err(anyhow::anyhow!(
                "Transcription timed out after {}s",
                TRANSCRIBE_TIMEOUT_SECS
            ));
        }

        // 60-second stall watchdog
        let time_left = (overall_deadline - now).min(std::time::Duration::from_secs(60));
        let line_result = tokio::time::timeout(
            time_left,
            stdout_reader.next_line()
        ).await;

        match line_result {
            Err(_) => {
                let _ = child.kill().await;
                return Err(anyhow::anyhow!(
                    "Transcription stalled: no progress received from Python for 60s"
                ));
            }
            Ok(Err(e)) => {
                return Err(anyhow::anyhow!("I/O error reading transcription output: {}", e));
            }
            Ok(Ok(None)) => {
                // EOF — process has exited cleanly
                break;
            }
            Ok(Ok(Some(line))) => {
                let line = line.trim().to_string();
                if line.is_empty() { continue; }

                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(val) => {
                        match val.get("type").and_then(|t| t.as_str()) {
                            Some("progress") => {
                                let pct = val.get("pct").and_then(|p| p.as_f64()).unwrap_or(0.0);
                                let msg = val.get("msg").and_then(|m| m.as_str()).unwrap_or("Transcribing...");
                                // Map Python 0-100 into our 0.05-0.35 pipeline band
                                let mapped = 0.05 + (pct / 100.0) * 0.30;
                                emit_progress(app_handle, project_id, mapped as f32, msg).await;
                            }
                            Some("done") => {
                                if let Some(tv) = val.get("transcript") {
                                    match serde_json::from_value::<Transcript>(tv.clone()) {
                                        Ok(t) => { final_transcript = Some(t); }
                                        Err(e) => { last_error = Some(format!("Transcript parse error: {}", e)); }
                                    }
                                }
                            }
                            Some("error") => {
                                let msg = val.get("msg").and_then(|m| m.as_str()).unwrap_or("unknown error");
                                last_error = Some(msg.to_string());
                            }
                            _ => {
                                debug!("[Transcribe] unrecognised line: {}", &line[..line.len().min(120)]);
                            }
                        }
                    }
                    Err(_) => {
                        debug!("[Transcribe] non-JSON stdout: {}", &line[..line.len().min(120)]);
                    }
                }
            }
        }
    }

    let exit_status = child.wait().await
        .map_err(|e| anyhow::anyhow!("Failed to wait for Python process: {}", e))?;
    let _ = stderr_task.await;

    if let Some(err) = last_error {
        return Err(anyhow::anyhow!("Transcription error: {}", err));
    }
    if !exit_status.success() {
        return Err(anyhow::anyhow!("Python exited with code {:?}", exit_status.code()));
    }

    let transcript = final_transcript.ok_or_else(|| {
        anyhow::anyhow!("Transcription completed without output — audio may be silent or model failed to load")
    })?;

    info!("[Transcribe] Done: {} segments, {:.1}s audio, elapsed {:.1}s",
        transcript.segments.len(), transcript.duration, t_start.elapsed().as_secs_f64());

    Ok(transcript)
}
pub fn find_python() -> String {
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
        .unwrap_or_else(|| "python".to_string())
}
fn resolve_script(app_handle: &AppHandle, rel: &str) -> std::path::PathBuf {
    if let Ok(p) = app_handle.path().resolve(rel, tauri::path::BaseDirectory::Resource) {
        if p.exists() { return p; }
    }
    let dev = std::path::PathBuf::from(rel);
    if dev.exists() { return dev; }
    std::path::PathBuf::from(format!("../{}", rel))
}
async fn emit_progress(app_handle: &AppHandle, project_id: &str, progress: f32, message: &str) {
    let mut pipelines = PIPELINES.lock().await;
    if let Some(state) = pipelines.get_mut(project_id) {
        state.progress = progress;
        state.message = message.to_string();
        let clone = state.clone();
        drop(pipelines);
        let _ = app_handle.emit("pipeline-update", &clone);
    }
}
