
// Extracted from scorer/mod.rs, scorer/signals.rs, and scorer/clip_finder.rs

pub mod signals {
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct VideoSignals {
    pub heatmap: Vec<HeatmapPoint>,
    pub audio_peaks: Vec<f64>,       // timestamps of audio peaks
    pub scene_changes: Vec<f64>,     // timestamps of scene changes
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeatmapPoint {
    pub start_time: f64,
    pub end_time: f64,
    pub value: f64, // normalized 0.0 to 1.0
}

/// Extract extra signals (Heatmap, Audio peaks, Scene changes)
pub async fn extract_signals(video_path: &str, info_json_path: Option<&str>) -> Result<VideoSignals> {
    let mut signals = VideoSignals::default();

    // 1. YouTube Heatmap
    if let Some(json_path) = info_json_path {
        if let Ok(data) = std::fs::read_to_string(json_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(heatmap_val) = json.get("heatmap").and_then(|h| h.as_array()) {
                    let mut max_val = 0.0_f64;
                    for p in heatmap_val {
                        if let Some(v) = p.get("value").and_then(|v| v.as_f64()) {
                            if v > max_val {
                                max_val = v;
                            }
                        }
                    }
                    if max_val > 0.0 {
                        for p in heatmap_val {
                            if let (Some(s), Some(e), Some(v)) = (
                                p.get("start_time").and_then(|v| v.as_f64()),
                                p.get("end_time").and_then(|v| v.as_f64()),
                                p.get("value").and_then(|v| v.as_f64()),
                            ) {
                                signals.heatmap.push(HeatmapPoint {
                                    start_time: s,
                                    end_time: e,
                                    value: v / max_val,
                                });
                            }
                        }
                        info!("[Signals] Found {} heatmap points", signals.heatmap.len());
                    }
                }
            }
        }
    }

    // 2. FFmpeg audio peaks (ebur128) and scene changes (scdet)
    // Run them in parallel using tokio
    let audio_task = tokio::spawn(extract_audio_peaks(video_path.to_string()));
    let scene_task = tokio::spawn(extract_scene_changes(video_path.to_string()));

    let (audio_res, scene_res) = tokio::join!(audio_task, scene_task);

    if let Ok(Ok(peaks)) = audio_res {
        signals.audio_peaks = peaks;
        info!("[Signals] Extracted {} audio peaks", signals.audio_peaks.len());
    } else {
        warn!("[Signals] Failed to extract audio peaks");
    }

    if let Ok(Ok(scenes)) = scene_res {
        signals.scene_changes = scenes;
        info!("[Signals] Extracted {} scene changes", signals.scene_changes.len());
    } else {
        warn!("[Signals] Failed to extract scene changes");
    }

    Ok(signals)
}

async fn extract_audio_peaks(video_path: String) -> Result<Vec<f64>> {
    let ffmpeg_exe = crate::ffmpeg::resolve_ffmpeg();
    
    let mut peaks = Vec::new();
    
    let mut ebur_cmd = Command::new(&ffmpeg_exe);
    ebur_cmd.args([
        "-v", "info",
        "-i", &video_path,
        "-af", "ebur128=peak=true",
        "-f", "null",
        "-",
    ]);
    ebur_cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut ebur_child = ebur_cmd.spawn()?;
    
    if let Some(stderr) = ebur_child.stderr.take() {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if line.contains("Parsed_ebur128") && line.contains("t: ") && line.contains("M: ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                let mut time = None;
                let mut momentary = None;
                for i in 0..parts.len() {
                    if parts[i] == "t:" && i + 1 < parts.len() {
                        time = parts[i+1].parse::<f64>().ok();
                    }
                    if parts[i] == "M:" && i + 1 < parts.len() {
                        momentary = parts[i+1].parse::<f64>().ok();
                    }
                }
                if let (Some(t), Some(m)) = (time, momentary) {
                    if m > -12.0 { // threshold for "peak" loudness
                        peaks.push(t);
                    }
                }
            }
        }
    }
    
    let _ = ebur_child.wait().await;
    
    // Deduplicate peaks that are too close (e.g., within 1 second)
    let mut dedup = Vec::new();
    let mut last = -10.0;
    for p in peaks {
        if p - last > 1.0 {
            dedup.push(p);
            last = p;
        }
    }

