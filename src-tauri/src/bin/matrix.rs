use anyhow::Result;
use clipper_lib::common::Transcript;
use clipper_lib::pipeline::analyze::analyze_and_score;
use clipper_lib::ffmpeg::render_clip;
use std::env;
use std::fs;

#[tokio::main]
async fn main() -> Result<()> {
    // Arguments: matrix <video_path> <transcript_json_path>
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: matrix <video_path> <transcript_json_path>");
        std::process::exit(1);
    }

    let _video_path = &args[1];
    let transcript_path = &args[2];

    // Load transcript
    let transcript_str = fs::read_to_string(transcript_path)?;
    let transcript: Transcript = serde_json::from_str(&transcript_str)?;

    println!("Loaded transcript: {} segments, {:.1}s", transcript.segments.len(), transcript.duration);

    // Run AI Clip Finder
    let max_clips = 5;
    println!("Analyzing and scoring...");
    let clips = analyze_and_score(&transcript, max_clips).await?;

    println!("Found {} clips:", clips.len());
    for (i, clip) in clips.iter().enumerate() {
        println!("Clip {}: {:.1}s - {:.1}s (Score: {:?})", i + 1, clip.start_s, clip.end_s, clip.viral_score);
    }

    if let Some(best_clip) = clips.first() {
        let template = clipper_lib::templates::default_templates()
            .into_iter()
            .find(|t| t.id == "tiktok")
            .unwrap_or_else(|| clipper_lib::templates::default_templates().remove(0));

        let out_dir = env::current_dir()?.join("..").join("docs").join("matrix_outputs");
        fs::create_dir_all(&out_dir)?;
        env::set_var("LOCALAPPDATA", out_dir.to_str().unwrap());

        println!("Rendering best clip...");
        match render_clip(_video_path, best_clip, &template, "matrix_test", &transcript).await {
            Ok((rendered_path, thumb_path, _caption_warning)) => {
                println!("RENDERED={}", rendered_path);
                println!("THUMB={}", thumb_path);
                
                // Copy the rendered output to a more visible directory
                let final_vid = out_dir.join(format!("{}.mp4", best_clip.id));
                let final_thumb = out_dir.join(format!("{}.jpg", best_clip.id));
                if fs::copy(&rendered_path, &final_vid).is_ok() {
                    println!("COPIED_VID={}", final_vid.display());
                }
                if fs::copy(&thumb_path, &final_thumb).is_ok() {
                    println!("COPIED_THUMB={}", final_thumb.display());
                }
            },
            Err(e) => eprintln!("Render failed: {}", e),
        }
    }

    Ok(())
}
