use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

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
pub struct Asset {
    pub id: String,
    pub asset_type: String,
    pub path: String,
    pub tags: Option<String>,
    pub metadata_json: Option<String>,
    pub source_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub ass_style_json: String,
    pub filter_graph_json: String,
    pub theme_json: String,
    pub thumbnail_json: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserPreferences {
    pub id: String,
    pub key: String,
    pub value_json: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelConfig {
    pub id: String,
    pub provider: String,
    pub model_name: String,
    pub api_key: Option<String>,
    pub api_endpoint: Option<String>,
    pub is_default: bool,
    pub use_case: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SocialAccount {
    pub id: String,
    pub platform: String,
    pub account_name: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<String>,
    pub account_id: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScheduledPost {
    pub id: String,
    pub clip_id: String,
    pub social_account_id: String,
    pub platform: String,
    pub title: String,
    pub caption: String,
    pub hashtags: String,
    pub scheduled_at: String,
    pub status: String,
    pub posted_at: Option<String>,
    pub error_message: Option<String>,
    pub post_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub type DbPool = Arc<Mutex<Connection>>;

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

pub fn init(app_handle: AppHandle) -> Result<DbPool> {
    let app_dir = app_handle
        .path()
        .app_data_dir()
        .expect("Failed to get app data dir");

    std::fs::create_dir_all(&app_dir)?;
    let db_path = app_dir.join("clipper.db");
    
    let conn = Connection::open(&db_path).map_err(map_err)?;
    
    conn.execute("PRAGMA foreign_keys = ON", []).map_err(map_err)?;
    
    migrate(&conn)?;

    let pool = Arc::new(Mutex::new(conn));
    
    app_handle.manage(pool.clone());

    Ok(pool)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            source_url TEXT NOT NULL,
            source_path TEXT,
            title TEXT NOT NULL,
            created_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending'
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS clips (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            start_s REAL NOT NULL,
            end_s REAL NOT NULL,
            duration_s REAL NOT NULL,
            title TEXT NOT NULL,
            hook_score REAL,
            viral_score REAL,
            heuristic_json TEXT,
            llm_reason TEXT,
            template_id TEXT,
            render_path TEXT,
            thumbnail_path TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            timeline_data TEXT,
            hook_segments TEXT,
            montage_order INTEGER,
            platform_exports TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS assets (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL,
            path TEXT NOT NULL,
            tags TEXT,
            metadata_json TEXT,
            source_url TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS templates (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            ass_style_json TEXT NOT NULL,
            filter_graph_json TEXT NOT NULL,
            theme_json TEXT NOT NULL,
            thumbnail_json TEXT,
            category TEXT,
            description TEXT
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        "ALTER TABLE templates ADD COLUMN thumbnail_json TEXT",
        [],
    ).map_err(|_| anyhow::anyhow!("")).ok();

    conn.execute(
        "ALTER TABLE templates ADD COLUMN category TEXT",
        [],
    ).map_err(|_| anyhow::anyhow!("")).ok();

    conn.execute(
        "ALTER TABLE templates ADD COLUMN description TEXT",
        [],
    ).map_err(|_| anyhow::anyhow!("")).ok();

    // Migrate existing clips table columns if created by an older version
    let clips_migrations = [
        "ALTER TABLE clips ADD COLUMN thumbnail_path TEXT",
        "ALTER TABLE clips ADD COLUMN timeline_data TEXT",
        "ALTER TABLE clips ADD COLUMN hook_segments TEXT",
        "ALTER TABLE clips ADD COLUMN montage_order INTEGER",
        "ALTER TABLE clips ADD COLUMN platform_exports TEXT",
    ];
    for sql in clips_migrations {
        let _ = conn.execute(sql, []);
    }

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS prefs (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS social_accounts (
            id TEXT PRIMARY KEY,
            platform TEXT NOT NULL,
            account_name TEXT NOT NULL,
            access_token TEXT NOT NULL,
            refresh_token TEXT,
            token_expires_at TEXT,
            account_id TEXT NOT NULL,
            is_active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(platform, account_id)
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS scheduled_posts (
            id TEXT PRIMARY KEY,
            clip_id TEXT NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
            social_account_id TEXT NOT NULL REFERENCES social_accounts(id) ON DELETE CASCADE,
            platform TEXT NOT NULL,
            title TEXT NOT NULL,
            caption TEXT NOT NULL,
            hashtags TEXT NOT NULL,
            scheduled_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            posted_at TEXT,
            error_message TEXT,
            post_id TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS thumbnails (
            id TEXT PRIMARY KEY,
            clip_id TEXT NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
            template_id TEXT REFERENCES templates(id) ON DELETE SET NULL,
            image_path TEXT NOT NULL,
            prompt TEXT,
            is_selected INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS viral_predictions (
            id TEXT PRIMARY KEY,
            clip_id TEXT NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
            platform TEXT NOT NULL,
            predicted_views INTEGER,
            predicted_likes INTEGER,
            predicted_shares INTEGER,
            predicted_retention REAL,
            viral_score REAL NOT NULL,
            hook_analysis TEXT NOT NULL,
            retention_curve TEXT,
            recommended_hooks TEXT,
            optimization_tips TEXT,
            model_used TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS user_preferences (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL
        )
        "#,
        [],
    ).map_err(map_err)?;

    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS model_configs (
            id TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            model_name TEXT NOT NULL,
            api_key TEXT,
            api_endpoint TEXT,
            is_default INTEGER NOT NULL DEFAULT 0,
            use_case TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        [],
    ).map_err(map_err)?;

    insert_default_templates(conn)?;

    Ok(())
}

fn insert_default_templates(conn: &Connection) -> Result<()> {
    let templates = vec![
        (
            "tiktok",
            "TikTok Viral",
            "video",
            "Optimized for TikTok with pop-in captions, zoom punches, and progress bar",
            r##"{"font":"Montserrat Bold","font_size":72,"primary_color":"&HFFFFFF","outline_color":"&H000000","outline_width":3,"shadow_offset":2,"alignment":2,"margin_v":150,"karaoke_style":"pop_in"}"##,
            r##"{"zoom_punch":true,"jump_cuts":true,"face_crop":true,"music_duck":true,"broll_overlay":true,"sfx_layer":true,"progress_bar":true}"##,
            r##"{"primary":"#FF0050","secondary":"#00F2EA","background":"#000000","accent":"#FFF000"}"##,
            None,
        ),
        (
            "hormozi",
            "Hormozi Style",
            "video",
            "High-energy Hormozi style with yellow highlights and punch zoom",
            r##"{"font":"Anton","font_size":80,"primary_color":"&HFFFF00","outline_color":"&H000000","outline_width":4,"shadow_offset":3,"alignment":2,"margin_v":180,"karaoke_style":"highlight"}"##,
            r##"{"zoom_punch":true,"jump_cuts":true,"face_crop":true,"music_duck":true,"broll_overlay":false,"sfx_layer":true,"progress_bar":false}"##,
            r##"{"primary":"#FFFF00","secondary":"#FFFFFF","background":"#000000","accent":"#FF0000"}"##,
            None,
        ),
        (
            "minimal",
            "Minimal Clean",
            "video",
            "Clean minimal style with fade captions, no distractions",
            r##"{"font":"Inter Medium","font_size":56,"primary_color":"&HFFFFFF","outline_color":"&H00000000","outline_width":0,"shadow_offset":0,"alignment":2,"margin_v":120,"karaoke_style":"fade"}"##,
            r##"{"zoom_punch":false,"jump_cuts":true,"face_crop":true,"music_duck":true,"broll_overlay":false,"sfx_layer":false,"progress_bar":true}"##,
            r##"{"primary":"#FFFFFF","secondary":"#CCCCCC","background":"#00000000","accent":"#007AFF"}"##,
            None,
        ),
        (
            "instagram_reels",
            "Instagram Reels",
            "video",
            "Optimized for Instagram Reels with aesthetic captions",
            r##"{"font":"Playfair Display","font_size":64,"primary_color":"&HFFFFFF","outline_color":"&H000000","outline_width":2,"shadow_offset":2,"alignment":2,"margin_v":140,"karaoke_style":"fade"}"##,
            r##"{"zoom_punch":true,"jump_cuts":true,"face_crop":true,"music_duck":true,"broll_overlay":true,"sfx_layer":false,"progress_bar":false}"##,
            r##"{"primary":"#E1306C","secondary":"#F77737","background":"#000000","accent":"#FFFFFF"}"##,
            None,
        ),
        (
            "youtube_shorts",
            "YouTube Shorts",
            "video",
            "High-retention YouTube Shorts with progress bar and hook emphasis",
            r##"{"font":"Bebas Neue","font_size":76,"primary_color":"&HFFFFFF","outline_color":"&H000000","outline_width":3,"shadow_offset":3,"alignment":2,"margin_v":160,"karaoke_style":"pop_in"}"##,
            r##"{"zoom_punch":true,"jump_cuts":true,"face_crop":true,"music_duck":true,"broll_overlay":true,"sfx_layer":true,"progress_bar":true}"##,
            r##"{"primary":"#FF0000","secondary":"#FFFFFF","background":"#000000","accent":"#00FF00"}"##,
            None,
        ),
        (
            "hook_master",
            "Hook Master",
            "video",
            "Maximum hook retention with aggressive zoom punches and highlight captions",
            r##"{"font":"Impact","font_size":84,"primary_color":"&HFFFF00","outline_color":"&HFF0000","outline_width":5,"shadow_offset":4,"alignment":2,"margin_v":180,"karaoke_style":"highlight"}"##,
            r##"{"zoom_punch":true,"jump_cuts":true,"face_crop":true,"music_duck":true,"broll_overlay":true,"sfx_layer":true,"progress_bar":true}"##,
            r##"{"primary":"#FFFF00","secondary":"#FF0000","background":"#000000","accent":"#00FFFF"}"##,
            None,
        ),
        (
            "montage_epic",
            "Epic Montage",
            "video",
            "Fast-paced montage with transitions, perfect for highlights",
            r##"{"font":"Oswald Bold","font_size":68,"primary_color":"&HFFFFFF","outline_color":"&H000000","outline_width":3,"shadow_offset":2,"alignment":2,"margin_v":150,"karaoke_style":"fade"}"##,
            r##"{"zoom_punch":true,"jump_cuts":true,"face_crop":false,"music_duck":true,"broll_overlay":true,"sfx_layer":true,"progress_bar":false}"##,
            r##"{"primary":"#FF6B35","secondary":"#F7F7F7","background":"#1A1A2E","accent":"#00F5FF"}"##,
            None,
        ),
        (
            "thumb_clickbait",
            "Clickbait Thumbnail",
            "thumbnail",
            "High-CTR clickbait style with bold text and arrows",
            r##"{"font":"Impact","font_size":120,"primary_color":"&HFFFF00","outline_color":"&HFF0000","outline_width":6,"shadow_offset":8,"alignment":2,"margin_v":200,"karaoke_style":"none"}"##,
            r##"{"zoom_punch":false,"jump_cuts":false,"face_crop":false,"music_duck":false,"broll_overlay":false,"sfx_layer":false,"progress_bar":false}"##,
            r##"{"primary":"#FFFF00","secondary":"#FF0000","background":"#000000","accent":"#FFFFFF"}"##,
            Some(r##"{"layout":"split","text_position":"top","arrow":true,"emoji":true}"##),
        ),
        (
            "thumb_clean",
            "Clean Thumbnail",
            "thumbnail",
            "Professional clean thumbnail with minimal text",
            r##"{"font":"Inter Bold","font_size":96,"primary_color":"&HFFFFFF","outline_color":"&H000000","outline_width":4,"shadow_offset":4,"alignment":2,"margin_v":180,"karaoke_style":"none"}"##,
            r##"{"zoom_punch":false,"jump_cuts":false,"face_crop":false,"music_duck":false,"broll_overlay":false,"sfx_layer":false,"progress_bar":false}"##,
            r##"{"primary":"#FFFFFF","secondary":"#CCCCCC","background":"#000000","accent":"#007AFF"}"##,
            Some(r##"{"layout":"center","text_position":"bottom","arrow":false,"emoji":false}"##),
        ),
        (
            "thumb_aesthetic",
            "Aesthetic Thumbnail",
            "thumbnail",
            "Instagram-style aesthetic with gradient and soft text",
            r##"{"font":"Playfair Display","font_size":88,"primary_color":"&HFFFFFF","outline_color":"&H000000","outline_width":3,"shadow_offset":3,"alignment":2,"margin_v":160,"karaoke_style":"none"}"##,
            r##"{"zoom_punch":false,"jump_cuts":false,"face_crop":false,"music_duck":false,"broll_overlay":false,"sfx_layer":false,"progress_bar":false}"##,
            r##"{"primary":"#E1306C","secondary":"#F77737","background":"#1A1A2E","accent":"#FFFFFF"}"##,
            Some(r##"{"layout":"gradient","text_position":"center","arrow":false,"emoji":true}"##),
        ),
    ];

    for (id, name, cat, desc, ass, filter, theme, thumb) in templates {
        conn.execute(
            r#"
            INSERT OR REPLACE INTO templates (id, name, category, description, ass_style_json, filter_graph_json, theme_json, thumbnail_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![id, name, cat, desc, ass, filter, theme, thumb],
        ).map_err(map_err)?;
    }

    Ok(())
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


// ============================================================
// TAURI COMMANDS
// ============================================================

#[tauri::command]
pub fn reset_database(pool: State<'_, DbPool>) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    conn.execute_batch("
        DELETE FROM scheduled_posts;
        DELETE FROM thumbnails;
        DELETE FROM viral_predictions;
        DELETE FROM clips;
        DELETE FROM projects;
        DELETE FROM assets;
    ").map_err(|e| e.to_string())?;
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
pub fn get_all_assets(pool: State<'_, DbPool>, limit: usize) -> Result<Vec<Asset>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_all_assets_internal(&conn, limit).map_err(|e| e.to_string())
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

#[tauri::command]
pub fn add_social_account(pool: State<'_, DbPool>, account: SocialAccount) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    add_social_account_internal(&conn, &account).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_social_accounts(pool: State<'_, DbPool>) -> Result<Vec<SocialAccount>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_social_accounts_internal(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_social_account(pool: State<'_, DbPool>, account_id: String) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    delete_social_account_internal(&conn, &account_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn schedule_post(pool: State<'_, DbPool>, post: ScheduledPost) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    schedule_post_internal(&conn, &post).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_scheduled_posts(pool: State<'_, DbPool>) -> Result<Vec<ScheduledPost>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_scheduled_posts_internal(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_post_status(pool: State<'_, DbPool>, post_id: String, status: String, post_id_external: Option<String>, error: Option<String>) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    update_post_status_internal(&conn, &post_id, &status, post_id_external.as_deref(), error.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_model_configs(pool: State<'_, DbPool>) -> Result<Vec<ModelConfig>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_model_configs_internal(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_model_config(pool: State<'_, DbPool>, config: ModelConfig) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    add_model_config_internal(&conn, &config).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_default_model(pool: State<'_, DbPool>, use_case: String, config_id: String) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    set_default_model_internal(&conn, &use_case, &config_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_preference(pool: State<'_, DbPool>, key: String, value_json: String) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    set_preference_internal(&conn, &key, &value_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_preference(pool: State<'_, DbPool>, key: String) -> Result<Option<String>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_preference_internal(&conn, &key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_user_preference(pool: State<'_, DbPool>, pref: UserPreferences) -> Result<(), String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    set_user_preference_internal(&conn, &pref).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_user_preference(pool: State<'_, DbPool>, key: String) -> Result<Option<UserPreferences>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_user_preference_internal(&conn, &key).map_err(|e| e.to_string())
}

// ============================================================
// INTERNAL TEMPLATE OPERATIONS
// ============================================================

pub fn list_templates_internal(conn: &Connection) -> Result<Vec<Template>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, ass_style_json, filter_graph_json, theme_json, thumbnail_json, category, description FROM templates"
    ).map_err(map_err)?;
    
    let templates = stmt.query_map([], |row| {
        Ok(Template {
            id: row.get(0)?,
            name: row.get(1)?,
            ass_style_json: row.get(2)?,
            filter_graph_json: row.get(3)?,
            theme_json: row.get(4)?,
            thumbnail_json: row.get(5)?,
            category: row.get(6)?,
            description: row.get(7)?,
        })
    }).map_err(map_err)?;

    Ok(templates.filter_map(Result::ok).collect())
}

pub fn get_template_internal(conn: &Connection, template_id: &str) -> Result<Option<Template>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, ass_style_json, filter_graph_json, theme_json, thumbnail_json, category, description FROM templates WHERE id = ?1"
    ).map_err(map_err)?;
    
    let template = stmt.query_row(params![template_id], |row| {
        Ok(Template {
            id: row.get(0)?,
            name: row.get(1)?,
            ass_style_json: row.get(2)?,
            filter_graph_json: row.get(3)?,
            theme_json: row.get(4)?,
            thumbnail_json: row.get(5)?,
            category: row.get(6)?,
            description: row.get(7)?,
        })
    }).optional().map_err(map_err)?;
    
    Ok(template)
}

pub fn create_template_internal(conn: &Connection, template: &Template) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO templates (id, name, ass_style_json, filter_graph_json, theme_json, thumbnail_json, category, description)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![template.id, template.name, template.ass_style_json, template.filter_graph_json, template.theme_json, template.thumbnail_json, template.category, template.description],
    ).map_err(map_err)?;
    Ok(())
}

pub fn delete_template_internal(conn: &Connection, template_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM templates WHERE id = ?1",
        params![template_id],
    ).map_err(map_err)?;
    Ok(())
}

// ============================================================
// INTERNAL ASSET OPERATIONS
// ============================================================

pub fn import_asset_internal(conn: &Connection, asset: &Asset) -> Result<()> {
    conn.execute(
        "INSERT INTO assets (id, type, path, tags, metadata_json, source_url, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![asset.id, asset.asset_type, asset.path, asset.tags, asset.metadata_json, asset.source_url, asset.created_at.to_rfc3339()],
    ).map_err(map_err)?;
    Ok(())
}

pub fn search_assets_internal(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT id, type, path, tags, metadata_json, source_url, created_at FROM assets WHERE type LIKE ?1 OR tags LIKE ?1 OR path LIKE ?1 ORDER BY created_at DESC LIMIT ?2"
    ).map_err(map_err)?;
    
    let search_term = format!("%{}%", query);
    let assets = stmt.query_map(params![search_term, limit], |row| {
        Ok(Asset {
            id: row.get(0)?,
            asset_type: row.get(1)?,
            path: row.get(2)?,
            tags: row.get(3)?,
            metadata_json: row.get(4)?,
            source_url: row.get(5)?,
            created_at: parse_rfc3339_in_row(row, 6)?,
        })
    }).map_err(map_err)?;

    Ok(assets.filter_map(Result::ok).collect())
}

pub fn get_all_assets_internal(conn: &Connection, limit: usize) -> Result<Vec<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT id, type, path, tags, metadata_json, source_url, created_at FROM assets ORDER BY created_at DESC LIMIT ?1"
    ).map_err(map_err)?;
    
    let assets = stmt.query_map(params![limit], |row| {
        Ok(Asset {
            id: row.get(0)?,
            asset_type: row.get(1)?,
            path: row.get(2)?,
            tags: row.get(3)?,
            metadata_json: row.get(4)?,
            source_url: row.get(5)?,
            created_at: parse_rfc3339_in_row(row, 6)?,
        })
    }).map_err(map_err)?;

    Ok(assets.filter_map(Result::ok).collect())
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

// ============================================================
// INTERNAL SOCIAL ACCOUNT OPERATIONS
// ============================================================

pub fn add_social_account_internal(conn: &Connection, account: &SocialAccount) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO social_accounts (id, platform, account_name, access_token, refresh_token, token_expires_at, account_id, is_active, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![account.id, account.platform, account.account_name, account.access_token, account.refresh_token, account.token_expires_at, account.account_id, account.is_active as i32, account.created_at.to_rfc3339(), account.updated_at.to_rfc3339()],
    ).map_err(map_err)?;
    Ok(())
}

pub fn get_social_accounts_internal(conn: &Connection) -> Result<Vec<SocialAccount>> {
    let mut stmt = conn.prepare(
        "SELECT id, platform, account_name, access_token, refresh_token, token_expires_at, account_id, is_active, created_at, updated_at FROM social_accounts WHERE is_active = 1 ORDER BY created_at DESC"
    ).map_err(map_err)?;
    
    let accounts = stmt.query_map([], |row| {
        Ok(SocialAccount {
            id: row.get(0)?,
            platform: row.get(1)?,
            account_name: row.get(2)?,
            access_token: row.get(3)?,
            refresh_token: row.get(4)?,
            token_expires_at: row.get(5)?,
            account_id: row.get(6)?,
            is_active: row.get::<_, i32>(7)? == 1,
            created_at: parse_rfc3339_in_row(row, 8)?,
            updated_at: parse_rfc3339_in_row(row, 9)?,
        })
    }).map_err(map_err)?;

    Ok(accounts.filter_map(Result::ok).collect())
}

pub fn delete_social_account_internal(conn: &Connection, account_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE social_accounts SET is_active = 0 WHERE id = ?1",
        params![account_id],
    ).map_err(map_err)?;
    Ok(())
}

// ============================================================
// INTERNAL SCHEDULED POST OPERATIONS
// ============================================================

pub fn schedule_post_internal(conn: &Connection, post: &ScheduledPost) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO scheduled_posts (id, clip_id, social_account_id, platform, title, caption, hashtags, scheduled_at, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![post.id, post.clip_id, post.social_account_id, post.platform, post.title, post.caption, post.hashtags, post.scheduled_at, post.status, post.created_at.to_rfc3339()],
    ).map_err(map_err)?;
    Ok(())
}

pub fn get_scheduled_posts_internal(conn: &Connection) -> Result<Vec<ScheduledPost>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, clip_id, social_account_id, platform, title, caption, hashtags, scheduled_at, status, posted_at, error_message, post_id, created_at
        FROM scheduled_posts ORDER BY scheduled_at ASC
        "#
    ).map_err(map_err)?;
    
    let posts = stmt.query_map([], |row| {
        Ok(ScheduledPost {
            id: row.get(0)?,
            clip_id: row.get(1)?,
            social_account_id: row.get(2)?,
            platform: row.get(3)?,
            title: row.get(4)?,
            caption: row.get(5)?,
            hashtags: row.get(6)?,
            scheduled_at: row.get(7)?,
            status: row.get(8)?,
            posted_at: row.get(9)?,
            error_message: row.get(10)?,
            post_id: row.get(11)?,
            created_at: parse_rfc3339_in_row(row, 12)?,
        })
    }).map_err(map_err)?;

    Ok(posts.filter_map(Result::ok).collect())
}

pub fn update_post_status_internal(conn: &Connection, post_id: &str, status: &str, post_id_external: Option<&str>, error: Option<&str>) -> Result<()> {
    if let Some(ext_id) = post_id_external {
        conn.execute(
            "UPDATE scheduled_posts SET status = ?1, post_id = ?2, error_message = ?3, posted_at = datetime('now') WHERE id = ?4",
            params![status, ext_id, error, post_id],
        ).map_err(map_err)?;
    } else {
        conn.execute(
            "UPDATE scheduled_posts SET status = ?1, error_message = ?2 WHERE id = ?3",
            params![status, error, post_id],
        ).map_err(map_err)?;
    }
    Ok(())
}

// ============================================================
// INTERNAL MODEL CONFIG OPERATIONS
// ============================================================

pub fn get_model_configs_internal(conn: &Connection) -> Result<Vec<ModelConfig>> {
    let mut stmt = conn.prepare(
        "SELECT id, provider, model_name, api_key, api_endpoint, is_default, use_case, created_at FROM model_configs ORDER BY created_at DESC"
    ).map_err(map_err)?;
    
    let configs = stmt.query_map([], |row| {
        Ok(ModelConfig {
            id: row.get(0)?,
            provider: row.get(1)?,
            model_name: row.get(2)?,
            api_key: row.get(3)?,
            api_endpoint: row.get(4)?,
            is_default: row.get::<_, i32>(5)? == 1,
            use_case: row.get(6)?,
            created_at: parse_rfc3339_in_row(row, 7)?,
        })
    }).map_err(map_err)?;

    Ok(configs.filter_map(Result::ok).collect())
}

pub fn add_model_config_internal(conn: &Connection, config: &ModelConfig) -> Result<()> {
    conn.execute(
        "INSERT INTO model_configs (id, provider, model_name, api_key, api_endpoint, is_default, use_case, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![config.id, config.provider, config.model_name, config.api_key, config.api_endpoint, config.is_default as i32, config.use_case, config.created_at.to_rfc3339()],
    ).map_err(map_err)?;
    Ok(())
}

pub fn set_default_model_internal(conn: &Connection, use_case: &str, config_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE model_configs SET is_default = 0 WHERE use_case = ?1",
        params![use_case],
    ).map_err(map_err)?;
    conn.execute(
        "UPDATE model_configs SET is_default = 1 WHERE id = ?1",
        params![config_id],
    ).map_err(map_err)?;
    Ok(())
}

// ============================================================
// INTERNAL PREFERENCE OPERATIONS
// ============================================================

pub fn set_preference_internal(conn: &Connection, key: &str, value_json: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO prefs (key, value_json) VALUES (?1, ?2)",
        params![key, value_json],
    ).map_err(map_err)?;
    Ok(())
}

pub fn get_preference_internal(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value_json FROM prefs WHERE key = ?1").map_err(map_err)?;
    let value = stmt.query_row(params![key], |row| row.get(0)).optional().map_err(map_err)?;
    Ok(value)
}

pub fn set_user_preference_internal(conn: &Connection, pref: &UserPreferences) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO user_preferences (id, key, value_json) VALUES (?1, ?2, ?3)",
        params![pref.id, pref.key, pref.value_json],
    ).map_err(map_err)?;
    Ok(())
}

pub fn get_user_preference_internal(conn: &Connection, key: &str) -> Result<Option<UserPreferences>> {
    let mut stmt = conn.prepare("SELECT id, key, value_json FROM user_preferences WHERE key = ?1").map_err(map_err)?;
    let pref = stmt.query_row(params![key], |row| {
        Ok(UserPreferences {
            id: row.get(0)?,
            key: row.get(1)?,
            value_json: row.get(2)?,
        })
    }).optional().map_err(map_err)?;
    Ok(pref)
}