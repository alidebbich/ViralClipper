use anyhow::Result;
use crate::common::{Clip, Transcript, Edl, EdlEffect, EdlHook, EdlTimeRange, EdlCaptions, EdlAudio, EdlMusic, EdlFraming, EdlEnd, EdlSource, EdlCut};
use reqwest::Client;
use tracing::{info, warn};

/// Decides how a clip should be edited based on video type and specific clip moments.
/// If an API key is available, queries Gemini for creative direction and validates/repairs the result.
/// If offline or if the API call fails, seamlessly falls back to the deterministic, story-aware heuristic generator.
pub async fn build_edl_for_clip(
    api_key: &str,
    clip: &Clip,
    transcript: &Transcript,
    _signals: &crate::pipeline::analyze::signals::VideoSignals,
) -> Result<Edl> {
    info!("[Editor Brain] Generating Edit Decision Plan for clip {} ({:.1}s - {:.1}s)", clip.id, clip.start_s, clip.end_s);

    let style = clip.template_id.as_deref().unwrap_or("viral_high_energy");

    if !api_key.trim().is_empty() {
        // Extract transcript segment text for context
        let mut clip_text = String::new();
        for seg in &transcript.segments {
            if seg.start < clip.end_s && seg.end > clip.start_s {
                clip_text.push_str(&format!("[{:.1}s] {} ", seg.start, seg.text));
            }
        }

        let prompt = format!(
            r##"You are the "Editor Brain", an elite short-form video editor.
Generate an Edit Decision List (EDL) in JSON format for the following clip.

Clip Context:
Start: {:.1}s | End: {:.1}s | Duration: {:.1}s
Text:
{}

Style Presets (choose one):
- viral_high_energy: High-energy viral pacing, hook punch-in, contrasting keyword highlights.
- podcast_clean: Clean talking head, subtle micro-zooms, removes silences, clean captions.
- cinematic_story: Slower push-ins, wide framing with ambient blur, restrained typography.
- educational_explainer: Clear pacing, keyword focus, UI area respect.
- gaming_fast: Fast, cuts dead air, punch-in on action.

Rules:
1. "version": 1
2. "source": {{"video_id": "{}", "start": {:.1}, "end": {:.1}}}
3. "content_type": Detected content type (e.g. "podcast", "tutorial", "story", "gaming").
4. "language": Language code (e.g. "en", "ar", "fr").
5. "style_preset": "{}"
6. "why_this_style": One sentence explaining your creative editing rationale.
7. "hook": Provide 3 compelling hook options in the clip's language. "chosen": 0. "strategy": "cold_open". "pull_from": {{"t0": 0.0, "t1": 2.5}}.
8. "cuts": List of silent or dead-air sections [t0, t1, reason] to trim. Do NOT clip speech phonemes.
9. "effects": Camera movements (punch_in, zoom_ramp, shake). Max 1 major punch per 3s.
10. "overlays": Hook title banners, keyword callouts.
11. "captions": {{"style": "bold_pop", "words_per_line": 3, "highlight": "word", "font": "Noto Sans", "position": "lower_middle", "rtl": false, "sync_nudge_ms": 0, "keyword_emphasis": [], "highlight_color": "#FFE600"}}
12. "audio": {{"loudnorm_lufs": -14, "music": {{"mood": "upbeat", "duck_db": -14, "file": null}}, "sfx": []}}
13. "framing": {{"mode": "auto", "keyframes": []}} (use "smart_crop" for talking heads/speakers, "framed_blur" for wide landscape/gameplay UI, or "auto")
14. "end": {{"loop_friendly": true, "cta": null}}

Return ONLY valid JSON matching this schema. No markdown wrapping.
"##,
            clip.start_s, clip.end_s, clip.duration_s, clip_text,
            clip.id, clip.start_s, clip.end_s,
            style
        );

        let client = Client::new();
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
            api_key.trim()
        );

        match client.post(&url).json(&req).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(gemini_resp) = resp.json::<crate::gemini::GeminiResponse>().await {
                    if let Some(part_text) = gemini_resp.candidates.first()
                        .and_then(|c| c.content.parts.first())
                        .map(|p| p.text.as_str()) {
                        let clean = part_text.trim().trim_start_matches("```json").trim_end_matches("```").trim();
                        if let Ok(mut parsed) = serde_json::from_str::<Edl>(clean) {
                            parsed.validate_and_repair(clip.duration_s);
                            info!("[Editor Brain] Gemini produced valid EDL for clip {}", clip.id);
                            return Ok(parsed);
                        }
                    }
                }
            }
            Ok(resp) => {
                warn!("[Editor Brain] Gemini returned status {}. Falling back to heuristic plan.", resp.status());
            }
            Err(e) => {
                warn!("[Editor Brain] Gemini request error: {}. Falling back to heuristic plan.", e);
            }
        }
    }

    // High quality, deterministic, story-aware heuristic fallback
    info!("[Editor Brain] Generating story-aware heuristic EDL for clip {}", clip.id);
    let mut heuristic = generate_heuristic_edl(clip, transcript, style);
    heuristic.validate_and_repair(clip.duration_s);
    Ok(heuristic)
}