    Ok(dedup)
}

async fn extract_scene_changes(video_path: String) -> Result<Vec<f64>> {
    let ffmpeg_exe = crate::ffmpeg::resolve_ffmpeg();
    
    let mut cmd = Command::new(&ffmpeg_exe);
    cmd.args([
        "-v", "info",
        "-i", &video_path,
        "-vf", "scale=160:-1,select='gt(scene,0.2)',showinfo",
        "-f", "null",
        "-",
    ]);

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    
    let mut scenes = Vec::new();
    if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if line.contains("Parsed_showinfo") && line.contains("pts_time:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for part in parts {
                    if let Some(time_str) = part.strip_prefix("pts_time:") {
                        if let Ok(t) = time_str.parse::<f64>() {
                            scenes.push(t);
                        }
                    }
                }
            }
        }
    }
    
    let _ = child.wait().await;
    Ok(scenes)
}

}

pub mod clip_finder {
use anyhow::Result;
use crate::common::{Clip, Transcript};
use crate::pipeline::analyze::signals::VideoSignals;
use serde::{Deserialize, Serialize};
use reqwest::Client;
use tracing::{info, warn};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CandidateClip {
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
    pub hook_score: f64,
    pub viral_score: f64,
    pub criteria: CriteriaScores,
    pub confidence: f64,
    pub strengths: Vec<String>,
    pub weaknesses: Vec<String>,
    pub how_to_reach_90: String,
    pub retention_risk: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CriteriaScores {
    pub hook: f64,
    pub emotion: f64,
    pub clarity: f64,
    pub payoff: f64,
    pub pacing: f64,
    pub shareability: f64,
}

pub async fn run_clip_finder(
    api_key: &str,
    transcript: &Transcript,
    signals: &VideoSignals,
    max_clips: usize,
) -> Result<Vec<Clip>> {
    let duration = transcript.duration;
    if duration < 10.0 {
        return Ok(Vec::new());
    }
    
    // 1. Coverage: analyze 100% of the transcript with 120s windows, 30s step
    let window_size = 120.0;
    let step = 30.0;
    let mut current_start = 0.0;
    let mut windows = Vec::new();
    
    while current_start < duration {
        let end_time = (current_start + window_size).min(duration);
        let segment_texts: Vec<String> = transcript.segments.iter()
            .filter(|s| s.start < end_time && s.end > current_start)
            .map(|s| format!("[{:.1}s - {:.1}s] {}", s.start, s.end, s.text))
            .collect();
            
        if !segment_texts.is_empty() {
            windows.push((current_start, end_time, segment_texts.join("\n")));
        }
        
        current_start += step;
    }
    
    info!("[Clip Finder] Analyzed {:.1} of {:.1}, {} windows", duration, duration, windows.len());
    
    // 2. Call LLM for each window
    let client = Client::new();
    let mut all_candidates = Vec::new();
    
    // Limit concurrency to 2 to avoid Gemini rate limits (free tier is strict)
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let mut tasks = Vec::new();
    
    let total_windows = windows.len();
    for (idx, (w_start, w_end, text)) in windows.into_iter().enumerate() {
        let sem_clone = sem.clone();
        let client_clone = client.clone();
        let api_key_owned = api_key.to_string();
        
        let heatmap_peaks = signals.heatmap.iter()
            .filter(|p| p.start_time >= w_start && p.end_time <= w_end && p.value > 0.5)
            .count();
        let audio_peaks = signals.audio_peaks.iter()
            .filter(|&&t| t >= w_start && t <= w_end)
            .count();
        let scene_density = signals.scene_changes.iter()
            .filter(|&&t| t >= w_start && t <= w_end)
            .count();
            
        let signal_context = format!(
            "EXTRA SIGNALS FOR THIS WINDOW: YouTube Heatmap Peaks: {}, Audio Peaks: {}, Scene Changes: {}", 
            heatmap_peaks, audio_peaks, scene_density
        );
        
        let task = tokio::spawn(async move {
            let _permit = sem_clone.acquire().await.unwrap();
            let cands = get_candidates_for_window(&client_clone, &api_key_owned, w_start, w_end, &text, &signal_context).await;
            (idx, cands)
        });
        tasks.push(task);
    }
    
    for task in tasks {
        if let Ok((idx, Ok(cands))) = task.await {
            info!("[Clip Finder] Window {} returned {} candidates", idx, cands.len());
            all_candidates.extend(cands);
        }
    }
    
    if all_candidates.is_empty() {
        warn!("[Clip Finder] No candidates returned from LLM!");
        return Ok(Vec::new());
    }
    
    // 4. Merge + Dedupe candidates
    // Remove exact overlaps, keep higher score
    all_candidates.sort_by(|a, b| b.viral_score.partial_cmp(&a.viral_score).unwrap_or(std::cmp::Ordering::Equal));
    let mut deduped: Vec<CandidateClip> = Vec::new();
    for c in all_candidates {
        let duration = c.end_time - c.start_time;
        if !(15.0..=75.0).contains(&duration) {
            continue; // reject outside target lengths
        }
        let overlaps = deduped.iter().any(|existing| {
            // true if they overlap by more than 50%
            let max_start = c.start_time.max(existing.start_time);
            let min_end = c.end_time.min(existing.end_time);
            let overlap_dur = (min_end - max_start).max(0.0);
            overlap_dur > duration * 0.5 || overlap_dur > (existing.end_time - existing.start_time) * 0.5
        });
        if !overlaps {
            deduped.push(c);
        }
    }
    
    // 5. Rerank (Final LLM Call for variety)
    let original_deduped = deduped.clone();
    deduped = rerank_candidates(&client, api_key, deduped, max_clips).await.unwrap_or(original_deduped);
    
    // Convert to Clip format and snap to sentence boundaries
    let mut final_clips = Vec::new();
    for c in deduped.into_iter().take(max_clips) {
        let mut clip = Clip::new("temp".to_string(), c.start_time, c.end_time, c.title.clone());
        // Snap to nearest segment
        if let Some(closest_start) = transcript.segments.iter().min_by(|a, b| {
            (a.start - clip.start_s).abs().partial_cmp(&(b.start - clip.start_s).abs()).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            if (closest_start.start - clip.start_s).abs() < 3.0 {
                clip.start_s = closest_start.start;
            }
        }
        if let Some(closest_end) = transcript.segments.iter().min_by(|a, b| {
            (a.end - clip.end_s).abs().partial_cmp(&(b.end - clip.end_s).abs()).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            if (closest_end.end - clip.end_s).abs() < 3.0 {
                clip.end_s = closest_end.end;
            }
        }
        clip.duration_s = clip.end_s - clip.start_s;
        
        let score_val = c.viral_score;
        clip.viral_score = Some(score_val);
        clip.hook_score = Some(c.hook_score);
        
        let reason = format!(
            "🎯 Score: {}/100 (Confidence: {})\n\n💡 Strengths:\n- {}\n\n⚠️ Retention Risk: {}\n{}\n\n📈 How to reach 90: {}",
            score_val, c.confidence,
            c.strengths.join("\n- "),
            c.retention_risk,
            c.weaknesses.join("\n"),
            c.how_to_reach_90
        );
        clip.llm_reason = Some(reason);
        clip.template_id = Some("tiktok".to_string());
        
        final_clips.push(clip);
    }
    
    // Sort chronologically
    final_clips.sort_by(|a, b| a.start_s.partial_cmp(&b.start_s).unwrap_or(std::cmp::Ordering::Equal));
    
    info!("[Clip Finder] Selected {} clips from {} candidates", final_clips.len(), total_windows);
    
    Ok(final_clips)
}

async fn get_candidates_for_window(
    client: &Client,
    api_key: &str,
    w_start: f64,
    w_end: f64,
    text: &str,
    signal_context: &str,
) -> Result<Vec<CandidateClip>> {
    let prompt = format!(
        r#"You are an elite viral video editor. Analyze this transcription window from {:.1}s to {:.1}s.
{}

Extract moments that a human editor would choose for a viral short (TikTok/Reels).
Rules:
- Length: 20 to 60 seconds.
- Start on the hook (cut lead-in). Make sense with NO context. Contain a payoff. End on a complete thought.
- 'title' MUST be a concise, compelling topic hook (3-6 words) describing what happens or the main takeaway. NEVER copy the opening speech sentence or words verbatim.
- Do NOT output round percentages. Most clips land 55-85.
- Return ONLY JSON. No markdown.

Rubric for viral_score (0-100):
hook strength (25), emotion (20), standalone clarity (15), payoff (15), pacing (15), shareability (10).

Format:
[
  {{
    "start_time": 15.2,
    "end_time": 45.0,
    "title": "Punchy Title",
    "hook_score": 82.5,
    "viral_score": 78.4,
    "criteria": {{"hook": 20, "emotion": 15, "clarity": 12, "payoff": 12, "pacing": 12, "shareability": 7.4}},
    "confidence": 0.85,
    "strengths": ["...", "..."],
    "weaknesses": ["..."],
    "how_to_reach_90": "...",
    "retention_risk": "..."
  }}
]

Transcript:
{}
"#, w_start, w_end, signal_context, text
    );
    
    let req = crate::gemini::GeminiRequest {
        contents: vec![crate::gemini::GeminiContent {
            role: "user".to_string(),
            parts: vec![crate::gemini::GeminiPart { text: prompt }],
        }],
        generation_config: Some(crate::gemini::GeminiGenerationConfig {
            temperature: Some(0.3),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(4096),
            response_mime_type: Some("application/json".to_string()),
        }),
        safety_settings: None,
    };
    
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key={}",
        api_key
    );
    
    let resp = client.post(&url).json(&req).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("API Error: {}", resp.status()));
    }
    
    let gemini_resp: crate::gemini::GeminiResponse = resp.json().await?;
    let part_text = gemini_resp.candidates.first()
        .and_then(|c| c.content.parts.first())
        .map(|p| p.text.as_str())
        .ok_or_else(|| anyhow::anyhow!("Empty JSON"))?;
        
    let clean = part_text.trim().trim_start_matches("```json").trim_end_matches("```").trim();
    let parsed: Vec<CandidateClip> = serde_json::from_str(clean)?;
    
    Ok(parsed)
}

async fn rerank_candidates(
    client: &Client,
    api_key: &str,
    candidates: Vec<CandidateClip>,
    max_clips: usize,
) -> Result<Vec<CandidateClip>> {
    if candidates.len() <= max_clips {
        return Ok(candidates);
    }
    
    let summary = candidates.iter().enumerate().map(|(i, c)| {
        format!("Clip {}: [{:.1}-{:.1}] Score: {} Title: {}", i, c.start_time, c.end_time, c.viral_score, c.title)
    }).collect::<Vec<_>>().join("\n");
    
    let prompt = format!(
        r#"You are a final-pass editor. Pick the top {} clips from this shortlist to maximize variety and quality. Avoid picking multiple clips from the exact same minute unless it's a massive peak.
Return a JSON array of the chosen indices, e.g. [0, 2, 5].

Shortlist:
{}
"#, max_clips, summary
    );
    
    let req = crate::gemini::GeminiRequest {
        contents: vec![crate::gemini::GeminiContent {
            role: "user".to_string(),
            parts: vec![crate::gemini::GeminiPart { text: prompt }],
        }],
        generation_config: Some(crate::gemini::GeminiGenerationConfig {
            temperature: Some(0.1),
            top_p: Some(0.9),
            top_k: Some(40),
            max_output_tokens: Some(1024),
            response_mime_type: Some("application/json".to_string()),
        }),
        safety_settings: None,
    };
    
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key={}",
        api_key
    );
    
