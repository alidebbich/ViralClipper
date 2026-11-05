use anyhow::Result;
use clipper_lib::common::{Clip, Transcript};
use clipper_lib::render::*;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use sysinfo::{Pid, System};

#[tokio::main]
async fn main() -> Result<()> {
    // Usage: render_matrix_v2 <video_path> <transcript_path> <output_dir> <template_id> [--cpu] [--edl] [--title "Title"] [--start 0.0] [--dur 20.0]
    let args: Vec<String> = env::args().collect();
    if args.len() < 5 {
        eprintln!("Usage: render_matrix_v2 <video_path> <transcript_path> <output_dir> <template_id> [--cpu] [--edl] [--title <title>] [--start <s>] [--dur <s>]");
        std::process::exit(1);
    }

    let video_path = &args[1];
    let transcript_path = &args[2];
    let output_dir = PathBuf::from(&args[3]);
    let template_id = &args[4];

    let force_cpu = args.iter().any(|a| a == "--cpu");
    let enable_edl = args.iter().any(|a| a == "--edl");

    let title = args.windows(2)
        .find(|w| w[0] == "--title")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "Test Render".to_string());

    let start_s = args.windows(2)
        .find(|w| w[0] == "--start")
        .and_then(|w| w[1].parse::<f64>().ok())
        .unwrap_or(0.0);

    let transcript_str = fs::read_to_string(transcript_path)?;
    let clean_str = transcript_str.trim_start_matches('\u{feff}');
    let transcript: Transcript = serde_json::from_str(clean_str)?;

    let dur_s = args.windows(2)
        .find(|w| w[0] == "--dur")
        .and_then(|w| w[1].parse::<f64>().ok())
        .unwrap_or_else(|| (transcript.duration - start_s).max(1.0));

    fs::create_dir_all(&output_dir)?;

    // Pick template
    let templates = clipper_lib::templates::default_templates();
    let template = templates
        .into_iter()
        .find(|t| t.id == *template_id)
        .ok_or_else(|| anyhow::anyhow!("Template '{}' not found", template_id))?;

    let mut clip = Clip {
        id: format!("{}_{}", template_id, if force_cpu { "x264" } else { "nvenc" }),
        project_id: "matrix_v2".to_string(),
        start_s,
        end_s: start_s + dur_s,
        duration_s: dur_s,
        title,
        heuristic_json: None,
        llm_reason: None,
        template_id: Some(template_id.clone()),
        render_path: None,
        status: "rendering".to_string(),
        hook_segments: None,
        montage_order: None,
        hook_score: Some(95.0),
        viral_score: Some(92.0),
        is_clip_relative: false,
        caption_warning: None,
        timeline_data: None,
    };

    if enable_edl {
        // Handcrafted dynamic EDL with jump cuts and zoom punch
        let edl_json = serde_json::json!({
            "framing": {"mode": "smart_crop"},
            "cuts": [
                {"t0": start_s + 4.0, "t1": start_s + 5.5}
            ],
            "effects": [
                {"type": "punch_in", "t": 1.0, "dur": 2.0, "intensity": 0.8},
                {"type": "zoom_ramp", "t": 6.0, "dur": 1.8, "intensity": 0.9}
            ],
            "audio": {
                "sfx": [
                    {"t": 0.2, "type": "whoosh"},
                    {"t": 1.0, "type": "pop"},
                    {"t": 6.0, "type": "hit"}
                ]
            },
            "hook": {"options": ["SHOCKING MOMENT", "WATCH TILL THE END"], "chosen": 0}
        });
        clip.timeline_data = Some(edl_json.to_string());
    }

    let out_file = output_dir.join(format!("{}.mp4", clip.id));
    let out_file_str = out_file.to_string_lossy().to_string();

    let assets = resolve_assets();
    let src = probe_source(video_path).await;
    let face = if src.w > src.h {
        detect_faces(video_path, clip.start_s, clip.duration_s).await
    } else {
        FaceStats::default()
    };

    let words = collect_clip_words(&clip, &transcript.segments, transcript.is_clip_relative);
    let plan = plan_edit(&clip, &template, &words, src.w, src.h, &face, assets.bg_video.is_some());

    println!("PLAN: framing={:?}, pieces={}, out_dur={:.2}s, punches={}, sfx={}, pan_keys={}",
        plan.framing, plan.timeline.pieces.len(), plan.timeline.out_duration, plan.punches.len(), plan.sfx.len(), plan.pan_keys.len());

    let ass_path = generate_ass_file_with_timeline(&clip, &transcript.segments, &template, transcript.is_clip_relative, &plan.timeline)?;

    let params = EditParams {
        template: &template,
        ass_path: &ass_path,
        fonts_dir: &assets.fonts_dir,
        src: src.clone(),
        timeline: &plan.timeline,
        framing: plan.framing,
        pan_keys: &plan.pan_keys,
        punches: &plan.punches,
        sfx: &plan.sfx,
        sfx_files: &assets.sfx,
        bg_video: assets.bg_video.as_deref(),
        music: assets.music.as_deref(),
    };
    let graph = build_edit_graph(&params)?;

    let mut sys = System::new_all();
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_process(pid);
    let start_mem_mb = sys.process(pid).map(|p| p.memory() as f64 / 1024.0 / 1024.0).unwrap_or(0.0);

    let t0 = Instant::now();
    let enc_used = run_graph(video_path, clip.start_s, clip.duration_s, &graph, &out_file_str, !force_cpu).await?;
    let render_time_s = t0.elapsed().as_secs_f64();

    sys.refresh_process(pid);
    let end_mem_mb = sys.process(pid).map(|p| p.memory() as f64 / 1024.0 / 1024.0).unwrap_or(0.0);

    let verified_dur = verify_rendered_file(&out_file_str).await?;
    let file_size_bytes = fs::metadata(&out_file)?.len();

    let speed_mult = if render_time_s > 0.0 { verified_dur / render_time_s } else { 0.0 };

    // Extract contact sheet (1 frame per second)
    let contact_sheet = output_dir.join(format!("{}_contact.jpg", clip.id));
    let contact_sheet_str = contact_sheet.to_string_lossy().to_string();
    let ffmpeg_exe = resolve_ffmpeg();

    let _ = tokio::process::Command::new(&ffmpeg_exe)
        .args([
            "-y", "-i", &out_file_str,
            "-vf", "fps=1,scale=270:480,tile=5x4",
            "-q:v", "2",
            &contact_sheet_str
        ])
        .output()
        .await;

    // Extract 3 key frames: beginning (t=1.0s), middle, end (t=dur-1.0s)
    let f_start = output_dir.join(format!("{}_f_start.jpg", clip.id));
    let f_mid = output_dir.join(format!("{}_f_mid.jpg", clip.id));
    let f_end = output_dir.join(format!("{}_f_end.jpg", clip.id));

    let _ = extract_clip_thumbnail(&out_file_str, &f_start.to_string_lossy(), 1.0f64.min(verified_dur * 0.1)).await;
    let _ = extract_clip_thumbnail(&out_file_str, &f_mid.to_string_lossy(), verified_dur * 0.5).await;
    let _ = extract_clip_thumbnail(&out_file_str, &f_end.to_string_lossy(), (verified_dur - 0.8).max(0.0)).await;

    // Measure loudness using ebur128
    let loud_out = tokio::process::Command::new(&ffmpeg_exe)
        .args([
            "-i", &out_file_str,
            "-af", "ebur128",
            "-f", "null", "-"
        ])
        .output()
        .await;

    let mut integrated_lufs = "unknown".to_string();
    let mut true_peak = "unknown".to_string();
    if let Ok(lo) = loud_out {
        let err_str = String::from_utf8_lossy(&lo.stderr);
        for line in err_str.lines() {
            if line.contains("I:") && line.contains("LUFS") {
                integrated_lufs = line.trim().to_string();
            }
            if line.contains("Peak:") && line.contains("dBFS") {
                true_peak = line.trim().to_string();
            }
        }
    }

    let report = serde_json::json!({
        "clip_id": clip.id,
        "template": template_id,
        "encoder": if enc_used == Encoder::Nvenc { "NVENC" } else { "x264" },
        "requested_encoder": if force_cpu { "x264" } else { "NVENC" },
        "out_file": out_file_str,
        "size_bytes": file_size_bytes,
        "duration_s": verified_dur,
        "render_time_s": render_time_s,
        "speed_factor": speed_mult,
        "mem_start_mb": start_mem_mb,
        "mem_end_mb": end_mem_mb,
        "framing": format!("{:?}", plan.framing),
        "punch_count": plan.punches.len(),
        "sfx_count": plan.sfx.len(),
        "pieces_count": plan.timeline.pieces.len(),
        "integrated_lufs": integrated_lufs,
        "true_peak": true_peak,
        "contact_sheet": contact_sheet_str,
        "frames": {
            "start": f_start.to_string_lossy(),
            "mid": f_mid.to_string_lossy(),
            "end": f_end.to_string_lossy()
        }
    });

    println!("RESULT_JSON={}", serde_json::to_string(&report)?);
    Ok(())
}
