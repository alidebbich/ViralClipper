use anyhow::Result;
use clipper_lib::common::{Clip, Transcript};
use clipper_lib::ffmpeg::render_clip;
use std::env;
use std::fs;

#[tokio::main]
async fn main() -> Result<()> {
    // Arguments: smoke <video_path> <transcript_json_path> <output_dir>
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: smoke <video_path> <transcript_json_path> <output_dir>");
        std::process::exit(1);
    }

    let video_path = &args[1];
    let transcript_path = &args[2];
    let output_dir = &args[3];

    // Load transcript
    let transcript_str = fs::read_to_string(transcript_path)?;
    let transcript: Transcript = serde_json::from_str(&transcript_str)?;

    let duration = transcript.duration;

    // Create a dummy clip spanning the whole video
    let clip = Clip {
        id: "smoke_test_clip".to_string(),
        project_id: "smoke".to_string(),
        start_s: 0.0,
        end_s: duration,
        duration_s: duration,
        title: "Smoke Test Hook".to_string(),
        heuristic_json: None,
        llm_reason: None,
        template_id: Some("tiktok".to_string()),
        render_path: None,
        status: "rendering".to_string(),
        hook_segments: None,
        montage_order: None,
        hook_score: None,
        viral_score: None,
        is_clip_relative: false,
        caption_warning: None,
        timeline_data: None,
    };

    let template = clipper_lib::templates::default_templates()
        .into_iter()
        .find(|t| t.id == "tiktok")
        .unwrap_or_else(|| clipper_lib::templates::default_templates().remove(0));

    // Ensure LOCALAPPDATA points to something we can control
    env::set_var("LOCALAPPDATA", output_dir);

    println!("Rendering...");
    let (rendered_path, thumb_path, _caption_warning) = render_clip(
        video_path,
        &clip,
        &template,
        "smoke",
        &transcript,
    ).await?;

    println!("RENDERED={}", rendered_path);
    println!("THUMB={}", thumb_path);

    Ok(())
}