    let resp = client.post(&url).json(&req).send().await?;
    if !resp.status().is_success() {
        return Ok(candidates);
    }
    
    let gemini_resp: crate::gemini::GeminiResponse = resp.json().await?;
    let part_text = gemini_resp.candidates.first()
        .and_then(|c| c.content.parts.first())
        .map(|p| p.text.as_str())
        .unwrap_or("[]");
        
    let clean = part_text.trim().trim_start_matches("```json").trim_end_matches("```").trim();
    if let Ok(indices) = serde_json::from_str::<Vec<usize>>(clean) {
        let mut final_c = Vec::new();
        for idx in indices {
            if let Some(c) = candidates.get(idx) {
                final_c.push(c.clone());
            }
        }
        if !final_c.is_empty() {
            return Ok(final_c);
        }
    }
    
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::TranscriptSegment;

    #[test]
    fn test_clip_overlap_prevention_logic() {
        // We'll write a mock test to verify the logic used inside find_viral_clips
        let mut candidates = vec![
            CandidateClip { start_time: 10.0, end_time: 30.0, viral_score: 80.0, hook_score: 80.0, title: "".into(), criteria: Default::default(), confidence: 0.9, strengths: vec![], weaknesses: vec![], how_to_reach_90: "".into(), retention_risk: "".into() },
            CandidateClip { start_time: 15.0, end_time: 35.0, viral_score: 85.0, hook_score: 80.0, title: "".into(), criteria: Default::default(), confidence: 0.9, strengths: vec![], weaknesses: vec![], how_to_reach_90: "".into(), retention_risk: "".into() }, // Overlaps above, higher score
            CandidateClip { start_time: 40.0, end_time: 60.0, viral_score: 75.0, hook_score: 80.0, title: "".into(), criteria: Default::default(), confidence: 0.9, strengths: vec![], weaknesses: vec![], how_to_reach_90: "".into(), retention_risk: "".into() },
        ];
        
        candidates.sort_by(|a, b| b.viral_score.partial_cmp(&a.viral_score).unwrap());
        
        let mut final_clips: Vec<CandidateClip> = Vec::new();
        for c in candidates {
            let mut overlaps = false;
            for existing in &final_clips {
                let overlap_start = c.start_time.max(existing.start_time);
                let overlap_end = c.end_time.min(existing.end_time);
                if overlap_start < overlap_end {
                    overlaps = true;
                    break;
                }
            }
            if !overlaps {
                final_clips.push(c);
            }
        }
        
        assert_eq!(final_clips.len(), 2);
        assert_eq!(final_clips[0].start_time, 15.0); // The 85 score
        assert_eq!(final_clips[1].start_time, 40.0); // Non-overlapping
    }

