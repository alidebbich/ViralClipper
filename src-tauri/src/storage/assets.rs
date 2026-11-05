use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
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
pub struct Asset {
    pub id: String,
    pub asset_type: String,
    pub path: String,
    pub tags: Option<String>,
    pub metadata_json: Option<String>,
    pub source_url: Option<String>,
    pub created_at: DateTime<Utc>,
}


#[tauri::command]
pub fn get_all_assets(pool: State<'_, DbPool>, limit: usize) -> Result<Vec<Asset>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    get_all_assets_internal(&conn, limit).map_err(|e| e.to_string())
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

