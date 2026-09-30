// Pipeline module - Rust sidecar for orchestrating Python pipeline

use std::process::Command;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use anyhow::Result;
use tokio::process::Command as AsyncCommand;
use tauri::AppHandle;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PipelineConfig {
    pub python_path: String,
    pub script_dir: String,
    pub work_dir: String,
    pub models_dir: String,
    pub assets_dir: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectInput {
    pub id: String,
    pub url: String,
    pub context: String,
    pub template: String,
    pub target_clips: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineProgress {
    pub project_id: String,
    pub stage: String,
    pub progress: f32,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClipResult {
    pub id: String,
    pub project_id: String,
    pub start: f32,
    pub end: f32,
    pub duration: f32,
    pub title: String,
    pub hook: String,
    pub viral_score: f32,
    pub heuristic_score: f32,
    pub reason: String,
    pub render_path: Option<String>,
    pub status: String,
}

pub struct PipelineRunner {
    config: PipelineConfig,
    app_handle: Option<AppHandle>,
}

impl PipelineRunner {
    pub fn new(config: PipelineConfig) -> Self {
        Self {
            config,
            app_handle: None,
        }
    }
    
    pub fn with_app_handle(mut self, handle: AppHandle) -> Self {
        self.app_handle = Some(handle);
        self
    }
    
    fn emit_progress(&self, progress: PipelineProgress) {
        if let Some(handle) = &self.app_handle {
            let _ = handle.emit("pipeline-progress", &progress);
        }
    }
    
    pub async fn run_pipeline(&self, input: ProjectInput) -> Result<Vec<ClipResult>> {
        let project_id = &input.id;
        
        // Stage 1: Download
        self.emit_progress(PipelineProgress {
            project_id: project_id.clone(),
            stage: "downloading".to_string(),
            progress: 0.05,
            message: "Downloading video...".to_string(),
        });
        
        let video_path = self.download_video(&input.url, project_id).await?;
        
        // Stage 2: Extract audio
        self.emit_progress(PipelineProgress {
            project_id: project_id.clone(),
            stage: "extracting_audio".to_string(),
            progress: 0.15,
            message: "Extracting audio...".to_string(),
        });
        
        let audio_path = self.extract_audio(&video_path, project_id).await?;
        
        // Stage 3: Transcribe
        self.emit_progress(PipelineProgress {
            project_id: project_id.clone(),
            stage: "transcribing".to_string(),
            progress: 0.25,
            message: "Transcribing with Whisper...".to_string(),
        });
        
        let transcript_path = self.transcribe_audio(&audio_path, project_id).await?;
        
        // Stage 4: Analyze clips
        self.emit_progress(PipelineProgress {
            project_id: project_id.clone(),
            stage: "analyzing".to_string(),
            progress: 0.55,
            message: "Analyzing clips with heuristics + LLM...".to_string(),
        });
        
        let clips = self.analyze_clips(&transcript_path, &audio_path, project_id, &input).await?;
        
        // Stage 5: Render clips
        self.emit_progress(PipelineProgress {
            project_id: project_id.clone(),
            stage: "rendering".to_string(),
            progress: 0.7,
            message: "Rendering clips with creative effects...".to_string(),
        });
        
        let rendered_clips = self.render_clips(&video_path, &clips, project_id, &input).await?;
        
        // Complete
        self.emit_progress(PipelineProgress {
            project_id: project_id.clone(),
            stage: "completed".to_string(),
            progress: 1.0,
            message: "Done!".to_string(),
        });
        
        Ok(rendered_clips)
    }
    
    async fn download_video(&self, url: &str, project_id: &str) -> Result<String> {
        let output_dir = PathBuf::from(&self.config.work_dir).join(project_id);
        std::fs::create_dir_all(&output_dir)?;
        
        let output_template = output_dir.join("source.%(ext)s");
        
        let mut cmd = AsyncCommand::new("yt-dlp");
        cmd.args([
            "-f", "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best",
            "-o", output_template.to_str().unwrap(),
            "--merge-output-format", "mp4",
            "--no-playlist",
            "--write-info-json",
            url,
        ]);
        
        let output = cmd.output().await?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Download failed: {}", stderr));
        }
        
        // Find downloaded video
        let entries = std::fs::read_dir(&output_dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "mp4" || ext == "mkv" || ext == "webm") {
                if path.file_name().unwrap().to_string_lossy().starts_with("source") {
                    return Ok(path.to_string_lossy().to_string());
                }
            }
        }
        
        Err(anyhow::anyhow!("No video file found after download"))
    }
    
    async fn extract_audio(&self, video_path: &str, project_id: &str) -> Result<String> {
        let output_dir = PathBuf::from(&self.config.work_dir).join(project_id);
        let audio_path = output_dir.join("audio.wav");
        
        let mut cmd = AsyncCommand::new("ffmpeg");
        cmd.args([
            "-y", "-hide_banner", "-loglevel", "error",
            "-i", video_path,
            "-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1",
            audio_path.to_str().unwrap(),
        ]);
        
        let output = cmd.output().await?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Audio extraction failed: {}", stderr));
        }
        
        Ok(audio_path.to_string_lossy().to_string())
    }
    
    async fn transcribe_audio(&self, audio_path: &str, project_id: &str) -> Result<String> {
        let output_dir = PathBuf::from(&self.config.work_dir).join(project_id);
        let transcript_path = output_dir.join("transcript.json");
        let ass_path = output_dir.join("captions.ass");
        
        // Run Python transcription script
        let script_path = PathBuf::from(&self.config.script_dir).join("transcribe.py");
        
        let mut cmd = AsyncCommand::new(&self.config.python_path);
        cmd.args([
            script_path.to_str().unwrap(),
            audio_path,
            "-o", transcript_path.to_str().unwrap(),
            "-f", "json",
            "--model", "large-v3",
            "--device", "auto",
            "--compute-type", "int8_float16",
        ]);
        
        let output = cmd.output().await?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Transcription failed: {}", stderr));
        }
        
        // Also generate ASS
        let mut cmd = AsyncCommand::new(&self.config.python_path);
        cmd.args([
            script_path.to_str().unwrap(),
            audio_path,
            "-o", ass_path.to_str().unwrap(),
            "-f", "ass",
            "--model", "large-v3",
        ]);
        
        let _ = cmd.output().await; // Ignore errors for ASS
        
        Ok(transcript_path.to_string_lossy().to_string())
    }
    
    async fn analyze_clips(
        &self,
        transcript_path: &str,
        audio_path: &str,
        project_id: &str,
        input: &ProjectInput,
    ) -> Result<Vec<ClipResult>> {
        let output_dir = PathBuf::from(&self.config.work_dir).join(project_id);
        let analysis_path = output_dir.join("analysis.json");
        
        let script_path = PathBuf::from(&self.config.script_dir).join("heuristics.py");
        
        let mut cmd = AsyncCommand::new(&self.config.python_path);
        cmd.args([
            script_path.to_str().unwrap(),
            transcript_path,
            "-c", &input.context,
            "-k", &input.target_clips.to_string(),
            "-o", analysis_path.to_str().unwrap(),
        ]);
        
        let output = cmd.output().await?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Analysis failed: {}", stderr));
        }
        
        // Parse results
        let analysis_json = std::fs::read_to_string(&analysis_path)?;
        let analysis: serde_json::Value = serde_json::from_str(&analysis_json)?;
        
        let clips = analysis["clips"].as_array().unwrap_or(&vec![]);
        
        let mut results = Vec::new();
        for (i, clip) in clips.iter().enumerate() {
            results.push(ClipResult {
                id: format!("{}_clip_{}", project_id, i + 1),
                project_id: project_id.to_string(),
                start: clip["start"].as_f64().unwrap_or(0.0) as f32,
                end: clip["end"].as_f64().unwrap_or(0.0) as f32,
                duration: clip["duration"].as_f64().unwrap_or(0.0) as f32,
                title: clip["title"].as_str().unwrap_or("Untitled").to_string(),
                hook: clip["hook"].as_str().unwrap_or("").to_string(),
                viral_score: clip["viral_score"].as_f64().unwrap_or(0.0) as f32,
                heuristic_score: clip["heuristic_score"].as_f64().unwrap_or(0.0) as f32,
                reason: clip["reason"].as_str().unwrap_or("").to_string(),
                render_path: None,
                status: "pending".to_string(),
            });
        }
        
        Ok(results)
    }
    
    async fn render_clips(
        &self,
        video_path: &str,
        clips: &[ClipResult],
        project_id: &str,
        input: &ProjectInput,
    ) -> Result<Vec<ClipResult>> {
        let output_dir = PathBuf::from(&self.config.work_dir).join(project_id).join("output");
        std::fs::create_dir_all(&output_dir)?;
        
        let script_path = PathBuf::from(&self.config.script_dir).join("render.py");
        let ass_path = PathBuf::from(&self.config.work_dir).join(project_id).join("captions.ass");
        
        let mut rendered = Vec::new();
        
        for (i, clip) in clips.iter().enumerate() {
            self.emit_progress(PipelineProgress {
                project_id: project_id.to_string(),
                stage: "rendering".to_string(),
                progress: 0.7 + (i as f32 / clips.len() as f32) * 0.3,
                message: format!("Rendering clip {}/{}", i + 1, clips.len()),
            });
            
            let clip_output = output_dir.join(format!("{}.mp4", clip.id));
            
            let mut cmd = AsyncCommand::new(&self.config.python_path);
            cmd.args([
                script_path.to_str().unwrap(),
                video_path,
                clip_output.to_str().unwrap(),
                "--start", &clip.start.to_string(),
                "--end", &clip.end.to_string(),
                "--title", &clip.title,
                "--ass", ass_path.to_str().unwrap(),
            ]);
            
            let output = cmd.output().await?;
            
            let mut updated_clip = clip.clone();
            if output.status.success() {
                updated_clip.render_path = Some(clip_output.to_string_lossy().to_string());
                updated_clip.status = "completed".to_string();
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                updated_clip.status = "error".to_string();
                eprintln!("Render failed for clip {}: {}", clip.id, stderr);
            }
            
            rendered.push(updated_clip);
        }
        
        Ok(rendered)
    }
}