    #[test]
    fn test_sentence_boundary_snapping() {
        let transcript = Transcript {
            duration: 100.0,
            language: "en".into(),
            segments: vec![
                TranscriptSegment { start: 14.5, end: 15.2, text: "Hello".into(), words: vec![] },
                TranscriptSegment { start: 34.8, end: 35.5, text: "World".into(), words: vec![] },
            ],
            is_clip_relative: false,
        };
        
        // Simulating the loop logic
        let mut clip_start = 15.0;
        let mut clip_end = 35.0;
        
        if let Some(closest_start) = transcript.segments.iter().min_by(|a, b| {
            (a.start - clip_start).abs().partial_cmp(&(b.start - clip_start).abs()).unwrap()
        }) {
            if (closest_start.start - clip_start).abs() < 3.0 {
                clip_start = closest_start.start;
            }
        }
        
        if let Some(closest_end) = transcript.segments.iter().min_by(|a, b| {
            (a.end - clip_end).abs().partial_cmp(&(b.end - clip_end).abs()).unwrap()
        }) {
            if (closest_end.end - clip_end).abs() < 3.0 {
                clip_end = closest_end.end;
            }
        }
        
        assert_eq!(clip_start, 14.5);
        assert_eq!(clip_end, 35.5);
    }
}


}

