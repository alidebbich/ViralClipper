use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct VoiceInfo {
    pub name: String,
    pub locale: String,
    pub gender: String,
    pub short_name: String,
    pub style_list: Option<Vec<String>>,
    pub sample_rate_hertz: Option<u32>,
    pub voice_type: String,
    pub status: String,
    pub words_per_minute: Option<String>,
}

pub async fn generate_tts(text: &str, voice: &str, output_path: &str) -> Result<()> {
    info!("Running edge-tts for voice: {}", voice);

    // edge-tts --voice <voice> --text <text> --write-media <output.mp3>
    let output = Command::new("edge-tts")
        .args([
            "--voice", voice,
            "--text", text,
            "--write-media", output_path
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("edge-tts failed: {}", stderr));
    }

    if !std::path::Path::new(output_path).exists() {
        return Err(anyhow::anyhow!("edge-tts did not create the output file"));
    }

    Ok(())
}

pub async fn list_voices() -> Result<Vec<String>> {
    let output = Command::new("edge-tts")
        .arg("--list-voices")
        .output()
        .await?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("Failed to list edge-tts voices"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut voices = Vec::new();

    for line in stdout.lines() {
        if line.contains("Name:") {
            let parts: Vec<&str> = line.split("Name:").collect();
            if parts.len() > 1 {
                let voice_name = parts[1].trim();
                voices.push(voice_name.to_string());
            }
        }
    }

    Ok(voices)
}

pub async fn list_voices_by_language(language: &str) -> Result<Vec<VoiceInfo>> {
    let output = Command::new("edge-tts")
        .arg("--list-voices")
        .output()
        .await?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("Failed to list edge-tts voices"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut voices = Vec::new();
    let mut current_voice: Option<VoiceInfo> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            if let Some(v) = current_voice.take() {
                voices.push(v);
            }
            continue;
        }

        if line.starts_with("Name:") {
            if let Some(v) = current_voice.take() {
                voices.push(v);
            }
            let parts: Vec<&str> = line.split("Name:").collect();
            if parts.len() > 1 {
                current_voice = Some(VoiceInfo {
                    name: parts[1].trim().to_string(),
                    locale: String::new(),
                    gender: String::new(),
                    short_name: String::new(),
                    style_list: None,
                    sample_rate_hertz: None,
                    voice_type: String::new(),
                    status: String::new(),
                    words_per_minute: None,
                });
            }
        } else if line.starts_with("Locale:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                v.locale = line.strip_prefix("Locale:").unwrap_or("").trim().to_string();
            }
        } else if line.starts_with("Gender:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                v.gender = line.strip_prefix("Gender:").unwrap_or("").trim().to_string();
            }
        } else if line.starts_with("ShortName:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                v.short_name = line.strip_prefix("ShortName:").unwrap_or("").trim().to_string();
            }
        } else if line.starts_with("StyleList:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                let styles = line.strip_prefix("StyleList:").unwrap_or("").trim();
                if !styles.is_empty() && styles != "[]" {
                    v.style_list = Some(styles.split(',').map(|s| s.trim().to_string()).collect());
                }
            }
        } else if line.starts_with("SampleRateHertz:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                v.sample_rate_hertz = line.strip_prefix("SampleRateHertz:").unwrap_or("").trim().parse().ok();
            }
        } else if line.starts_with("VoiceType:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                v.voice_type = line.strip_prefix("VoiceType:").unwrap_or("").trim().to_string();
            }
        } else if line.starts_with("Status:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                v.status = line.strip_prefix("Status:").unwrap_or("").trim().to_string();
            }
        } else if line.starts_with("WordsPerMinute:") && current_voice.is_some() {
            if let Some(v) = current_voice.as_mut() {
                v.words_per_minute = Some(line.strip_prefix("WordsPerMinute:").unwrap_or("").trim().to_string());
            }
        }
    }

    if let Some(v) = current_voice {
        voices.push(v);
    }

    // Filter by language
    let lang = language.to_lowercase();
    let filtered: Vec<VoiceInfo> = voices.into_iter()
        .filter(|v| {
            let locale = v.locale.to_lowercase();
            if lang == "auto" {
                true
            } else if lang == "ar" || lang == "arabic" {
                locale.starts_with("ar")
            } else if lang == "fr" || lang == "french" {
                locale.starts_with("fr")
            } else if lang == "en" || lang == "english" {
                locale.starts_with("en")
            } else if lang == "tn" || lang == "tunisian" {
                locale.starts_with("ar-tn") || v.short_name.contains("ar-TN")
            } else {
                locale.starts_with(&lang)
            }
        })
        .collect();

    Ok(filtered)
}
