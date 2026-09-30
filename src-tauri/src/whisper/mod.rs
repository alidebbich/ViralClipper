use anyhow::Result;
use crate::common::{Transcript, Word};

pub async fn transcribe(app_handle: tauri::AppHandle, video_path: &str) -> Result<Transcript> {
    transcribe_with_python(app_handle, video_path).await
}

async fn transcribe_with_python(app_handle: tauri::AppHandle, video_path: &str) -> Result<Transcript> {
    use tauri_plugin_shell::ShellExt;

    let python_script = r#"
import sys
import json
import os

# Add local site-packages if needed
sys.path.insert(0, os.path.join(os.path.dirname(__file__), 'venv', 'Lib', 'site-packages'))

def main():
    video_path = sys.argv[1]

    try:
        from faster_whisper import WhisperModel
    except ImportError:
        print(json.dumps({"error": "faster-whisper not installed. Run: pip install -r requirements.txt"}))
        sys.exit(1)

    # Load model with int8 quantization, CPU offload
    model = WhisperModel(
        "large-v3",
        device="auto",
        compute_type="int8_float16",
        cpu_threads=4,
        num_workers=1,
        download_root=os.path.join(os.path.dirname(__file__), 'models')
    )

    segments, info = model.transcribe(
        video_path,
        beam_size=5,
        word_timestamps=True,
        vad_filter=True,
        vad_parameters=dict(min_silence_duration_ms=500)
    )

    result = {
        "language": info.language,
        "duration": info.duration,
        "segments": []
    }

    for segment in segments:
        seg_data = {
            "start": segment.start,
            "end": segment.end,
            "text": segment.text.strip(),
            "words": []
        }

        if segment.words:
            for word in segment.words:
                seg_data["words"].append({
                    "start": word.start,
                    "end": word.end,
                    "text": word.word.strip(),
                    "confidence": word.probability
                })

        result["segments"].append(seg_data)

    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#;

    // Write Python script to temp file
    let script_path = std::env::temp_dir().join("clipper_transcribe.py");
    std::fs::write(&script_path, python_script)?;

    let sidecar = app_handle.shell().sidecar("python")?;

    let output = sidecar
        .args([
            script_path.to_str().unwrap(),
            video_path,
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Transcription failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let transcript: Transcript = serde_json::from_str(&stdout)?;

    Ok(transcript)
}

pub fn extract_words(transcript: &Transcript) -> Vec<Word> {
    transcript.segments.iter()
        .flat_map(|s| s.words.clone())
        .collect()
}

pub fn find_silence_regions(transcript: &Transcript, min_silence: f64) -> Vec<(f64, f64)> {
    let mut regions = Vec::new();
    let words = extract_words(transcript);

    let mut last_end = 0.0;
    for word in words {
        if word.start - last_end > min_silence {
            regions.push((last_end, word.start));
        }
        last_end = word.end;
    }

    // Check end
    if transcript.duration - last_end > min_silence {
        regions.push((last_end, transcript.duration));
    }

    regions
}

pub fn calculate_pace(transcript: &Transcript) -> f64 {
    let words = extract_words(transcript);
    if words.is_empty() {
        return 0.0;
    }

    let total_words = words.len() as f64;
    let duration = transcript.duration;

    total_words / duration * 60.0 // words per minute
}

pub fn calculate_hook_density(transcript: &Transcript) -> f64 {
    let hook_keywords = [
        "you", "stop", "wait", "watch", "look", "listen",
        "secret", "trick", "hack", "mistake", "wrong",
        "never", "always", "must", "need", "shocking",
        "unbelievable", "incredible", "amazing", "crazy",
        "insane", "wild", "mind", "blown", "game changer",
    ];

    let first_3_seconds = transcript.segments.iter()
        .filter(|s| s.start < 3.0)
        .flat_map(|s| s.words.iter())
        .collect::<Vec<_>>();

    if first_3_seconds.is_empty() {
        return 0.0;
    }

    let hook_count = first_3_seconds.iter()
        .filter(|w| {
            let text = w.text.to_lowercase();
            hook_keywords.iter().any(|k| text.contains(k))
        })
        .count();

    hook_count as f64 / first_3_seconds.len() as f64
}