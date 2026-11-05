// Common types shared across all modules
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Clip {
    pub id: String,
    pub project_id: String,
    pub start_s: f64,
    pub end_s: f64,
    pub duration_s: f64,
    pub title: String,
    pub hook_score: Option<f64>,
    pub viral_score: Option<f64>,
    pub heuristic_json: Option<String>,
    pub llm_reason: Option<String>,
    pub template_id: Option<String>,
    pub render_path: Option<String>,
    pub status: String,
    pub hook_segments: Option<String>,
    pub montage_order: Option<i64>,
    #[serde(default)]
    pub is_clip_relative: bool,
    #[serde(default)]
    pub caption_warning: Option<String>,
    #[serde(default)]
    pub timeline_data: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub confidence: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Transcript {
    pub language: String,
    pub duration: f64,
    pub segments: Vec<TranscriptSegment>,
    #[serde(default)]
    pub is_clip_relative: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeuristicScores {
    pub hook_density: f64,
    pub pace: f64,
    pub sentiment_arc: f64,
    pub self_containment: f64,
    pub audio_energy_variance: f64,
    pub duration_compliance: f64,
    pub total: f64,
}

impl Clip {
    pub fn new(
        project_id: String,
        start_s: f64,
        end_s: f64,
        title: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            project_id,
            start_s,
            end_s,
            duration_s: end_s - start_s,
            title,
            hook_score: None,
            viral_score: None,
            heuristic_json: None,
            llm_reason: None,
            template_id: None,
            render_path: None,
            status: "pending".to_string(),
            hook_segments: None,
            montage_order: None,
            is_clip_relative: false,
            caption_warning: None,
            timeline_data: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaTranscript {
    pub text: String,
    pub language: String,
    pub duration: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClipSegment {
    pub id: String,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

pub fn sanitize_filename(name: &str) -> String {
    let invalid = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let cleaned: String = name
        .chars()
        .map(|c| if invalid.contains(&c) { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim_matches('_');
    if trimmed.is_empty() {
        "clip".to_string()
    } else {
        trimmed.to_string()
    }
}

// ==========================================
// EDL (Edit Decision List) Schema
// ==========================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Edl {
    pub version: i32,
    pub source: EdlSource,
    pub content_type: String,
    pub language: String,
    pub style_preset: String,
    pub why_this_style: String,
    pub hook: EdlHook,
    pub cuts: Vec<EdlCut>,
    pub effects: Vec<EdlEffect>,
    pub overlays: Vec<EdlOverlay>,
    pub captions: EdlCaptions,
    pub audio: EdlAudio,
    pub framing: EdlFraming,
    pub end: EdlEnd,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlSource {
    pub video_id: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlHook {
    pub options: Vec<String>,
    pub chosen: usize,
    pub strategy: String,
    pub pull_from: EdlTimeRange,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlTimeRange {
    pub t0: f64,
    pub t1: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlCut {
    pub t0: f64,
    pub t1: f64,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlEffect {
    pub t: f64,
    pub dur: f64,
    pub r#type: String, // using r#type because type is a reserved keyword
    pub intensity: f64,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlOverlay {
    pub t: f64,
    pub dur: f64,
    pub r#type: String,
    pub text: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlCaptions {
    pub style: String,
    pub words_per_line: i32,
    pub highlight: String,
    pub font: String,
    pub position: String,
    pub rtl: bool,
    #[serde(default)]
    pub sync_nudge_ms: i32,
    #[serde(default)]
    pub keyword_emphasis: Vec<String>,
    #[serde(default)]
    pub highlight_color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlAudio {
    pub loudnorm_lufs: i32,
    pub music: EdlMusic,
    pub sfx: Vec<EdlSfx>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlMusic {
    pub mood: String,
    pub duck_db: i32,
    pub file: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlSfx {
    pub t: f64,
    pub r#type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlFraming {
    pub mode: String,
    pub keyframes: Vec<serde_json::Value>, // Keeping flexible for now
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EdlEnd {
    pub loop_friendly: bool,
    pub cta: Option<String>,
}

impl Edl {
    pub fn default_for_clip(clip: &Clip) -> Self {
        let dur = if clip.duration_s > 0.5 { clip.duration_s } else { 30.0 };
        Self {
            version: 1,
            source: EdlSource {
                video_id: clip.id.clone(),
                start: clip.start_s,
                end: clip.end_s,
            },
            content_type: "general".to_string(),
            language: "en".to_string(),
            style_preset: "viral_high_energy".to_string(),
            why_this_style: "Dynamic viral pacing with opening hook urgency and rhythmic sentence emphasis.".to_string(),
            hook: EdlHook {
                options: vec![
                    "WAIT FOR THIS 🔥".to_string(),
                    "You won't believe this 🤯".to_string(),
                    "Watch until the end ⚡".to_string(),
                ],
                chosen: 0,
                strategy: "cold_open".to_string(),
                pull_from: EdlTimeRange { t0: 0.0, t1: 2.5f64.min(dur) },
            },
            cuts: Vec::new(),
            effects: vec![
                EdlEffect {
                    t: 0.0,
                    dur: 2.4f64.min(dur * 0.4),
                    r#type: "punch_in".to_string(),
                    intensity: 0.16,
                    reason: "Hook visual urgency to capture immediate attention".to_string(),
                }
            ],
            overlays: Vec::new(),
            captions: EdlCaptions {
                style: "bold_pop".to_string(),
                words_per_line: 3,
                highlight: "word".to_string(),
                font: "Noto Sans".to_string(),
                position: "lower_middle".to_string(),
                rtl: false,
                sync_nudge_ms: 0,
                keyword_emphasis: Vec::new(),
                highlight_color: Some("#FFE600".to_string()),
            },
            audio: EdlAudio {
                loudnorm_lufs: -14,
                music: EdlMusic {
                    mood: "upbeat".to_string(),
                    duck_db: -14,
                    file: None,
                },
                sfx: Vec::new(),
            },
            framing: EdlFraming {
                mode: "framed_blur".to_string(),
                keyframes: Vec::new(),
            },
            end: EdlEnd {
                loop_friendly: true,
                cta: None,
            },
        }
    }

    pub fn validate_and_repair(&mut self, clip_duration: f64) {
        self.version = 1;
        let dur = if clip_duration > 0.5 { clip_duration } else { 30.0 };

        // 1. Sanitize style preset
        if self.style_preset.is_empty() {
            self.style_preset = "viral_high_energy".to_string();
        }

        // 2. Sanitize Hook
        if self.hook.options.is_empty() {
            self.hook.options = vec![
                "WAIT FOR THIS 🔥".to_string(),
                "You won't believe this 🤯".to_string(),
                "Watch until the end ⚡".to_string(),
            ];
        }
        if self.hook.chosen >= self.hook.options.len() {
            self.hook.chosen = 0;
        }
        self.hook.pull_from.t0 = self.hook.pull_from.t0.clamp(0.0, dur);
        self.hook.pull_from.t1 = self.hook.pull_from.t1.clamp(self.hook.pull_from.t0, dur);

        // 3. Sanitize Effects (Camera punch-ins & movements)
        let mut valid_effects: Vec<EdlEffect> = Vec::new();
        for eff in &self.effects {
            if eff.t.is_nan() || eff.dur.is_nan() || eff.intensity.is_nan() {
                continue;
            }
            if eff.t < 0.0 || eff.t >= dur - 0.4 {
                continue;
            }
            let clamped_t = eff.t;
            let max_dur = dur - clamped_t;
            let clamped_dur = eff.dur.clamp(0.4, 6.0).min(max_dur);
            let clamped_intensity = eff.intensity.clamp(0.05, 0.35);

            if clamped_dur >= 0.2 {
                valid_effects.push(EdlEffect {
                    t: clamped_t,
                    dur: clamped_dur,
                    r#type: if eff.r#type.is_empty() { "punch_in".to_string() } else { eff.r#type.clone() },
                    intensity: clamped_intensity,
                    reason: eff.reason.clone(),
                });
            }
        }

        // Sort effects chronologically
        valid_effects.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

        // Space out punch-ins: enforce at least 1.8s between adjacent punches to prevent crop jitter
        let mut spaced_effects: Vec<EdlEffect> = Vec::new();
        for eff in valid_effects {
            if let Some(last) = spaced_effects.last() {
                if eff.t < last.t + 1.8 {
                    continue; // skip clashing punch
                }
            }
            spaced_effects.push(eff);
        }
        self.effects = spaced_effects;

        // 4. Sanitize cuts (dead-air trims)
        let mut valid_cuts = Vec::new();
        for cut in &self.cuts {
            if cut.t0.is_nan() || cut.t1.is_nan() { continue; }
            let c0 = cut.t0.clamp(0.0, dur);
            let c1 = cut.t1.clamp(c0, dur);
            if c1 - c0 >= 0.2 {
                valid_cuts.push(EdlCut {
                    t0: c0,
                    t1: c1,
                    reason: cut.reason.clone(),
                });
            }
        }
        self.cuts = valid_cuts;

        // 5. Sanitize captions
        self.captions.words_per_line = self.captions.words_per_line.clamp(1, 6);
        self.captions.sync_nudge_ms = self.captions.sync_nudge_ms.clamp(-300, 300);
        if self.captions.highlight.is_empty() {
            self.captions.highlight = "word".to_string();
        }
        if self.captions.position.is_empty() {
            self.captions.position = "lower_middle".to_string();
        }

        // 6. Audio
        self.audio.loudnorm_lufs = self.audio.loudnorm_lufs.clamp(-24, -12);
        self.audio.music.duck_db = self.audio.music.duck_db.clamp(-30, -6);
    }
}

