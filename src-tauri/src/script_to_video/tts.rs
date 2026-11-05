use anyhow::Result;

// Compatibility shim: re-export edge_tts provider until Step 7 shim removal
pub use crate::providers::edge_tts::*;

pub async fn parse_script(raw_script: &str, _language: &str) -> Result<Vec<crate::script_to_video::Scene>> {
    // Parse markdown script into scenes
    // Handles **SPEAKER:** labels, [music cues], etc.
    use regex::Regex;
    
    let mut scenes = Vec::new();
    let lines: Vec<&str> = raw_script.lines().collect();
    let mut current_scene_text = String::new();
    let mut current_speaker = "NARRATOR".to_string();
    
    // Regex patterns
    let speaker_re = Regex::new(r"^\*\*(.+?):\*\*\s*(.*)$").unwrap();
    let music_cue_re = Regex::new(r"^\[(.+)\]$").unwrap();
    let heading_re = Regex::new(r"^#{1,3}\s+(.+)$").unwrap();

    for line in lines {
        let line = line.trim();
        
        // Skip empty lines
        if line.is_empty() {
            continue;
        }

        // Check for speaker label
        if let Some(caps) = speaker_re.captures(line) {
            // Save previous scene if any
            if !current_scene_text.trim().is_empty() {
                scenes.push(crate::script_to_video::Scene {
                    text: current_scene_text.trim().to_string(),
                    duration_hint: None,
                    visual_keywords: vec![],
                    visual_type: "stock_video".to_string(),
                    camera: "static".to_string(),
                    mood: "neutral".to_string(),
                    pacing_role: "build".to_string(),
                    sfx_hint: String::new(),
                });
            }
            
            current_speaker = caps[1].trim().to_string();
            current_scene_text = format!("{}: {}", current_speaker, caps[2].trim());
            continue;
        }

        // Check for music/sfx cue
        if let Some(caps) = music_cue_re.captures(line) {
            // Add as a separate scene with text_card type
            if !current_scene_text.trim().is_empty() {
                scenes.push(crate::script_to_video::Scene {
                    text: current_scene_text.trim().to_string(),
                    duration_hint: None,
                    visual_keywords: vec![],
                    visual_type: "stock_video".to_string(),
                    camera: "static".to_string(),
                    mood: "neutral".to_string(),
                    pacing_role: "build".to_string(),
                    sfx_hint: String::new(),
                });
            }
            
            let cue = caps[1].trim().to_string();
            scenes.push(crate::script_to_video::Scene {
                text: format!("[{}]", cue),
                duration_hint: Some(2.0),
                visual_keywords: vec!["music".to_string(), "atmosphere".to_string()],
                visual_type: "text_card".to_string(),
                camera: "static".to_string(),
                mood: "atmospheric".to_string(),
                pacing_role: "build".to_string(),
                sfx_hint: cue.to_lowercase().replace(' ', "_"),
            });
            
            current_scene_text = String::new();
            continue;
        }

        // Check for headings (treat as scene breaks)
        if let Some(caps) = heading_re.captures(line) {
            if !current_scene_text.trim().is_empty() {
                scenes.push(crate::script_to_video::Scene {
                    text: current_scene_text.trim().to_string(),
                    duration_hint: None,
                    visual_keywords: vec![],
                    visual_type: "stock_video".to_string(),
                    camera: "static".to_string(),
                    mood: "neutral".to_string(),
                    pacing_role: "build".to_string(),
                    sfx_hint: String::new(),
                });
            }
            
            current_scene_text = format!("{}: {}", current_speaker, caps[1].trim());
            continue;
        }

        // Regular text
        if !current_scene_text.is_empty() {
            current_scene_text.push(' ');
        }
        current_scene_text.push_str(line);
    }

    // Don't forget the last scene
    if !current_scene_text.trim().is_empty() {
        scenes.push(crate::script_to_video::Scene {
            text: current_scene_text.trim().to_string(),
            duration_hint: None,
            visual_keywords: vec![],
            visual_type: "stock_video".to_string(),
            camera: "static".to_string(),
            mood: "neutral".to_string(),
            pacing_role: "build".to_string(),
            sfx_hint: String::new(),
        });
    }

    // Post-process: estimate durations, detect language, tag first scene as hook
    for (i, scene) in scenes.iter_mut().enumerate() {
        // Estimate duration based on word count (average 150 words/min = 2.5 words/sec)
        let word_count = scene.text.split_whitespace().count();
        let estimated_duration = (word_count as f64 / 2.5).max(3.0).min(8.0);
        scene.duration_hint = Some(estimated_duration);
        
        // Tag first scene as hook
        if i == 0 {
            scene.pacing_role = "hook".to_string();
            scene.camera = "push_in".to_string();
            scene.mood = "intense".to_string();
        }
    }

    Ok(scenes)
}
