//! ffmpeg.rs - renders one clip: plan -> captions -> filter graph -> encode.
//!
//! `render_clip` keeps its old signature. Flow:
//!  1. clip audio -> clip-level transcription (word times start at 0 for THIS clip)
//!  2. probe the source, find the speaker's face (python/face_track.py, cached)
//!  3. plan_edit(): jump cuts, zoom punches, sfx, framing, face pan  (pure, unit-tested)
//!  4. ASS captions generated on the SAME timeline, then ONE ffmpeg pass with NVENC (x264 fallback)

use anyhow::Result;
use tracing::{info, warn};

use super::captions::{collect_clip_words, generate_ass_file_with_timeline, RelWord};
use super::edit_plan::*;
use super::presets::*;
use crate::common::{Clip, TranscriptSegment};
use crate::templates::Template;

// ───────────────────────── tool discovery ─────────────────────────

fn tool(name: &str) -> String {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = format!(r"{}\Microsoft\WinGet\Links\{}.exe", local, name);
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    name.to_string()
}
pub fn resolve_ffmpeg() -> String {
    tool("ffmpeg")
}
pub fn resolve_ffprobe() -> String {
    tool("ffprobe")
}

fn data_dir() -> std::path::PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("clipper")
}

// ───────────────────────── assets (fonts, sfx, music, backgrounds) ─────────────────────────

#[derive(Debug, Clone, Default)]
pub struct RenderAssets {
    pub fonts_dir: String,
    pub sfx: Vec<(String, String)>,
    pub music: Option<String>,
    pub bg_video: Option<String>,
}

fn first_file_with_ext(dir: &std::path::Path, exts: &[&str]) -> Option<String> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()).map(|x| exts.contains(&x.to_lowercase().as_str())).unwrap_or(false))
        .collect();
    v.sort();
    v.first().map(|p| p.to_string_lossy().to_string())
}

pub fn resolve_assets() -> RenderAssets {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("CLIPPER_ASSETS") {
        roots.push(p.into());
    }
    roots.push("assets".into());
    roots.push("../assets".into());
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            roots.push(d.join("assets"));
            roots.push(d.join("resources").join("assets"));
            roots.push(d.join("_up_").join("assets"));
        }
    }
    let root = roots.into_iter().find(|r| r.join("fonts").is_dir()).unwrap_or_else(|| "assets".into());
    let fonts = root.join("fonts");
    let fonts_dir = if fonts.is_dir() { fonts } else { std::path::PathBuf::from("C:/Windows/Fonts") };
    let mut sfx = Vec::new();
    for k in ["whoosh", "pop", "hit", "ding"] {
        let f = root.join("sfx").join(format!("{k}.wav"));
        if f.is_file() {
            sfx.push((k.to_string(), f.to_string_lossy().to_string()));
        }
    }
    RenderAssets {
        fonts_dir: fonts_dir.to_string_lossy().replace('\\', "/"),
        sfx,
        music: first_file_with_ext(&root.join("music"), &["mp3", "m4a", "wav", "ogg"]),
        bg_video: first_file_with_ext(&root.join("backgrounds"), &["mp4", "mov", "webm"]),
    }
}

// ───────────────────────── probing ─────────────────────────

pub async fn probe_source(path: &str) -> SourceInfo {
    let out = tokio::process::Command::new(resolve_ffprobe())
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height", "-of", "json", path])
        .output()
        .await;
    let mut info = SourceInfo { w: 1920, h: 1080, has_audio: true };
    if let Ok(o) = out {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&o.stdout) {
            let mut audio = false;
            if let Some(streams) = v["streams"].as_array() {
                for s in streams {
                    match s["codec_type"].as_str() {
                        Some("video") => {
                            if let (Some(w), Some(h)) = (s["width"].as_i64(), s["height"].as_i64()) {
                                info.w = w as i32;
                                info.h = h as i32;
                            }
                        }
                        Some("audio") => audio = true,
                        _ => {}
                    }
                }
                info.has_audio = audio;
            }
        }
    }
    info
}