/// Resolves the active Edit Decision Plan for a clip.
/// Checks timeline_data, then heuristic_json, then falls back to story-aware heuristic generation.
pub fn resolve_clip_edl(clip: &Clip, transcript: Option<&Transcript>) -> Edl {
    // 1. Try timeline_data
    if let Some(ref data) = clip.timeline_data {
        if let Ok(mut edl) = serde_json::from_str::<Edl>(data) {
            edl.validate_and_repair(clip.duration_s);
            return edl;
        }
    }

    // 2. Try heuristic_json
    if let Some(ref data) = clip.heuristic_json {
        if let Ok(mut edl) = serde_json::from_str::<Edl>(data) {
            edl.validate_and_repair(clip.duration_s);
            return edl;
        }
    }

    // 3. Fallback: generate story-aware heuristic EDL from transcript
    if let Some(t) = transcript {
        let style = clip.template_id.as_deref().unwrap_or("viral_high_energy");
        let mut edl = generate_heuristic_edl(clip, t, style);
        edl.validate_and_repair(clip.duration_s);
        return edl;
    }

    // 4. Default baseline fallback
    let mut def = Edl::default_for_clip(clip);
    def.validate_and_repair(clip.duration_s);
    def
}

/// Generates a story-aware heuristic EDL from transcript timestamps and text content.
pub fn generate_heuristic_edl(
    clip: &Clip,
    transcript: &Transcript,
    style_preset: &str,
) -> Edl {
    let dur = if clip.duration_s > 0.5 { clip.duration_s } else { 30.0 };
    let explicit_relative = clip.is_clip_relative;
    let base_start = if explicit_relative { 0.0 } else { clip.start_s };

    // 1. Collect clip words
    let mut clip_words: Vec<&crate::common::Word> = Vec::new();
    for seg in &transcript.segments {
        for word in &seg.words {
            let (ws, we) = if explicit_relative {
                (word.start, word.end)
            } else {
                (word.start, word.end)
            };
            if explicit_relative || (we >= clip.start_s && ws <= clip.end_s) {
                clip_words.push(word);
            }
        }
    }

    // 2. Language & Arabic detection
    let has_arabic = clip_words.iter().any(|w| {
        w.text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c))
    }) || transcript.language == "ar";

    let has_french = transcript.language == "fr";

    let lang = if has_arabic {
        "ar"
    } else if has_french {
        "fr"
    } else {
        "en"
    };

    // 3. Hook strategy & options
    let opening_words: Vec<String> = clip_words.iter().take(8).map(|w| w.text.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let clean_title = clip.title.trim();
    let is_placeholder = crate::render::captions::is_placeholder_title(clean_title);
    let title_is_dup = crate::render::captions::is_substantially_duplicate(clean_title, &opening_words);

    let primary_hook = if !clean_title.is_empty() && !is_placeholder && !title_is_dup {
        clean_title.to_string()
    } else if has_arabic {
        "شاهد للنهاية 🔥".to_string()
    } else if has_french {
        "REGARDEZ JUSQU'À LA FIN 🔥".to_string()
    } else {
        "WATCH TILL THE END 🔥".to_string()
    };

    let hook_options = if has_arabic {
        vec![
            primary_hook,
            "لن تصدق ما حدث هنا 🤯".to_string(),
            "انتبه لهذه النقطة المهمة ⚡".to_string(),
        ]
    } else if has_french {
        vec![
            primary_hook,
            "Attendez la fin de cette vidéo 🤯".to_string(),
            "Le moment le plus important ⚡".to_string(),
        ]
    } else {
        vec![
            primary_hook,
            "You won't believe what happens next 🤯".to_string(),
            "Watch closely right here ⚡".to_string(),
        ]
    };

    // 4. Speech Pacing & Dead-Air Trimmer (Phase 2)
    // Detect awkward pauses (>550ms - 750ms) and apply 100ms breathing room padding
    let cuts = detect_dead_air_cuts(&clip_words, base_start, dur, style_preset);
    let total_cuts_dur: f64 = cuts.iter().map(|c| c.t1 - c.t0).sum();
    let effective_dur = (dur - total_cuts_dur).max(1.0);

    // 5. Story-Aware Camera Punch-Ins
    // A. Hook punch-in (0.0s to ~2.2s): immediate visual engagement on the edited timeline
    let mut effects = Vec::new();
    let hook_punch_dur = 2.4f64.min(effective_dur * 0.35).max(1.0);
    effects.push(EdlEffect {
        t: 0.0,
        dur: hook_punch_dur,
        r#type: "punch_in".to_string(),
        intensity: match style_preset {
            "podcast_clean" => 0.09,
            "cinematic_story" => 0.08,
            _ => 0.16,
        },
        reason: "Opening hook visual punch to seize immediate viewer retention".to_string(),
    });

    // B. Sentence & Narrative transition punch-ins mapped to post-cut timeline
    let mut last_punch_t = 0.0;
    let mut last_punch_end = hook_punch_dur;
    for i in 0..clip_words.len().saturating_sub(1) {
        let curr = clip_words[i];
        let next = clip_words[i + 1];

        let curr_end_rel = (curr.end - base_start).max(0.0);
        let next_start_rel = (next.start - base_start).max(0.0);
        let pause_dur = next_start_rel - curr_end_rel;

        let has_terminal_punct = curr.text.ends_with('.') || curr.text.ends_with('!') || curr.text.ends_with('?') || curr.text.ends_with('،');
        let is_sentence_transition = has_terminal_punct || pause_dur >= 0.35;

        if is_sentence_transition {
            // Remap start time to post-cut timeline
            let remapped_start = remap_timestamp(next_start_rel, &cuts);

            // Rule: Max 1 major punch per 3s, with at least 1.2s spacing after previous punch
            if remapped_start >= last_punch_t + 3.0 && remapped_start >= last_punch_end + 1.2 && remapped_start + 1.8 < effective_dur {
                let p_dur = 2.2f64.min(effective_dur - remapped_start - 0.5);
                if p_dur >= 1.0 {
                    let next_word_preview = next.text.trim().to_string();
                    effects.push(EdlEffect {
                        t: (remapped_start * 10.0).round() / 10.0,
                        dur: (p_dur * 10.0).round() / 10.0,
                        r#type: "punch_in".to_string(),
                        intensity: match style_preset {
                            "podcast_clean" => 0.08,
                            "cinematic_story" => 0.10,
                            _ => 0.14,
                        },
                        reason: format!("Sentence transition emphasis punch on '{}'", next_word_preview),
                    });
                    last_punch_t = remapped_start;
                    last_punch_end = remapped_start + p_dur;
                }
            }
        }
    }

    // 6. Keyword Emphasis Extraction
    // Automatically identify high-impact words (numbers, key metrics, superlatives, strong words)
    let impact_keywords: &[&str] = &[
        "secret", "never", "always", "money", "mistake", "danger", "crucial", "important",
        "problem", "solution", "viral", "billion", "million", "percent", "win", "lose",
        "argent", "jamais", "toujours", "important", "erreur", "facile", "meilleur",
        "سر", "مهم", "أفضل", "خطأ", "ربح", "مال", "أكبر", "فرصة", "حقيقي"
    ];

    let mut keyword_emphasis = Vec::new();
    for word in &clip_words {
        let clean = word.text.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        if clean.len() >= 3 {
            let is_number = clean.chars().any(|c| c.is_numeric());
            let is_impact = impact_keywords.iter().any(|k| clean.contains(k));
            if is_number || is_impact {
                if !keyword_emphasis.contains(&clean) && keyword_emphasis.len() < 8 {
                    keyword_emphasis.push(clean);
                }
            }
        }
    }

    // 7. Style presets customization
    let (font, words_per_line, highlight_color, why_style) = match style_preset {
        "podcast_clean" => (
            if has_arabic { "Cairo".to_string() } else { "Inter".to_string() },
            4,
            Some("#00F0FF".to_string()), // cyan accent
            "Clean podcast aesthetic: restrained framing, calm sentence transitions, readable typography.".to_string(),
        ),
        "cinematic_story" => (
            if has_arabic { "Tajawal".to_string() } else { "Montserrat".to_string() },
            3,
            Some("#E2B714".to_string()), // warm gold accent
            "Cinematic narrative flow: ambient depth blur, dramatic pacing, elegant typography.".to_string(),
        ),
        "educational_explainer" => (
            if has_arabic { "Cairo".to_string() } else { "Roboto".to_string() },
            3,
            Some("#00FF66".to_string()), // crisp lime accent
            "Educational clarity: keyword callouts, high-contrast readability, steady framing.".to_string(),
        ),
        _ => (
            if has_arabic { "Cairo".to_string() } else { "Arial Black".to_string() },
            3,
            Some("#FFE600".to_string()), // viral electric yellow
            "High-energy viral style: hook punch urgency, pop kinetic typography, contrasting keyword highlights.".to_string(),
        ),
    };

    Edl {
        version: 1,
        source: EdlSource {
            video_id: clip.id.clone(),
            start: clip.start_s,
            end: clip.end_s,
        },
        content_type: if has_arabic { "arabic_vlog".to_string() } else { "talking_head".to_string() },
        language: lang.to_string(),
        style_preset: style_preset.to_string(),
        why_this_style: why_style,
        hook: EdlHook {
            options: hook_options,
            chosen: 0,
            strategy: "cold_open".to_string(),
            pull_from: EdlTimeRange { t0: 0.0, t1: hook_punch_dur },
        },
        cuts,
        effects,
        overlays: Vec::new(),
        captions: EdlCaptions {
            style: "bold_pop".to_string(),
            words_per_line,
            highlight: "word".to_string(),
            font,
            position: "lower_middle".to_string(),
            rtl: has_arabic,
            sync_nudge_ms: 0,
            keyword_emphasis,
            highlight_color,
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
            mode: if style_preset == "gaming_fast" { "framed_blur".to_string() } else { "auto".to_string() },
            keyframes: Vec::new(),
        },
        end: EdlEnd {
            loop_friendly: true,
            cta: None,
        },
    }
}

