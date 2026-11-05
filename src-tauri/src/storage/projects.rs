use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::storage::db::DbPool;

fn map_err<T>(e: T) -> anyhow::Error
where
    T: std::fmt::Display,
{
    anyhow::anyhow!(e.to_string())
}

fn parse_rfc3339_in_row(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<DateTime<Utc>> {
    let s: String = row.get(idx)?;
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

// ============================================================
// DATABASE TYPES
// ============================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: String,
    pub source_url: String,
    pub source_path: Option<String>,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub status: String,
}


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
    pub thumbnail_path: Option<String>,
    pub status: String,
    pub timeline_data: Option<String>,
    pub hook_segments: Option<String>,
    pub montage_order: Option<i64>,
    pub platform_exports: Option<String>,
    pub created_at: DateTime<Utc>,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Thumbnail {
    pub id: String,
    pub clip_id: String,
    pub template_id: Option<String>,
    pub image_path: String,
    pub prompt: Option<String>,
    pub is_selected: bool,
    pub created_at: DateTime<Utc>,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ViralPrediction {
    pub id: String,
    pub clip_id: String,
    pub platform: String,
    pub predicted_views: Option<i64>,
    pub predicted_likes: Option<i64>,
    pub predicted_shares: Option<i64>,
    pub predicted_retention: Option<f64>,
    pub viral_score: f64,
    pub hook_analysis: String,
    pub retention_curve: Option<String>,
    pub recommended_hooks: Option<String>,
    pub optimization_tips: Option<String>,
    pub model_used: String,
    pub created_at: DateTime<Utc>,
}


// Internal functions (not Tauri commands)
pub fn create_project_internal(conn: &Connection, project: &Project) -> Result<()> {
    conn.execute(
        "INSERT INTO projects (id, source_url, source_path, title, created_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![project.id, project.source_url, project.source_path, project.title, project.created_at.to_rfc3339(), project.status],
    ).map_err(map_err)?;
    Ok(())
}


pub fn get_projects_internal(conn: &Connection) -> Result<Vec<Project>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_url, source_path, title, created_at, status FROM projects ORDER BY created_at DESC"
    ).map_err(map_err)?;
    
    let projects = stmt.query_map([], |row| {
        Ok(Project {
            id: row.get(0)?,
            source_url: row.get(1)?,
            source_path: row.get(2)?,
            title: row.get(3)?,
            created_at: parse_rfc3339_in_row(row, 4)?,
            status: row.get(5)?,
        })
    }).map_err(map_err)?;

    Ok(projects.filter_map(Result::ok).collect())
}


pub fn get_project_internal(conn: &Connection, project_id: &str) -> Result<Option<Project>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_url, source_path, title, created_at, status FROM projects WHERE id = ?1"
    ).map_err(map_err)?;
    
    let project = stmt.query_row(params![project_id], |row| {
        Ok(Project {
            id: row.get(0)?,
            source_url: row.get(1)?,
            source_path: row.get(2)?,
            title: row.get(3)?,
            created_at: parse_rfc3339_in_row(row, 4)?,
            status: row.get(5)?,
        })
    }).optional().map_err(map_err)?;
    
    Ok(project)
}


pub fn update_project_status_internal(conn: &Connection, project_id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE projects SET status = ?1 WHERE id = ?2",
        params![status, project_id],
    ).map_err(map_err)?;
    Ok(())
}


pub fn update_project_source_path_internal(conn: &Connection, project_id: &str, source_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE projects SET source_path = ?1 WHERE id = ?2",
        params![source_path, project_id],
    ).map_err(map_err)?;
    Ok(())
}


// ============================================================
// CLIP OPERATIONS (internal)
// ============================================================

pub fn create_clip_internal(conn: &Connection, clip: &Clip) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO clips (
            id, project_id, start_s, end_s, duration_s, title,
            hook_score, viral_score, heuristic_json, llm_reason,
            template_id, render_path, thumbnail_path, status,
            timeline_data, hook_segments, montage_order, platform_exports, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
        "#,
        params![
            clip.id, clip.project_id, clip.start_s, clip.end_s, clip.duration_s, clip.title,
            clip.hook_score, clip.viral_score, clip.heuristic_json, clip.llm_reason,
            clip.template_id, clip.render_path, clip.thumbnail_path, clip.status,
            clip.timeline_data, clip.hook_segments, clip.montage_order, clip.platform_exports, clip.created_at.to_rfc3339()
        ],
    ).map_err(map_err)?;
    Ok(())
}


