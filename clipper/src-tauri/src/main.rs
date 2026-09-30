#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            greet,
            start_pipeline,
            check_dependencies,
            get_projects,
            get_project,
            delete_project,
            get_templates,
            get_assets,
            add_asset,
        ])
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                let window = app.get_webview_window("main").unwrap();
                window.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to Clipper.", name)
}

// Re-export pipeline commands
mod pipeline;
use pipeline::{start_pipeline, check_dependencies};

// Database commands
mod database;
use database::Database;

#[tauri::command]
async fn get_projects() -> Result<Vec<database::Project>, String> {
    let db = Database::new("clipper.db").await.map_err(|e| e.to_string())?;
    db.list_projects(50, 0).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_project(id: String) -> Result<Option<database::Project>, String> {
    let db = Database::new("clipper.db").await.map_err(|e| e.to_string())?;
    db.get_project(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_project(id: String) -> Result<(), String> {
    let db = Database::new("clipper.db").await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&id)
        .execute(&db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_templates() -> Result<Vec<database::Template>, String> {
    let db = Database::new("clipper.db").await.map_err(|e| e.to_string())?;
    db.list_templates().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_assets(asset_type: Option<String>) -> Result<Vec<database::Asset>, String> {
    let db = Database::new("clipper.db").await.map_err(|e| e.to_string())?;
    db.list_assets(asset_type.as_deref(), 100, 0).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_asset(
    asset_type: String,
    path: String,
    tags: Vec<String>,
    metadata: serde_json::Value,
    source_url: Option<String>,
) -> Result<database::Asset, String> {
    let db = Database::new("clipper.db").await.map_err(|e| e.to_string())?;
    let asset = database::Asset {
        id: uuid::Uuid::new_v4().to_string(),
        asset_type,
        path,
        tags: serde_json::to_string(&tags).unwrap(),
        tags_vec: None,
        metadata_json: metadata.to_string(),
        source_url,
        created_at: chrono::Utc::now(),
    };
    db.create_asset(&asset).await.map_err(|e| e.to_string())?;
    Ok(asset)
}