/// Speech pause analyzer with 80-120ms safety breathing padding.
/// Identifies awkward silences / dead air between words without cutting speech phonemes.
pub fn detect_dead_air_cuts(
    words: &[&crate::common::Word],
    base_start: f64,
    clip_dur: f64,
    style_preset: &str,
) -> Vec<EdlCut> {
    if words.is_empty() {
        return Vec::new();
    }

    // Silence threshold depends on creative style
    let threshold = match style_preset {
        "viral_high_energy" | "gaming_fast" => 0.55,
        "podcast_clean" => 0.70,
        "educational_explainer" => 0.75,
        "cinematic_story" => 1.20,
        _ => 0.65,
    };

    let mut raw_cuts: Vec<EdlCut> = Vec::new();

    // 1. Check leading silence (dead air before first word)
    let first_word_start = (words[0].start - base_start).max(0.0);
    if first_word_start >= 0.8 {
        let cut_end = (first_word_start - 0.10).max(0.0);
        if cut_end >= 0.20 {
            raw_cuts.push(EdlCut {
                t0: 0.0,
                t1: (cut_end * 100.0).round() / 100.0,
                reason: "intro_silence".to_string(),
            });
        }
    }

    // 2. Check pauses between adjacent words
    for i in 0..words.len().saturating_sub(1) {
        let curr = words[i];
        let next = words[i + 1];

        let curr_end = (curr.end - base_start).max(0.0);
        let next_start = (next.start - base_start).max(0.0);
        let pause = next_start - curr_end;

        if pause >= threshold {
            // Keep 100ms breathing room after curr_word and 100ms before next_word
            let cut_t0 = curr_end + 0.10;
            let cut_t1 = next_start - 0.10;

            if cut_t1 - cut_t0 >= 0.18 && cut_t0 < clip_dur && cut_t1 <= clip_dur {
                raw_cuts.push(EdlCut {
                    t0: (cut_t0 * 100.0).round() / 100.0,
                    t1: (cut_t1 * 100.0).round() / 100.0,
                    reason: "dead_air".to_string(),
                });
            }
        }
    }

    // 3. Check trailing silence (dead air after last word)
    let last_word_end = (words.last().unwrap().end - base_start).max(0.0);
    if clip_dur - last_word_end >= 0.8 {
        let cut_t0 = last_word_end + 0.10;
        if clip_dur - cut_t0 >= 0.20 {
            raw_cuts.push(EdlCut {
                t0: (cut_t0 * 100.0).round() / 100.0,
                t1: (clip_dur * 100.0).round() / 100.0,
                reason: "outro_silence".to_string(),
            });
        }
    }

    // Sort cuts chronologically
    raw_cuts.sort_by(|a, b| a.t0.partial_cmp(&b.t0).unwrap_or(std::cmp::Ordering::Equal));

    // Merge overlapping or closely adjacent cuts (<0.10s apart)
    let mut merged_cuts: Vec<EdlCut> = Vec::new();
    for cut in raw_cuts {
        if let Some(prev) = merged_cuts.last_mut() {
            if cut.t0 <= prev.t1 + 0.10 {
                prev.t1 = prev.t1.max(cut.t1);
                continue;
            }
        }
        merged_cuts.push(cut);
    }

    // Safety guard: Limit total cut duration to at most 35% of clip duration
    let max_allowed_cut_dur = clip_dur * 0.35;
    let mut total_cut = 0.0;
    let mut safe_cuts = Vec::new();
    for c in merged_cuts {
        let cut_len = c.t1 - c.t0;
        if total_cut + cut_len <= max_allowed_cut_dur {
            total_cut += cut_len;
            safe_cuts.push(c);
        } else {
            let remaining = max_allowed_cut_dur - total_cut;
            if remaining >= 0.20 {
                safe_cuts.push(EdlCut {
                    t0: c.t0,
                    t1: c.t0 + remaining,
                    reason: c.reason,
                });
            }
            break;
        }
    }

    safe_cuts
}