/// Face positions for the smart crop. Cached per (video, start, duration). Never fails: no face = empty stats.
pub async fn detect_faces(video_path: &str, start_s: f64, dur_s: f64) -> FaceStats {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(format!("{}|{:.2}|{:.2}", video_path, start_s, dur_s).as_bytes());
    if let Ok(m) = std::fs::metadata(video_path) {
        h.update(m.len().to_le_bytes());
    }
    let key = format!("{:x}", h.finalize());
    let cache_dir = data_dir().join("face_cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    let cache_file = cache_dir.join(format!("{}.json", &key[..24]));

    let parse = |raw: &str| -> Option<FaceStats> {
        let v: serde_json::Value = serde_json::from_str(raw.lines().rev().find(|l| l.trim_start().starts_with('{'))?).ok()?;
        let keys = v["keys"]
            .as_array()?
            .iter()
            .filter_map(|k| Some((k.get(0)?.as_f64()?, k.get(1)?.as_f64()?)))
            .collect();
        Some(FaceStats {
            keys,
            found_ratio: v["found_ratio"].as_f64().unwrap_or(0.0),
            spread_ratio: v["spread_ratio"].as_f64().unwrap_or(0.0),
        })
    };
    if let Ok(c) = std::fs::read_to_string(&cache_file) {
        if let Some(f) = parse(&c) {
            return f;
        }
    }
    let script = ["python/face_track.py", "../python/face_track.py", "python/workers/face_track.py"]
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists());
    let Some(script) = script else {
        warn!("[Face] python/face_track.py not found: using blur-fit framing");
        return FaceStats::default();
    };
    let python = crate::pipeline::find_python();
    let run = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        tokio::process::Command::new(&python)
            .args(["-X", "utf8"])
            .arg(&script)
            .arg(video_path)
            .args(["--start", &format!("{:.3}", start_s), "--dur", &format!("{:.3}", dur_s)])
            .output(),
    )
    .await;
    match run {
        Ok(Ok(o)) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            if let Some(f) = parse(&stdout) {
                let _ = std::fs::write(&cache_file, &stdout);
                return f;
            }
            warn!("[Face] unreadable tracker output");
        }
        Ok(Ok(o)) => warn!("[Face] tracker failed: {}", String::from_utf8_lossy(&o.stderr).chars().take(300).collect::<String>()),
        Ok(Err(e)) => warn!("[Face] tracker could not start: {}", e),
        Err(_) => warn!("[Face] tracker timed out"),
    }
    FaceStats::default()
}

// ───────────────────────── the edit plan ─────────────────────────

#[derive(Debug, Clone)]
pub struct EditPlan {
    pub timeline: Timeline,
    pub punches: Vec<Punch>,
    pub sfx: Vec<SfxEvent>,
    pub framing: Framing,
    pub pan_keys: Vec<(f64, f64)>,
}

