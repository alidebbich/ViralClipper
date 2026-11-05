use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Scene {
    pub text: String,
    pub duration_hint: Option<f64>,
    pub visual_keywords: Vec<String>,
    pub visual_type: String, // stock_video | stock_image | text_card | imported
    pub camera: String,      // push_in | pan | static
    pub mood: String,
    #[serde(default)]
    pub pacing_role: String, // hook | build | peak | payoff | cta
    #[serde(default)]
    pub sfx_hint: String,    // editor-facing sound hint
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScriptToVideoPlan {
    pub scenes: Vec<Scene>,
    pub language: String,
    #[serde(default)]
    pub voice: String,       // edge-tts voice name selected by user
    #[serde(default)]
    pub title: Option<String>, // used for thumbnail
}

pub mod tts;
pub mod visuals;
pub mod generator;
pub mod pipeline;
pub mod commands;
