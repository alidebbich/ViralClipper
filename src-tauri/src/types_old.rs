// Common types shared across all modules
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Clip {
    pub id: String,
    pub project_id: String,
    pub start_s: f64,
    pub end_s: f64,
    pub duration_s: f64,
    pub title: String,
    pub hook_score: Option<f64>,
    pub viral_score: Option<f64>,
    pub heuristic_json: Option<String>,
    pub llm_reason: Option<String>,
    pub template_id: Option<String>,
    pub render_path: Option<String>,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub confidence: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Transcript {
    pub language: String,
    pub duration: f64,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeuristicScores {
    pub hook_density: f64,
    pub pace: f64,
    pub sentiment_arc: f64,
    pub self_containment: f64,
    pub audio_energy_variance: f64,
    pub duration_compliance: f64,
    pub total: f64,
}

impl Clip {
    pub fn new(
        project_id: String,
        start_s: f64,
        end_s: f64,
        title: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            project_id,
            start_s,
            end_s,
            duration_s: end_s - start_s,
            title,
            hook_score: None,
            viral_score: None,
            heuristic_json: None,
            llm_reason: None,
            template_id: None,
            render_path: None,
            status: "pending".to_string(),
        }
    }
}