/// Pure planning step (no IO). Everything downstream uses `plan.timeline`.
pub fn plan_edit(clip: &Clip, template: &Template, words_rel: &[RelWord], src_w: i32, src_h: i32, face: &FaceStats, has_bg: bool) -> EditPlan {
    let fg = template.filter_graph().unwrap_or_default();
    let edl = EdlView::parse(clip.timeline_data.as_deref().or(clip.heuristic_json.as_deref()));
    let dur = clip.duration_s;
    let spans: Vec<Span> = words_rel.iter().map(|w| Span { start: w.start, end: w.end }).collect();

    // 1. jump cuts: silences from the transcript + (snapped) cuts proposed by the Editor Brain
    let mut cuts: Vec<(f64, f64)> = Vec::new();
    if fg.jump_cuts {
        cuts = plan_silence_cuts(&spans, dur, &CutRules::default());
        for c in &edl.cuts {
            if c.0 >= 0.0 && c.1 <= dur + 0.5 {
                if let Some(s) = snap_cut_to_words(*c, &spans) {
                    cuts.push(s);
                }
            }
        }
    }
    let timeline = Timeline::from_cuts(dur, &cuts);

    // 2. zooms on the OUTPUT timeline
    let spans_out: Vec<Span> = spans
        .iter()
        .filter_map(|s| Some(Span { start: timeline.remap(s.start)?, end: timeline.remap_clamped(s.end) }))
        .collect();
    let mut punches = plan_punches(&edl, &timeline, &spans_out, fg.zoom_punch, fg.zoom_amount);
    if fg.zoom_punch && timeline.pieces.len() >= 3 {
        punches.extend(plan_step_zooms(&timeline, 0.08 * fg.zoom_amount));
    }

    // 3. sound effects
    let sfx = plan_sfx(&edl, &timeline, &punches, fg.sfx_layer);

    // 4. framing + face pan (face times are clip time -> remap through the cuts)
    let tmpl_framing = if fg.framing == "auto" && !fg.face_crop { "blur_fit" } else { fg.framing.as_str() };
    let framing = choose_framing(&edl.framing_mode, tmpl_framing, src_w, src_h, face, has_bg);
    let mapped: Vec<(f64, f64)> = face.keys.iter().filter_map(|(t, c)| Some((timeline.remap(*t)?, *c))).collect();
    let pan_keys = if framing == Framing::BlurFit { vec![] } else { smooth_pan_keys(&mapped, 24, 0.04) };

    EditPlan { timeline, punches, sfx, framing, pan_keys }
}

// ───────────────────────── encoding ─────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Encoder {
    Nvenc,
    X264,
}

pub fn build_ffmpeg_args(input: &str, start: f64, dur: f64, g: &EditGraph, out: &str, enc: Encoder) -> Vec<String> {
    let mut a: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-ss".into(), format!("{:.3}", start), "-t".into(), format!("{:.3}", dur), "-i".into(), input.into()];
    for e in &g.extra_inputs {
        if e.loop_forever {
            a.push("-stream_loop".into());
            a.push("-1".into());
        }
        a.push("-i".into());
        a.push(e.path.clone());
    }
    a.push("-filter_complex".into());
    a.push(g.filter_complex.clone());
    a.push("-map".into());
    a.push(g.map_v.clone());
    if let Some(m) = &g.map_a {
        a.push("-map".into());
        a.push(m.clone());
    }
    let preset = std::env::var("CLIPPER_X264_PRESET").unwrap_or_else(|_| "medium".to_string());
    let video: Vec<String> = match enc {
        // high quality VBR: constant-quality target with a ceiling; adaptive quant on
        Encoder::Nvenc => ["-c:v", "h264_nvenc", "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "19", "-b:v", "0", "-maxrate", "16M",
            "-bufsize", "32M", "-spatial-aq", "1", "-temporal-aq", "1", "-profile:v", "high"]
            .iter().map(|s| s.to_string()).collect(),
        Encoder::X264 => ["-c:v", "libx264", "-preset", preset.as_str(), "-crf", "18", "-maxrate", "16M", "-bufsize", "32M", "-profile:v", "high"]
            .iter().map(|s| s.to_string()).collect(),
    };
    a.extend(video);
    for s in [
        "-pix_fmt", "yuv420p", "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-c:a", "aac", "-b:a", "192k",
        "-ar", "48000", "-movflags", "+faststart",
    ] {
        a.push(s.into());
    }
    a.push("-t".into());
    a.push(format!("{:.3}", g.out_duration));
    a.push(out.into());
    a
}

fn tail(s: &str, n: usize) -> String {
    let v: Vec<char> = s.chars().collect();
    v[v.len().saturating_sub(n)..].iter().collect()
}

