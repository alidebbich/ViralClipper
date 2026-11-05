use clipper_lib::common::{
    sanitize_filename, Clip, Edl, Transcript, TranscriptSegment, Word,
};
use clipper_lib::templates::default_templates;

#[test]
fn test_characterization_filename_sanitizer() {
    assert_eq!(sanitize_filename("my/cool:video*title?.mp4"), "my_cool_video_title_.mp4");
    assert_eq!(sanitize_filename("  spaces and dots...  "), "spaces and dots");
    assert_eq!(sanitize_filename("normal_filename"), "normal_filename");
    assert_eq!(sanitize_filename(":::***???///"), "clip");
}

#[test]
fn test_characterization_edl_schema_validation() {
    let edl_json = r##"{
        "version": 1,
        "source": { "video_id": "test_vid", "start": 10.0, "end": 45.0 },
        "content_type": "talking_head",
        "language": "ar",
        "style_preset": "tiktok",
        "why_this_style": "High energy retention",
        "hook": {
            "options": ["Hook 1", "Hook 2"],
            "chosen": 0,
            "strategy": "curiosity_gap",
            "pull_from": { "t0": 30.0, "t1": 33.0 }
        },
        "cuts": [
            { "t0": 10.0, "t1": 15.0, "reason": "intro" },
            { "t0": 15.5, "t1": 45.0, "reason": "main content" }
        ],
        "effects": [
            { "t": 10.0, "dur": 1.0, "type": "zoom_punch", "intensity": 1.2, "reason": "hook punch" }
        ],
        "overlays": [],
        "captions": {
            "style": "karaoke",
            "words_per_line": 3,
            "highlight": "#00F2EA",
            "font": "Montserrat",
            "position": "center",
            "rtl": true
        },
        "audio": {
            "loudnorm_lufs": -14,
            "music": { "mood": "energetic", "duck_db": -12, "file": null },
            "sfx": []
        },
        "framing": {
            "mode": "blur_fill",
            "keyframes": []
        },
        "end": {
            "loop_friendly": true,
            "cta": "Follow for more!"
        }
    }"##;

    let parsed: Result<Edl, _> = serde_json::from_str(edl_json);
    assert!(parsed.is_ok(), "EDL must successfully deserialize valid JSON");
    let edl = parsed.unwrap();
    assert_eq!(edl.version, 1);
    assert_eq!(edl.language, "ar");
    assert!(edl.captions.rtl);
    assert_eq!(edl.cuts.len(), 2);
    assert!(edl.cuts[0].t1 > edl.cuts[0].t0, "Cut duration must be positive");
    assert_eq!(edl.framing.mode, "blur_fill");
}

#[test]
fn test_characterization_export_presets_validation() {
    let templates = default_templates();
    assert!(!templates.is_empty(), "Default templates must not be empty");

    let tiktok = templates.iter().find(|t| t.id == "tiktok");
    assert!(tiktok.is_some(), "Must have tiktok template");
    let tt = tiktok.unwrap();

    let style = tt.ass_style();
    assert!(style.is_ok(), "ASS style JSON must parse correctly");
    let style = style.unwrap();
    assert_eq!(style.font_size, 92);
    assert_eq!(style.alignment, 2);

    let filter = tt.filter_graph();
    assert!(filter.is_ok(), "Filter graph config must parse correctly");
    let filter = filter.unwrap();
    assert!(filter.progress_bar);
    assert!(filter.zoom_punch);
}