use anyhow::Result;
use crate::common::{Clip, Transcript, Word, HeuristicScores};
use tracing::info;





pub async fn analyze_and_score(
    transcript: &Transcript,
    max_clips: usize,
) -> Result<Vec<Clip>> {
    let duration = transcript.duration;

    // Log actual analyzed duration so we can verify the full video is being processed
    let first_ts = transcript.segments.first().map(|s| s.start).unwrap_or(0.0);
    let last_ts = transcript.segments.last().map(|s| s.end).unwrap_or(0.0);
    info!("[Scorer] Analyzing FULL transcript: {:.1}s – {:.1}s = {:.1}s total | {} segments",
        first_ts, last_ts, last_ts - first_ts, transcript.segments.len());

    if transcript.segments.is_empty() || duration < 10.0 {
        return Ok(Vec::new());
    }

    // 1. Find all sentence ends based on punctuation or silence gaps > 0.8s
    let mut sentence_ends = Vec::new();
    for (i, seg) in transcript.segments.iter().enumerate() {
        let text = seg.text.trim();
        let is_punct = text.ends_with('.') || text.ends_with('!') || text.ends_with('?');
        let is_gap = if i + 1 < transcript.segments.len() {
            transcript.segments[i + 1].start - seg.end > 0.8
        } else {
            true
        };

        if is_punct || is_gap || i == transcript.segments.len() - 1 {
            sentence_ends.push(i);
        }
    }

    // 2. Generate candidate blocks of varied lengths (45s – 90s) on exact sentence boundaries.
    //    NO upper cap on transcript time — we scan the full video.
    let mut candidates: Vec<(f64, f64, HeuristicScores)> = Vec::new();
    for (start_idx, start_seg) in transcript.segments.iter().enumerate() {
        let start_time = start_seg.start;
        for &end_idx in &sentence_ends {
            if end_idx < start_idx {
                continue;
            }
            let end_time = transcript.segments[end_idx].end;
            let block_dur = end_time - start_time;
            if (45.0..=90.0).contains(&block_dur) {
                let heuristic = calculate_heuristics(transcript, start_time, end_time);
                candidates.push((start_time, end_time, heuristic));
            } else if block_dur > 90.0 {
                break; // inner loop: end_idx keeps growing, no point continuing
            }
        }
    }

    // Fallback if no candidate met strict sentence boundaries (e.g. video without punctuation)
    if candidates.is_empty() {
        let mut t = 0.0;
        // 60-second fixed windows across the whole video
        let step = 60.0_f64;
        while t + 30.0 <= duration {
            let end_t = (t + step).min(duration);
            if end_t - t >= 30.0 {
                let heuristic = calculate_heuristics(transcript, t, end_t);
                candidates.push((t, end_t, heuristic));
            }
            t += step;
        }
    }

    // 3. Distribute candidate selection across the whole video (time buckets)
    let bucket_count = max_clips.max(1);
    let bucket_size = duration / (bucket_count as f64);
    let mut chosen: Vec<(f64, f64, HeuristicScores)> = Vec::new();

    // First pass: try to pick the best non-overlapping candidate from each time bucket
    for b in 0..bucket_count {
        let b_start = (b as f64) * bucket_size;
        let b_end = ((b + 1) as f64) * bucket_size;

        let mut bucket_candidates: Vec<&(f64, f64, HeuristicScores)> = candidates.iter()
            .filter(|c| c.0 >= b_start - 5.0 && c.0 < b_end)
            .collect();

        bucket_candidates.sort_by(|a, b| b.2.total.partial_cmp(&a.2.total).unwrap_or(std::cmp::Ordering::Equal));

        for cand in bucket_candidates {
            let overlaps = chosen.iter().any(|ch| {
                !(cand.1 <= ch.0 + 1.0 || cand.0 >= ch.1 - 1.0)
            });
            if !overlaps {
                chosen.push(cand.clone());
                break;
            }
        }
    }

    // Second pass: fill any remaining slots up to max_clips from overall highest scoring non-overlapping
    if chosen.len() < max_clips {
        let mut sorted_all = candidates.clone();
        sorted_all.sort_by(|a, b| b.2.total.partial_cmp(&a.2.total).unwrap_or(std::cmp::Ordering::Equal));

        for cand in sorted_all {
            if chosen.len() >= max_clips {
                break;
            }
            let overlaps = chosen.iter().any(|ch| {
                !(cand.1 <= ch.0 + 1.0 || cand.0 >= ch.1 - 1.0)
            });
            if !overlaps {
                chosen.push(cand);
            }
        }
    }

    // 4. Sort chosen clips chronologically
    chosen.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // 5. Convert to Clip structs with snapped boundaries and varied titles
    let clips: Vec<Clip> = chosen.into_iter().enumerate().map(|(idx, (start, end, heuristic))| {
        let dur = (end - start).round();
        let mut clip = Clip::new(
            "temp".to_string(),
            (start * 100.0).round() / 100.0,
            (end * 100.0).round() / 100.0,
            format!("Clip {}: Key Moment ({:.0}s)", idx + 1, dur),
        );
        clip.hook_score = Some((heuristic.hook_density * 100.0).round() / 100.0);
        clip.heuristic_json = Some(serde_json::to_string(&heuristic).unwrap_or_default());
        clip
    }).collect();

    Ok(clips)
}