pub async fn run_graph(input: &str, start: f64, dur: f64, g: &EditGraph, out: &str, prefer_gpu: bool) -> Result<Encoder> {
    let ffmpeg = resolve_ffmpeg();
    let mut order = vec![Encoder::X264];
    if prefer_gpu {
        order.insert(0, Encoder::Nvenc);
    }
    let mut last_err = String::new();
    for enc in order {
        let args = build_ffmpeg_args(input, start, dur, g, out, enc);
        info!("[FFmpeg] {} -> {}", if enc == Encoder::Nvenc { "NVENC" } else { "x264" }, out);
        let o = tokio::process::Command::new(&ffmpeg).args(&args).output().await?;
        if o.status.success() {
            return Ok(enc);
        }
        last_err = tail(&String::from_utf8_lossy(&o.stderr), 1500);
        warn!("[FFmpeg] {} failed, trying next: {}", if enc == Encoder::Nvenc { "NVENC" } else { "x264" }, last_err);
    }
    Err(anyhow::anyhow!("FFmpeg render failed:\n{}", last_err))
}

// ───────────────────────── render (prepared inputs) ─────────────────────────

pub struct PreparedRender<'a> {
    pub video_path: &'a str,
    pub clip: &'a Clip,
    pub template: &'a Template,
    pub out_path: &'a str,
    pub caption_segments: &'a [TranscriptSegment],
    pub captions_relative: bool,
    pub src: SourceInfo,
    pub face: FaceStats,
    pub assets: &'a RenderAssets,
    pub prefer_gpu: bool,
}

pub async fn render_prepared(p: &PreparedRender<'_>) -> Result<EditPlan> {
    let words = collect_clip_words(p.clip, p.caption_segments, p.captions_relative);
    let plan = plan_edit(p.clip, p.template, &words, p.src.w, p.src.h, &p.face, p.assets.bg_video.is_some());
    info!(
        "[Edit] framing={:?} pieces={} out={:.1}s (clip {:.1}s) punches={} sfx={} pan_keys={}",
        plan.framing, plan.timeline.pieces.len(), plan.timeline.out_duration, p.clip.duration_s, plan.punches.len(), plan.sfx.len(), plan.pan_keys.len()
    );

    let ass_path = generate_ass_file_with_timeline(p.clip, p.caption_segments, p.template, p.captions_relative, &plan.timeline)?;
    let params = EditParams {
        template: p.template,
        ass_path: &ass_path,
        fonts_dir: &p.assets.fonts_dir,
        src: p.src.clone(),
        timeline: &plan.timeline,
        framing: plan.framing,
        pan_keys: &plan.pan_keys,
        punches: &plan.punches,
        sfx: &plan.sfx,
        sfx_files: &p.assets.sfx,
        bg_video: p.assets.bg_video.as_deref(),
        music: p.assets.music.as_deref(),
    };
    let graph = build_edit_graph(&params)?;
    let enc = run_graph(p.video_path, p.clip.start_s, p.clip.duration_s, &graph, p.out_path, p.prefer_gpu).await?;
    info!("[Render] done with {}", if enc == Encoder::Nvenc { "NVENC" } else { "x264" });
    Ok(plan)
}

// ───────────────────────── clip audio + transcription (unchanged behavior) ─────────────────────────

pub async fn extract_clip_audio(video_path: &str, out_wav: &str, start_s: f64, duration_s: f64) -> Result<()> {
    let output = tokio::process::Command::new(resolve_ffmpeg())
        .args(["-y", "-ss", &format!("{:.3}", start_s), "-t", &format!("{:.3}", duration_s), "-i", video_path, "-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1", out_wav])
        .output()
        .await?;
    if output.status.success() && std::path::Path::new(out_wav).exists() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to extract clip audio: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

pub async fn transcribe_clip_audio(clip_wav_path: &str, language: &str) -> Result<crate::common::Transcript> {
    let python = crate::pipeline::find_python();
    let script_path = [std::path::PathBuf::from("python/transcribe.py"), std::path::PathBuf::from("../python/transcribe.py")]
        .into_iter()
        .find(|p| p.exists())
        .ok_or_else(|| anyhow::anyhow!("python/transcribe.py not found"))?;
    let mut cmd = tokio::process::Command::new(&python);
    cmd.arg("-X").arg("utf8").arg(&script_path).arg(clip_wav_path);
    if !language.is_empty() && language != "auto" {
        cmd.arg("--lang").arg(language);
    }
    let output = cmd.output().await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("Clip transcribe failed with exit code {:?}", output.status.code()));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if val.get("type").and_then(|t| t.as_str()) == Some("done") {
                if let Some(t_val) = val.get("transcript") {
                    return Ok(serde_json::from_value(t_val.clone())?);
                }
            }
        }
    }
    Err(anyhow::anyhow!("No done line in clip transcription output"))
}