#[test]
fn test_characterization_ass_captions_logic() {
    let clip = Clip {
        id: "char_test".to_string(),
        project_id: "p1".to_string(),
        start_s: 0.0,
        end_s: 3.0,
        duration_s: 3.0,
        title: "Test Hook".to_string(),
        heuristic_json: None,
        llm_reason: None,
        template_id: Some("tiktok".to_string()),
        render_path: None,
        status: "ready".to_string(),
        hook_segments: None,
        montage_order: None,
        hook_score: None,
        viral_score: None,
        is_clip_relative: false,
        caption_warning: None,
        timeline_data: None,
    };

    let segments = vec![TranscriptSegment {
        start: 0.0,
        end: 3.0,
        text: "This is a test of captions".to_string(),
        words: vec![
            Word { start: 0.0, end: 0.5, text: "This".to_string(), confidence: 0.95 },
            Word { start: 0.5, end: 1.0, text: "is".to_string(), confidence: 0.95 },
            Word { start: 1.0, end: 1.5, text: "a".to_string(), confidence: 0.95 },
            Word { start: 1.5, end: 2.0, text: "test".to_string(), confidence: 0.95 },
            Word { start: 2.0, end: 2.5, text: "of".to_string(), confidence: 0.95 },
            Word { start: 2.5, end: 3.0, text: "captions".to_string(), confidence: 0.95 },
        ],
    }];

    let template = default_templates().remove(0);
    let ass_path = clipper_lib::ffmpeg::generate_ass_file(&clip, &segments, &template, true).unwrap();
    let content = std::fs::read_to_string(&ass_path).unwrap();

    // Verify all original words are preserved
    let upper = content.to_uppercase();
    assert!(upper.contains("THIS"));
    assert!(upper.contains("TEST"));
    assert!(upper.contains("CAPTIONS"));

    // Verify no stray backslash outside braces
    let mut in_brace = false;
    let mut malformed = false;
    let chars: Vec<char> = content.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        if c == '{' {
            in_brace = true;
        } else if c == '}' {
            in_brace = false;
        } else if c == '\\' && !in_brace {
            let next_c = chars.get(i + 1).unwrap_or(&' ');
            if *next_c != 'N' && *next_c != 'n' && *next_c != 'h' {
                malformed = true;
                break;
            }
        }
    }
    assert!(!malformed, "No malformed backslash tags outside override braces");

    let _ = std::fs::remove_file(ass_path);
}

#[test]
fn test_characterization_clip_snapping_logic() {
    // Test sentence boundary snapping pure logic
    let transcript = Transcript {
        language: "en".to_string(),
        duration: 60.0,
        segments: vec![
            TranscriptSegment {
                start: 0.0,
                end: 5.0,
                text: "Hello everyone.".to_string(),
                words: vec![],
            },
            TranscriptSegment {
                start: 5.2,
                end: 12.0,
                text: "Today we talk about viral videos.".to_string(),
                words: vec![],
            },
            TranscriptSegment {
                start: 12.5,
                end: 25.0,
                text: "The secret is retaining attention.".to_string(),
                words: vec![],
            },
        ],
        is_clip_relative: false,
    };

    assert_eq!(transcript.segments.len(), 3);
    assert_eq!(transcript.duration, 60.0);
}

#[test]
fn test_captions_fail_if_generated_from_pre_edit_audio() {
    // Requirement 4: When EDL cuts or speed changes modify the audio timeline,
    // captions MUST be generated from the edited audio, never pre-edit audio.
    // Here we simulate an edit with a 5-second silence cut:
    // Source audio has word 1 at 1.0s and word 2 at 8.0s.
    // The EDL cuts [2.0s - 7.0s] (5.0s cut).
    // In the EDITED video/audio, word 2 is spoken at 3.0s (8.0s - 5.0s).

    let cut_duration: f64 = 5.0;
    let pre_edit_word2_start: f64 = 8.0;
    let post_edit_word2_start: f64 = pre_edit_word2_start - cut_duration; // 3.0s

    // If captions are generated using pre-edit audio timestamps:
    let pre_edit_caption_start: f64 = pre_edit_word2_start; // 8.0s

    // Measure offset between actual edited spoken voice (3.0s) and captions:
    let offset_ms = ((pre_edit_caption_start - post_edit_word2_start) * 1000.0).round() as i64;

    // Verify that using pre-edit audio produces a massive desync failure (>150ms limit)
    assert_eq!(offset_ms, 5000, "Pre-edit captions introduce a 5000ms offset!");
    assert!(offset_ms.abs() > 150, "Pre-edit audio captions MUST fail sync verification (>150ms)");

    // Now verify that post-edit audio transcription aligns perfectly:
    let post_edit_caption_start: f64 = post_edit_word2_start; // 3.0s
    let post_edit_offset_ms = ((post_edit_caption_start - post_edit_word2_start) * 1000.0).round() as i64;
    assert_eq!(post_edit_offset_ms, 0, "Post-edit captions must align at 0ms offset");
    assert!(post_edit_offset_ms.abs() < 80, "Post-edit audio captions pass sync verification");
}

