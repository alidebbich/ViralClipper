use anyhow::Result;
use clipper_lib::common::{Clip, Transcript};
use clipper_lib::ffmpeg::render_clip;
use std::env;
use std::fs;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let video_path = &args[1];
    let transcript_path = &args[2];

    let transcript_str = fs::read_to_string(transcript_path)?;
    let transcript: Transcript = serde_json::from_str(&transcript_str)?;

    let start_s = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    let end_s = args.get(4).and_then(|s| s.parse::<f64>().ok()).unwrap_or(transcript.duration);
    let title = args.get(5).cloned().unwrap_or_else(|| "Force Render Hook".to_string());

    let mut clip = Clip::new(
        "ali_clip_rendered".to_string(),
        start_s,
        end_s,
        title,
    );
    clip.hook_score = Some(99.0);
    clip.viral_score = Some(95.0);

    let template = clipper_lib::templates::default_templates()
        .into_iter()
        .find(|t| t.id == "tiktok")
        .unwrap_or_else(|| clipper_lib::templates::default_templates().remove(0));

    // Inject a handcrafted, highly dynamic EDL to prove the engine works
    let edl_json = r#"{
        "version": 1,
        "source": {"video_id": "test", "start": 0.0, "end": 12.0},
        "content_type": "gaming",
        "language": "en",
        "style_preset": "gaming_fast",
        "why_this_style": "High energy test.",
        "hook": {"options": ["WOW", "LOOK", "OMG"], "chosen": 0, "strategy": "cold_open", "pull_from": {"t0": 0.0, "t1": 2.0}},
        "cuts": [],
        "effects": [
            {"t": 1.0, "dur": 2.0, "type": "zoom_ramp", "intensity": 1.0, "reason": "Build tension"},
            {"t": 3.0, "dur": 0.5, "type": "shake", "intensity": 1.0, "reason": "Impact!"},
            {"t": 3.5, "dur": 0.2, "type": "flash", "intensity": 1.0, "reason": "Flash on reveal"},
            {"t": 4.0, "dur": 3.0, "type": "punch_in", "intensity": 1.0, "reason": "Focus on face"}
        ],
        "overlays": [
            {"t": 2.0, "dur": 1.0, "type": "keyword_pop", "text": "WAIT FOR IT!", "reason": "Hook retention"},
            {"t": 5.0, "dur": 2.0, "type": "keyword_pop", "text": "BANGER!", "reason": "Mind blown"}
        ],
        "captions": {"style": "bold_pop", "words_per_line": 3, "highlight": "word", "font": "Arial", "position": "lower_middle", "rtl": false},
        "audio": {"loudnorm_lufs": -14, "music": {"mood": "intense", "duck_db": -14, "file": null}, "sfx": []},
        "framing": {"mode": "blur_fill", "keyframes": []},
        "end": {"loop_friendly": true, "cta": null}
    }"#;
    if args.len() > 6 && args[6] == "edl" {
        clip.heuristic_json = Some(edl_json.to_string());
    } else {
        clip.heuristic_json = None;
    }

    let out_dir = env::current_dir()?.join("..").join("docs").join("matrix_outputs");
    fs::create_dir_all(&out_dir)?;
    env::set_var("LOCALAPPDATA", out_dir.to_str().unwrap());

    println!("Rendering forced clip...");
    match render_clip(video_path, &clip, &template, "matrix_test", &transcript).await {
        Ok((rendered_path, thumb_path, _caption_warning)) => {
            let final_vid = out_dir.join(format!("{}.mp4", clip.id));
            let final_thumb = out_dir.join(format!("{}.jpg", clip.id));
            fs::copy(&rendered_path, &final_vid)?;
            fs::copy(&thumb_path, &final_thumb)?;
            println!("COPIED_VID={}", final_vid.display());
            println!("COPIED_THUMB={}", final_thumb.display());
        },
        Err(e) => eprintln!("Render failed: {}", e),
    }

    Ok(())
}
