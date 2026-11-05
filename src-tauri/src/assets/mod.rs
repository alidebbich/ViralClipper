use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::storage::DbPool;
use tauri::Manager;
use uuid::Uuid;
use rusqlite::OptionalExtension;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Asset {
    pub id: String,
    pub asset_type: String, // "broll", "music", "sfx", "template"
    pub path: String,
    pub tags: Option<String>, // JSON array of tags
    pub metadata_json: Option<String>,
    pub source_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AssetSearchQuery {
    pub asset_type: Option<String>,
    pub tags: Option<Vec<String>>,
    pub limit: Option<usize>,
}

#[tauri::command]
pub async fn search_assets(
    pool: tauri::State<'_, DbPool>,
    query: AssetSearchQuery,
) -> Result<Vec<Asset>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    
    let mut sql = "SELECT id, type, path, tags, metadata_json, source_url FROM assets WHERE 1=1".to_string();
    let mut args: Vec<String> = Vec::new();
    
    if let Some(asset_type) = &query.asset_type {
        sql.push_str(" AND type = ?");
        args.push(asset_type.clone());
    }
    
    if let Some(tags) = &query.tags {
        for tag in tags {
            sql.push_str(" AND tags LIKE ?");
            args.push(format!("%{}%", tag));
        }
    }
    
    sql.push_str(" ORDER BY created_at DESC");
    
    if let Some(limit) = query.limit {
        sql.push_str(&format!(" LIMIT {}", limit));
    }
    
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    
    let assets = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok(Asset {
                id: row.get(0)?,
                asset_type: row.get(1)?,
                path: row.get(2)?,
                tags: row.get(3)?,
                metadata_json: row.get(4)?,
                source_url: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    
    Ok(assets)
}

#[tauri::command]
pub async fn import_asset(
    app_handle: tauri::AppHandle,
    pool: tauri::State<'_, DbPool>,
    asset_type: String,
    source_path: String,
    tags: Option<Vec<String>>,
    metadata: Option<serde_json::Value>,
) -> Result<Asset, String> {
    use std::path::Path;
    
    let assets_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("assets")
        .join(&asset_type);
    
    std::fs::create_dir_all(&assets_dir).map_err(|e| e.to_string())?;
    
    let file_name = Path::new(&source_path)
        .file_name()
        .ok_or("Invalid source path")?
        .to_string_lossy()
        .to_string();
    
    let dest_path = assets_dir.join(&file_name);
    
    // Copy file
    std::fs::copy(&source_path, &dest_path).map_err(|e| e.to_string())?;
    
    // Generate embedding for tags (simplified - just store tags as JSON)
    let tags_json = tags.map(|t| serde_json::to_string(&t).unwrap_or_default());
    
    let asset = Asset {
        id: Uuid::new_v4().to_string(),
        asset_type: asset_type.clone(),
        path: dest_path.to_string_lossy().to_string(),
        tags: tags_json,
        metadata_json: metadata.map(|m| m.to_string()),
        source_url: None,
    };
    
    // Insert into database
    let conn = pool.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO assets (id, type, path, tags, metadata_json, source_url) VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![&asset.id, &asset.asset_type, &asset.path, &asset.tags, &asset.metadata_json, &asset.source_url],
    ).map_err(|e| e.to_string())?;
    
    Ok(asset)
}

#[tauri::command]
pub async fn delete_asset(
    pool: tauri::State<'_, DbPool>,
    asset_id: String,
) -> Result<(), String> {
    // Get asset path first
    let conn = pool.lock().map_err(|e| e.to_string())?;
    
    let path: String = conn.query_row(
        "SELECT path FROM assets WHERE id = ?",
        rusqlite::params![&asset_id],
        |row| row.get(0),
    ).optional().map_err(|e| e.to_string())?
    .ok_or("Asset not found".to_string())?;
    
    // Delete file
    std::fs::remove_file(&path).ok(); // Ignore errors
    
    // Delete from database
    conn.execute("DELETE FROM assets WHERE id = ?", rusqlite::params![&asset_id])
        .map_err(|e| e.to_string())?;
    
    Ok(())
}

#[tauri::command]
pub async fn sync_pexels_assets(
    app_handle: tauri::AppHandle,
    pool: tauri::State<'_, DbPool>,
    query: String,
) -> Result<Vec<Asset>, String> {
    // Pexels API integration (free tier)
    let api_key = std::env::var("PEXELS_API_KEY").ok();
    
    if api_key.is_none() {
        return Err("PEXELS_API_KEY not configured".to_string());
    }
    
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.pexels.com/videos/search")
        .header("Authorization", api_key.unwrap())
        .query(&[("query", query), ("per_page", "10".to_string()), ("orientation", "portrait".to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    
    if !response.status().is_success() {
        return Err("Pexels API error".to_string());
    }
    
    let data: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let videos: Vec<serde_json::Value> = data["videos"].as_array().cloned().unwrap_or_default();
    
    let mut imported = Vec::new();
    
    for video in videos {
        let video_files: Vec<serde_json::Value> = video["video_files"].as_array().cloned().unwrap_or_default();
        
        // Find best quality vertical video
        let best_file = video_files.iter()
            .filter(|f| f["width"].as_u64().unwrap_or(0) >= 1080)
            .max_by_key(|f| f["width"].as_u64().unwrap_or(0));
        
        if let Some(file) = best_file {
            let download_url = file["link"].as_str().unwrap_or("");
            if download_url.is_empty() {
                continue;
            }
            
            // Download and import
            let temp_path = std::env::temp_dir().join(format!("pexels_{}.mp4", Uuid::new_v4()));
            
            let video_response = client.get(download_url).send().await.ok();
            if let Some(vr) = video_response {
                if vr.status().is_success() {
                    let bytes = vr.bytes().await.ok();
                    if let Some(bytes) = bytes {
                        std::fs::write(&temp_path, bytes).ok();
                        
                        let tags = video["tags"].as_array()
                            .map(|t| t.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                            .unwrap_or_default();
                        
                        if let Ok(asset) = import_asset(
                            app_handle.clone(),
                            pool.clone(),
                            "broll".to_string(),
                            temp_path.to_string_lossy().to_string(),
                            Some(tags),
                            Some(serde_json::json!({
                                "source": "pexels",
                                "original_id": video["id"].as_u64().unwrap_or(0),
                                "photographer": video["user"]["name"].as_str().unwrap_or(""),
                            })),
                        ).await {
                            imported.push(asset);
                        }
                    }
                }
            }
        }
    }
    
    Ok(imported)
}
