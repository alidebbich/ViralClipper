// Clipper - Local-first viral clipper for Shorts
// Rust sidecar for Tauri v2

pub mod database;
pub mod pipeline;

// Re-exports
pub use database::{Database, Project, Clip, Asset, Template, Preference};
pub use pipeline::{PipelineRunner, PipelineConfig, ProjectInput, PipelineProgress, ClipResult};