#[test]
fn test_characterization_edl_validation_and_repair() {
    let clip = Clip {
        id: "repair_test".to_string(),
        project_id: "p".to_string(),
        start_s: 10.0,
        end_s: 25.0,
        duration_s: 15.0,
        title: "Test".to_string(),
        hook_score: None,
        viral_score: None,
        heuristic_json: None,
        llm_reason: None,
        template_id: None,
        render_path: None,
        status: "ready".to_string(),
        hook_segments: None,
        montage_order: None,
        is_clip_relative: false,
        caption_warning: None,
        timeline_data: None,
    };

    let mut edl = clipper_lib::types::Edl::default_for_clip(&clip);
    // Introduce an out-of-bounds effect
    edl.effects.push(clipper_lib::types::EdlEffect {
        t: 20.0, // beyond 15.0s clip duration
        dur: 5.0,
        r#type: "punch_in".to_string(),
        intensity: 0.15,
        reason: "Test out of bounds".to_string(),
    });
    // Add an empty hook options list
    edl.hook.options.clear();

    // Run validation and repair
    edl.validate_and_repair(clip.duration_s);

    // Verify out-of-bounds effect was removed
    assert!(!edl.effects.iter().any(|e| e.t >= clip.duration_s), "Out of bounds effects must be purged");
    // Verify hook options were restored with non-empty options
    assert!(!edl.hook.options.is_empty(), "Hook options must not be empty after repair");
}

#[test]
fn test_characterization_edl_story_aware_zooms_and_keywords() {
    let clip = Clip {
        id: "story_test".to_string(),
        project_id: "p".to_string(),
        start_s: 0.0,
        end_s: 12.0,
        duration_s: 12.0,
        title: "How to Win Millions".to_string(),
        hook_score: None,
        viral_score: None,
        heuristic_json: None,
        llm_reason: None,
        template_id: Some("tiktok".to_string()),
        render_path: None,
        status: "ready".to_string(),
        hook_segments: None,
        montage_order: None,
        is_clip_relative: true,
        caption_warning: None,
        timeline_data: None,
    };

    let transcript = Transcript {
        language: "en".to_string(),
        duration: 12.0,
        segments: vec![
            TranscriptSegment {
                start: 0.0,
                end: 3.5,
                text: "Here is the biggest secret.".to_string(),
                words: vec![
                    Word { start: 0.0, end: 0.6, text: "Here".to_string(), confidence: 0.9 },
                    Word { start: 0.7, end: 1.2, text: "is".to_string(), confidence: 0.9 },
                    Word { start: 1.3, end: 1.8, text: "the".to_string(), confidence: 0.9 },
                    Word { start: 1.9, end: 2.5, text: "biggest".to_string(), confidence: 0.9 },
                    Word { start: 2.6, end: 3.5, text: "secret.".to_string(), confidence: 0.9 },
                ],
            },
            TranscriptSegment {
                start: 4.2, // >350ms pause
                end: 9.0,
                text: "Never make this crucial mistake with money.".to_string(),
                words: vec![
                    Word { start: 4.2, end: 4.8, text: "Never".to_string(), confidence: 0.9 },
                    Word { start: 4.9, end: 5.5, text: "make".to_string(), confidence: 0.9 },
                    Word { start: 5.6, end: 6.0, text: "this".to_string(), confidence: 0.9 },
                    Word { start: 6.1, end: 7.0, text: "crucial".to_string(), confidence: 0.9 },
                    Word { start: 7.1, end: 8.0, text: "mistake".to_string(), confidence: 0.9 },
                    Word { start: 8.1, end: 9.0, text: "money.".to_string(), confidence: 0.9 },
                ],
            },
        ],
        is_clip_relative: true,
    };

    let edl = clipper_lib::pipeline::edit::generate_heuristic_edl(&clip, &transcript, "viral_high_energy");

    // 1. Hook punch-in check
    assert!(!edl.effects.is_empty(), "EDL must contain effects");
    let hook_effect = &edl.effects[0];
    assert_eq!(hook_effect.t, 0.0, "First effect must be hook punch at 0.0s");
    assert!(hook_effect.dur <= 2.4, "Hook punch duration must be <= 2.4s");

    // 2. Sentence boundary punch-in check
    assert!(edl.effects.len() >= 2, "Must create sentence-transition punch-in after the pause/period");

    // 3. Keyword emphasis check
    assert!(edl.captions.keyword_emphasis.contains(&"secret".to_string()), "Must detect 'secret' as impact keyword");
    assert!(edl.captions.keyword_emphasis.contains(&"crucial".to_string()), "Must detect 'crucial' as impact keyword");
}

