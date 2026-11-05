use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

pub type DbPool = Arc<Mutex<Connection>>;

fn map_err<T>(e: T) -> anyhow::Error
where
    T: std::fmt::Display,
{
    anyhow::anyhow!(e.to_string())
}

#[allow(dead_code)]
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
    // Remove obsolete template IDs. neon_gaming/arabic_classic are real built-ins now.
    let _ = conn.execute(
        "DELETE FROM templates WHERE id IN ('instagram_reels', 'youtube_shorts', 'thumb_clickbait', 'thumb_clean', 'thumb_aesthetic', 'hook_master', 'montage_epic', 'clean_minimal', 'bold_pop')",
        [],
    );

    for b in crate::templates::builtin_templates() {
        let ass = serde_json::to_string(&b.ass).map_err(map_err)?;
        let filter = serde_json::to_string(&b.filter).map_err(map_err)?;
        let theme = serde_json::to_string(&b.theme).map_err(map_err)?;
        conn.execute(
            r#"
            INSERT OR REPLACE INTO templates (id, name, category, description, ass_style_json, filter_graph_json, theme_json, thumbnail_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
            "#,
            params![b.id, b.name, b.category, b.description, ass, filter, theme],
        ).map_err(map_err)?;
    }

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