pub fn get_clips_internal(conn: &Connection, project_id: &str) -> Result<Vec<Clip>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, project_id, start_s, end_s, duration_s, title,
               hook_score, viral_score, heuristic_json, llm_reason,
               template_id, render_path, thumbnail_path, status,
               timeline_data, hook_segments, montage_order, platform_exports, created_at
        FROM clips WHERE project_id = ?1 ORDER BY start_s
        "#
    ).map_err(map_err)?;
    
    let clips = stmt.query_map(params![project_id], |row| {
        Ok(Clip {
            id: row.get(0)?,
            project_id: row.get(1)?,
            start_s: row.get(2)?,
            end_s: row.get(3)?,
            duration_s: row.get(4)?,
            title: row.get(5)?,
            hook_score: row.get(6)?,
            viral_score: row.get(7)?,
            heuristic_json: row.get(8)?,
            llm_reason: row.get(9)?,
            template_id: row.get(10)?,
            render_path: row.get(11)?,
            thumbnail_path: row.get(12)?,
            status: row.get(13)?,
            timeline_data: row.get(14)?,
            hook_segments: row.get(15)?,
            montage_order: row.get(16)?,
            platform_exports: row.get(17)?,
            created_at: parse_rfc3339_in_row(row, 18)?,
        })
    }).map_err(map_err)?;

    Ok(clips.filter_map(Result::ok).collect())
}


pub fn get_clip_internal(conn: &Connection, clip_id: &str) -> Result<Option<Clip>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, project_id, start_s, end_s, duration_s, title,
               hook_score, viral_score, heuristic_json, llm_reason,
               template_id, render_path, thumbnail_path, status,
               timeline_data, hook_segments, montage_order, platform_exports, created_at
        FROM clips WHERE id = ?1
        "#
    ).map_err(map_err)?;

    let clip = stmt.query_row(params![clip_id], |row| {
        Ok(Clip {
            id: row.get(0)?,
            project_id: row.get(1)?,
            start_s: row.get(2)?,
            end_s: row.get(3)?,
            duration_s: row.get(4)?,
            title: row.get(5)?,
            hook_score: row.get(6)?,
            viral_score: row.get(7)?,
            heuristic_json: row.get(8)?,
            llm_reason: row.get(9)?,
            template_id: row.get(10)?,
            render_path: row.get(11)?,
            thumbnail_path: row.get(12)?,
            status: row.get(13)?,
            timeline_data: row.get(14)?,
            hook_segments: row.get(15)?,
            montage_order: row.get(16)?,
            platform_exports: row.get(17)?,
            created_at: parse_rfc3339_in_row(row, 18)?,
        })
    }).optional().map_err(map_err)?;

    Ok(clip)
}


pub fn update_clip_timeline_internal(conn: &Connection, clip_id: &str, timeline_data: &str) -> Result<()> {
    conn.execute(
        "UPDATE clips SET timeline_data = ?1 WHERE id = ?2",
        params![timeline_data, clip_id],
    ).map_err(map_err)?;
    Ok(())
}


pub fn update_clip_hooks_internal(conn: &Connection, clip_id: &str, hook_segments: &str) -> Result<()> {
    conn.execute(
        "UPDATE clips SET hook_segments = ?1 WHERE id = ?2",
        params![hook_segments, clip_id],
    ).map_err(map_err)?;
    Ok(())
}


pub fn update_clip_montage_order_internal(conn: &Connection, clip_id: &str, order: i64) -> Result<()> {
    conn.execute(
        "UPDATE clips SET montage_order = ?1 WHERE id = ?2",
        params![order, clip_id],
    ).map_err(map_err)?;
    Ok(())
}


pub fn update_clip_render_path_internal(conn: &Connection, clip_id: &str, render_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE clips SET render_path = ?1, status = 'completed' WHERE id = ?2",
        params![render_path, clip_id],
    ).map_err(map_err)?;
    Ok(())
}


pub fn update_clip_thumbnail_internal(conn: &Connection, clip_id: &str, thumbnail_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE clips SET thumbnail_path = ?1 WHERE id = ?2",
        params![thumbnail_path, clip_id],
    ).map_err(map_err)?;
    Ok(())
}


pub fn update_clip_status_internal(conn: &Connection, clip_id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE clips SET status = ?1 WHERE id = ?2",
        params![status, clip_id],
    ).map_err(map_err)?;
    Ok(())
}


pub fn update_clip_platform_exports_internal(conn: &Connection, clip_id: &str, exports: &str) -> Result<()> {
    conn.execute(
        "UPDATE clips SET platform_exports = ?1 WHERE id = ?2",
        params![exports, clip_id],
    ).map_err(map_err)?;
    Ok(())
}



#[tauri::command]
pub fn get_projects(pool: State<'_, DbPool>) -> Result<Vec<Project>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_projects_internal(&conn).map_err(|e| e.to_string())
}


#[tauri::command]
pub fn get_clips(pool: State<'_, DbPool>, project_id: String) -> Result<Vec<Clip>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_clips_internal(&conn, &project_id).map_err(|e| e.to_string())
}


#[tauri::command]
pub fn create_thumbnail(pool: State<'_, DbPool>, thumbnail: Thumbnail) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    create_thumbnail_internal(&conn, &thumbnail).map_err(|e| e.to_string())
}