pub fn compute_audio_hash(path: &str) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

// ───────────────────────── public API (signature unchanged) ─────────────────────────

pub async fn render_clip(
    video_path: &str,
    clip: &Clip,
    template: &Template,
    project_id: &str,
    transcript: &crate::common::Transcript,
) -> Result<(String, String, Option<String>)> {
    let base = data_dir();
    let output_dir = base.join("renders").join(project_id);
    std::fs::create_dir_all(&output_dir)?;
    let output_path_str = output_dir.join(format!("{}.mp4", clip.id)).to_string_lossy().to_string();

    // 1. clip audio -> clip-level transcript (timestamps start at 0 for THIS clip), cached by audio hash
    let clip_audio = output_dir.join(format!("{}.audio.wav", clip.id)).to_string_lossy().to_string();
    let _ = extract_clip_audio(video_path, &clip_audio, clip.start_s, clip.duration_s).await;
    let cache_dir = base.join("clip_transcript_cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    let mut caption_warning: Option<String> = None;
    let mut clip_t: Option<crate::common::Transcript> = None;
    if std::path::Path::new(&clip_audio).exists() {
        if let Ok(hash) = compute_audio_hash(&clip_audio) {
            let cache_file = cache_dir.join(format!("{hash}.json"));
            if let Ok(c) = std::fs::read_to_string(&cache_file) {
                if let Ok(mut t) = serde_json::from_str::<crate::common::Transcript>(&c) {
                    t.is_clip_relative = true;
                    clip_t = Some(t);
                }
            }
            if clip_t.is_none() {
                match transcribe_clip_audio(&clip_audio, &transcript.language).await {
                    Ok(mut t) => {
                        t.is_clip_relative = true;
                        if let Ok(s) = serde_json::to_string(&t) {
                            let _ = std::fs::write(&cache_file, s);
                        }
                        clip_t = Some(t);
                    }
                    Err(e) => {
                        warn!("[Render] clip transcription failed ({e}); using project transcript. Caption timing may be off!");
                        caption_warning = Some("caption timing may be off".to_string());
                    }
                }
            }
        }
    } else {
        caption_warning = Some("caption timing may be off".to_string());
    }
    let (segments, relative) = match &clip_t {
        Some(ct) if !ct.segments.is_empty() => (&ct.segments, true),
        _ => (&transcript.segments, transcript.is_clip_relative),
    };

    // 2. source facts + face positions
    let src = probe_source(video_path).await;
    let assets = resolve_assets();
    let want_faces = {
        let fg = template.filter_graph().unwrap_or_default();
        src.w > src.h && fg.framing != "blur_fit"
    };
    let face = if want_faces { detect_faces(video_path, clip.start_s, clip.duration_s).await } else { FaceStats::default() };

    // 3. plan + render
    let prepared = PreparedRender { video_path, clip, template, out_path: &output_path_str, caption_segments: segments, captions_relative: relative, src, face, assets: &assets, prefer_gpu: true };
    render_prepared(&prepared).await?;

    let dur = verify_rendered_file(&output_path_str).await.map_err(|e| anyhow::anyhow!("Render validation failed for clip {}: {}", clip.id, e))?;
    info!("[Render] Clip {} verified ({:.2}s)", clip.id, dur);

    let thumb_dir = base.join("thumbnails");
    std::fs::create_dir_all(&thumb_dir).ok();
    let thumb = thumb_dir.join(format!("{}.jpg", clip.id)).to_string_lossy().to_string();
    let thumb_ts = (dur * 0.35).clamp(0.5, (dur - 0.5).max(0.5));
    let _ = extract_clip_thumbnail(&output_path_str, &thumb, thumb_ts).await;
    Ok((output_path_str, thumb, caption_warning))
}

pub async fn verify_rendered_file(path: &str) -> Result<f64> {
    if !std::path::Path::new(path).exists() {
        return Err(anyhow::anyhow!("Rendered file does not exist: {}", path));
    }
    let len = std::fs::metadata(path)?.len();
    if len < 50_000 {
        return Err(anyhow::anyhow!("Rendered file is suspiciously small ({} bytes)", len));
    }
    let output = tokio::process::Command::new(resolve_ffprobe())
        .args(["-v", "error", "-show_entries", "format=duration:stream=width,height,codec_name", "-of", "json", path])
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to execute ffprobe: {}", e))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("ffprobe failed on {}: {}", path, String::from_utf8_lossy(&output.stderr)));
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| anyhow::anyhow!("Failed to parse ffprobe json: {}", e))?;
    let duration = json["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.0);
    if duration < 1.0 {
        return Err(anyhow::anyhow!("Rendered video duration is invalid ({:.1}s)", duration));
    }
    Ok(duration)
}

pub async fn extract_clip_thumbnail(video_path: &str, output_path: &str, timestamp: f64) -> Result<()> {
    let output = tokio::process::Command::new(resolve_ffmpeg())
        .args(["-y", "-ss", &format!("{:.2}", timestamp), "-i", video_path, "-vframes", "1", "-q:v", "2", output_path])
        .output()
        .await?;
    if output.status.success() && std::path::Path::new(output_path).exists() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to extract thumbnail: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

/// Compatibility wrapper (old signature): pre-built video graph labelled [v], audio of input 0 copied through.
pub async fn run_ffmpeg_nvenc(input_path: &str, output_path: &str, filter_graph: &str, start_time: f64, duration: f64) -> Result<()> {
    let g = EditGraph { filter_complex: filter_graph.to_string(), extra_inputs: vec![], map_v: "[v]".into(), map_a: Some("0:a?".into()), out_duration: duration };
    run_graph(input_path, start_time, duration, &g, output_path, true).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip_with_words() -> (Clip, Vec<RelWord>) {
        let c = Clip {
            id: "t".into(), project_id: "p".into(), title: "T".into(), start_s: 0.0, end_s: 30.0, duration_s: 30.0, heuristic_json: None,
            llm_reason: None, template_id: None, render_path: None, status: "ready".into(), hook_segments: None, montage_order: None,
            hook_score: None, viral_score: None, is_clip_relative: true, caption_warning: None, timeline_data: None,
        };
        let mut w = vec![];
        let mut t = 1.0;
        for i in 0..40 {
            if i == 12 || i == 25 {
                t += 1.2; // two long pauses
            }
            w.push(RelWord { start: t, end: t + 0.35, text: format!("w{i}") });
            t += 0.55;
        }
        (c, w)
    }

    #[test]
    fn plan_has_cuts_zooms_sfx_and_consistent_timeline() {
        let (c, w) = clip_with_words();
        let tpl = crate::templates::default_templates().remove(0);
        let face = FaceStats { keys: (0..30).map(|i| (i as f64, 0.5)).collect(), found_ratio: 0.9, spread_ratio: 0.0 };
        let p = plan_edit(&c, &tpl, &w, 1920, 1080, &face, false);
        assert!(p.timeline.pieces.len() >= 3, "pauses should be cut");
        assert!(p.timeline.out_duration < c.duration_s);
        assert!(!p.punches.is_empty());
        assert!(!p.sfx.is_empty());
        assert_eq!(p.framing, Framing::FaceCrop);
        assert!(p.sfx.iter().all(|s| s.t < p.timeline.out_duration));
    }

    #[test]
    fn minimal_template_stays_quiet() {
        let (c, w) = clip_with_words();
        let tpl = crate::templates::default_templates().into_iter().find(|t| t.id == "minimal").unwrap();
        let p = plan_edit(&c, &tpl, &w, 1920, 1080, &FaceStats::default(), false);
        assert!(p.timeline.is_identity());
        assert!(p.punches.is_empty());
        assert!(p.sfx.is_empty());
    }
}