// Tauri commands
#[tauri::command]
pub async fn start_pipeline(
    app: tauri::AppHandle,
    input: ProjectInput,
) -> Result<Vec<ClipResult>, String> {
    let config = PipelineConfig {
        python_path: "python".to_string(),
        script_dir: "python".to_string(),
        work_dir: "clipper_workspace".to_string(),
        models_dir: "models".to_string(),
        assets_dir: "assets".to_string(),
    };
    
    let runner = PipelineRunner::new(config).with_app_handle(app);
    runner.run_pipeline(input).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_dependencies() -> Result<serde_json::Value, String> {
    let mut deps = serde_json::Map::new();
    
    // Check yt-dlp
    deps.insert("yt_dlp".to_string(), check_command("yt-dlp", &["--version"]).await);
    
    // Check ffmpeg
    deps.insert("ffmpeg".to_string(), check_command("ffmpeg", &["-version"]).await);
    
    // Check python
    deps.insert("python".to_string(), check_command("python", &["--version"]).await);
    
    // Check ollama
    deps.insert("ollama".to_string(), check_command("ollama", &["--version"]).await);
    
    Ok(serde_json::Value::Object(deps))
}

async fn check_command(cmd: &str, args: &[&str]) -> serde_json::Value {
    match AsyncCommand::new(cmd).args(args).output().await {
        Ok(output) => {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                serde_json::json!({"available": true, "version": version})
            } else {
                serde_json::json!({"available": false, "error": String::from_utf8_lossy(&output.stderr)})
            }
        }
        Err(e) => serde_json::json!({"available": false, "error": e.to_string()}),
    }
}