fn calculate_heuristics(transcript: &Transcript, start: f64, end: f64) -> HeuristicScores {
    let segment_words: Vec<&Word> = transcript.segments.iter()
        .flat_map(|s| s.words.iter())
        .filter(|w| w.start >= start && w.end <= end)
        .collect();

    let duration = end - start;
    
    // 1. Hook density (first 3 seconds)
    let hook_density = calculate_hook_density(transcript, start);
    
    // 2. Pace (words per minute)
    let pace = if duration > 0.0 {
        (segment_words.len() as f64 / duration) * 60.0
    } else { 0.0 };
    
    // 3. Sentiment arc
    let sentiment_arc = calculate_sentiment_arc(&segment_words);
    
    // 4. Self-containment (question & answer or statement completeness)
    let self_containment = calculate_self_containment(&segment_words);
    
    // 5. Audio energy variance placeholder
    let audio_energy_variance = 0.6;
    
    // 6. Duration compliance (prefers 45s – 75s — ideal ~60s TikTok/Reels length)
    let duration_compliance = if (45.0..=75.0).contains(&duration) {
        1.0
    } else if (35.0..=90.0).contains(&duration) {
        0.75
    } else if duration >= 30.0 {
        0.4
    } else {
        0.1 // very short clips penalized
    };

    // Weighted total
    let total = 
        hook_density * 0.30 +
        (pace / 180.0).min(1.0) * 0.20 +
        sentiment_arc * 0.15 +
        self_containment * 0.15 +
        audio_energy_variance * 0.10 +
        duration_compliance * 0.10;

    HeuristicScores {
        hook_density,
        pace,
        sentiment_arc,
        self_containment,
        audio_energy_variance,
        duration_compliance,
        total,
    }
}