#[tauri::command]
pub fn get_thumbnails(pool: State<'_, DbPool>, clip_id: String) -> Result<Vec<Thumbnail>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_thumbnails_internal(&conn, &clip_id).map_err(|e| e.to_string())
}


#[tauri::command]
pub fn select_thumbnail(pool: State<'_, DbPool>, clip_id: String, thumbnail_id: String) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    select_thumbnail_internal(&conn, &clip_id, &thumbnail_id).map_err(|e| e.to_string())
}


#[tauri::command]
pub fn create_viral_prediction(pool: State<'_, DbPool>, pred: ViralPrediction) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    create_viral_prediction_internal(&conn, &pred).map_err(|e| e.to_string())
}


#[tauri::command]
pub fn get_viral_predictions(pool: State<'_, DbPool>, clip_id: String) -> Result<Vec<ViralPrediction>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_viral_predictions_internal(&conn, &clip_id).map_err(|e| e.to_string())
}


// ============================================================
// INTERNAL THUMBNAIL OPERATIONS
// ============================================================

pub fn create_thumbnail_internal(conn: &Connection, thumbnail: &Thumbnail) -> Result<()> {
    conn.execute(
        "INSERT INTO thumbnails (id, clip_id, template_id, image_path, prompt, is_selected, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![thumbnail.id, thumbnail.clip_id, thumbnail.template_id, thumbnail.image_path, thumbnail.prompt, thumbnail.is_selected as i32, thumbnail.created_at.to_rfc3339()],
    ).map_err(map_err)?;
    Ok(())
}


pub fn get_thumbnails_internal(conn: &Connection, clip_id: &str) -> Result<Vec<Thumbnail>> {
    let mut stmt = conn.prepare(
        "SELECT id, clip_id, template_id, image_path, prompt, is_selected, created_at FROM thumbnails WHERE clip_id = ?1 ORDER BY created_at DESC"
    ).map_err(map_err)?;
    
    let thumbnails = stmt.query_map(params![clip_id], |row| {
        Ok(Thumbnail {
            id: row.get(0)?,
            clip_id: row.get(1)?,
            template_id: row.get(2)?,
            image_path: row.get(3)?,
            prompt: row.get(4)?,
            is_selected: row.get::<_, i32>(5)? == 1,
            created_at: parse_rfc3339_in_row(row, 6)?,
        })
    }).map_err(map_err)?;

    Ok(thumbnails.filter_map(Result::ok).collect())
}


pub fn select_thumbnail_internal(conn: &Connection, clip_id: &str, thumbnail_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE thumbnails SET is_selected = 0 WHERE clip_id = ?1",
        params![clip_id],
    ).map_err(map_err)?;
    conn.execute(
        "UPDATE thumbnails SET is_selected = 1 WHERE id = ?1",
        params![thumbnail_id],
    ).map_err(map_err)?;
    Ok(())
}


// ============================================================
// INTERNAL VIRAL PREDICTION OPERATIONS
// ============================================================

pub fn create_viral_prediction_internal(conn: &Connection, pred: &ViralPrediction) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO viral_predictions (
            id, clip_id, platform, predicted_views, predicted_likes, predicted_shares,
            predicted_retention, viral_score, hook_analysis, retention_curve,
            recommended_hooks, optimization_tips, model_used, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
        params![
            pred.id, pred.clip_id, pred.platform, pred.predicted_views, pred.predicted_likes, pred.predicted_shares,
            pred.predicted_retention, pred.viral_score, pred.hook_analysis, pred.retention_curve,
            pred.recommended_hooks, pred.optimization_tips, pred.model_used, pred.created_at.to_rfc3339()
        ],
    ).map_err(map_err)?;
    Ok(())
}


pub fn get_viral_predictions_internal(conn: &Connection, clip_id: &str) -> Result<Vec<ViralPrediction>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, clip_id, platform, predicted_views, predicted_likes, predicted_shares,
               predicted_retention, viral_score, hook_analysis, retention_curve,
               recommended_hooks, optimization_tips, model_used, created_at
        FROM viral_predictions WHERE clip_id = ?1 ORDER BY created_at DESC
        "#
    ).map_err(map_err)?;
    
    let preds = stmt.query_map(params![clip_id], |row| {
        Ok(ViralPrediction {
            id: row.get(0)?,
            clip_id: row.get(1)?,
            platform: row.get(2)?,
            predicted_views: row.get(3)?,
            predicted_likes: row.get(4)?,
            predicted_shares: row.get(5)?,
            predicted_retention: row.get(6)?,
            viral_score: row.get(7)?,
            hook_analysis: row.get(8)?,
            retention_curve: row.get(9)?,
            recommended_hooks: row.get(10)?,
            optimization_tips: row.get(11)?,
            model_used: row.get(12)?,
            created_at: parse_rfc3339_in_row(row, 13)?,
        })
    }).map_err(map_err)?;

    Ok(preds.filter_map(Result::ok).collect())
}

