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
        "INSERT OR REPLACE INTO user_preferences (key, value_json) VALUES (?1, ?2)",
        params![pref.key, pref.value_json],
    ).map_err(map_err)?;
    Ok(())
}


pub fn get_user_preference_internal(conn: &Connection, key: &str) -> Result<Option<UserPreferences>> {
    let mut stmt = conn.prepare("SELECT key, value_json FROM user_preferences WHERE key = ?1").map_err(map_err)?;
    let pref = stmt.query_row(params![key], |row| {
        let k: String = row.get(0)?;
        Ok(UserPreferences {
            id: k.clone(), // use key as id for frontend compat
            key: k,
            value_json: row.get(1)?,
        })
    }).optional().map_err(map_err)?;
    Ok(pref)
}