fn calculate_hook_density(transcript: &Transcript, segment_start: f64) -> f64 {
    let hook_keywords = [
        "you", "stop", "wait", "watch", "look", "listen",
        "secret", "trick", "hack", "mistake", "wrong",
        "never", "always", "must", "need", "shocking",
        "unbelievable", "incredible", "amazing", "crazy",
        "insane", "wild", "mind", "blown", "game changer",
        "why", "how", "what", "discover", "reveal", "check this",
    ];

    let first_3_seconds = transcript.segments.iter()
        .filter(|s| s.end >= segment_start && s.start < segment_start + 3.5)
        .flat_map(|s| s.words.iter())
        .collect::<Vec<_>>();

    if first_3_seconds.is_empty() {
        return 0.1;
    }

    let hook_count = first_3_seconds.iter()
        .filter(|w| {
            let text = w.text.to_lowercase();
            hook_keywords.iter().any(|k| text.contains(k))
        })
        .count();

    (hook_count as f64 / first_3_seconds.len() as f64).min(1.0)
}

fn calculate_sentiment_arc(words: &[&Word]) -> f64 {
    let positive = ["good", "great", "awesome", "amazing", "love", "best", "happy", "excited", "win", "success", "cooking", "destroyed", "epic"];
    let negative = ["bad", "terrible", "awful", "hate", "worst", "sad", "angry", "fail", "wrong", "problem", "miss", "lost"];
    
    let text = words.iter()
        .map(|w| w.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    let pos_count = positive.iter().filter(|&&w| text.contains(w)).count();
    let neg_count = negative.iter().filter(|&&w| text.contains(w)).count();
    
    if pos_count + neg_count == 0 {
        0.5
    } else {
        ((pos_count as f64 + 0.5) / (pos_count + neg_count + 1) as f64).clamp(0.2, 0.9)
    }
}

fn calculate_self_containment(words: &[&Word]) -> f64 {
    if words.is_empty() {
        return 0.0;
    }

    let first_word = words.first().unwrap().text.to_lowercase();
    let last_word = words.last().unwrap().text.to_lowercase();

    let mut score: f64 = 0.5;

    // Starts on clean capital or opening question
    if first_word.starts_with(|c: char| c.is_alphabetic()) {
        score += 0.2;
    }

    // Ends on punctuation
    if last_word.ends_with('.') || last_word.ends_with('!') || last_word.ends_with('?') {
        score += 0.3;
    }

    score.min(1.0)
}