/// Remaps a timestamp from the original clip timeline to the edited (jump-cut) timeline.
pub fn remap_timestamp(t: f64, cuts: &[EdlCut]) -> f64 {
    let mut shifted = t;
    for cut in cuts {
        if t >= cut.t1 {
            shifted -= cut.t1 - cut.t0;
        } else if t > cut.t0 {
            // Timestamp landed inside cut silence: clamp to cut start in new timeline
            let cut_offset_before: f64 = cuts.iter().take_while(|c| c.t1 <= cut.t0).map(|c| c.t1 - c.t0).sum();
            return (cut.t0 - cut_offset_before).max(0.0);
        }
    }
    shifted.max(0.0)
}

/// Remaps all transcript segments and words to post-cut timestamps for exact subtitle synchronization.
pub fn remap_transcript_for_cuts(transcript: &Transcript, cuts: &[EdlCut]) -> Transcript {
    if cuts.is_empty() {
        return transcript.clone();
    }

    let mut new_segments = Vec::new();
    for seg in &transcript.segments {
        let mut new_words = Vec::new();
        for w in &seg.words {
            let remapped_start = remap_timestamp(w.start, cuts);
            let remapped_end = remap_timestamp(w.end, cuts);
            if remapped_end > remapped_start {
                new_words.push(crate::common::Word {
                    start: remapped_start,
                    end: remapped_end,
                    text: w.text.clone(),
                    confidence: w.confidence,
                });
            }
        }

        if !new_words.is_empty() {
            let s_start = new_words.first().unwrap().start;
            let s_end = new_words.last().unwrap().end;
            new_segments.push(crate::common::TranscriptSegment {
                start: s_start,
                end: s_end,
                text: seg.text.clone(),
                words: new_words,
            });
        }
    }

    let total_cut_dur: f64 = cuts.iter().map(|c| c.t1 - c.t0).sum();
    let new_dur = (transcript.duration - total_cut_dur).max(1.0);

    Transcript {
        language: transcript.language.clone(),
        duration: new_dur,
        segments: new_segments,
        is_clip_relative: transcript.is_clip_relative,
    }
}