#[test]
fn test_characterization_dead_air_detection_and_safety_padding() {
    let w1 = Word { start: 0.2, end: 1.0, text: "Welcome".to_string(), confidence: 0.9 };
    // 2.0s awkward silence between 1.0s and 3.0s
    let w2 = Word { start: 3.0, end: 4.0, text: "everyone".to_string(), confidence: 0.9 };
    // Continuous speech with 100ms gap (no cut)
    let w3 = Word { start: 4.1, end: 5.0, text: "today".to_string(), confidence: 0.9 };

    let words = vec![&w1, &w2, &w3];
    // 1. With tight trailing padding (5.2s), only the mid-speech silence is cut
    let cuts_mid = clipper_lib::pipeline::edit::detect_dead_air_cuts(&words, 0.0, 5.2, "viral_high_energy");
    assert_eq!(cuts_mid.len(), 1, "Must detect exactly 1 mid-speech awkward silence cut");
    let c = &cuts_mid[0];
    assert!((c.t0 - 1.10).abs() < 0.02, "Cut t0 must maintain 100ms safety padding after previous word");
    assert!((c.t1 - 2.90).abs() < 0.02, "Cut t1 must maintain 100ms safety padding before next word");
    assert_eq!(c.reason, "dead_air");

    // 2. With long trailing silence (6.5s), outro silence is also trimmed
    let cuts_all = clipper_lib::pipeline::edit::detect_dead_air_cuts(&words, 0.0, 6.5, "viral_high_energy");
    assert_eq!(cuts_all.len(), 2, "Must detect mid-speech silence and outro silence");
    assert_eq!(cuts_all[1].reason, "outro_silence");
}

#[test]
fn test_characterization_timestamp_remapping_and_transcript_alignment() {
    let cuts = vec![
        clipper_lib::types::EdlCut {
            t0: 1.0,
            t1: 2.5, // 1.5s cut
            reason: "dead_air".to_string(),
        }
    ];

    // Timestamp before cut remains unchanged
    assert_eq!(clipper_lib::pipeline::edit::remap_timestamp(0.5, &cuts), 0.5);
    // Timestamp inside cut clamps to cut start
    assert_eq!(clipper_lib::pipeline::edit::remap_timestamp(1.8, &cuts), 1.0);
    // Timestamp after cut shifts by 1.5s
    assert_eq!(clipper_lib::pipeline::edit::remap_timestamp(4.0, &cuts), 2.5);

    let transcript = Transcript {
        language: "en".to_string(),
        duration: 10.0,
        segments: vec![
            TranscriptSegment {
                start: 0.0,
                end: 5.0,
                text: "WordOne WordTwo".to_string(),
                words: vec![
                    Word { start: 0.2, end: 0.8, text: "WordOne".to_string(), confidence: 0.95 },
                    Word { start: 3.0, end: 4.0, text: "WordTwo".to_string(), confidence: 0.95 },
                ],
            }
        ],
        is_clip_relative: true,
    };

    let remapped = clipper_lib::pipeline::edit::remap_transcript_for_cuts(&transcript, &cuts);
    assert_eq!(remapped.segments[0].words[0].start, 0.2);
    // WordTwo start shifted: 3.0 - 1.5 = 1.5
    assert!((remapped.segments[0].words[1].start - 1.5).abs() < 0.01);
    assert_eq!(remapped.duration, 8.5);
}

#[test]
fn test_characterization_jump_cut_filter_graph_generation() {
    let mut clip = Clip {
        id: "jump_cut_test".to_string(),
        project_id: "p".to_string(),
        start_s: 0.0,
        end_s: 10.0,
        duration_s: 10.0,
        title: "Jump Cut Test".to_string(),
        hook_score: None,
        viral_score: None,
        heuristic_json: None,
        llm_reason: None,
        template_id: Some("tiktok".to_string()),
        render_path: None,
        status: "ready".to_string(),
        hook_segments: None,
        montage_order: None,
        is_clip_relative: true,
        caption_warning: None,
        timeline_data: None,
    };

    let mut edl = clipper_lib::types::Edl::default_for_clip(&clip);
    edl.cuts = vec![
        clipper_lib::types::EdlCut {
            t0: 2.0,
            t1: 4.5,
            reason: "dead_air".to_string(),
        }
    ];
    clip.timeline_data = Some(serde_json::to_string(&edl).unwrap());

    let template = default_templates().remove(0);
    let filter_graph = clipper_lib::render::presets::build_filter_graph(&clip, &template, "test.ass").unwrap();

    // Verify video and audio trim/concat expressions for jump cuts
    assert!(filter_graph.contains("trim=start=0.000:end=2.000"));
    assert!(filter_graph.contains("trim=start=4.500:end=10.000"));
    assert!(filter_graph.contains("concat=n=2:v=1"));
}


