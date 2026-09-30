use anyhow::Result;
use serde::{Deserialize, Serialize};
use rusqlite::OptionalExtension;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub ass_style_json: String,
    pub filter_graph_json: String,
    pub theme_json: String,
}

impl Template {
    pub fn ass_style(&self) -> Result<AssStyle> {
        serde_json::from_str(&self.ass_style_json).map_err(|e| anyhow::anyhow!(e))
    }
    
    pub fn filter_graph(&self) -> Result<FilterGraphConfig> {
        serde_json::from_str(&self.filter_graph_json).map_err(|e| anyhow::anyhow!(e))
    }
    
    pub fn theme(&self) -> Result<Theme> {
        serde_json::from_str(&self.theme_json).map_err(|e| anyhow::anyhow!(e))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AssStyle {
    pub font: String,
    pub font_size: i32,
    pub primary_color: String,
    pub secondary_color: String,
    pub outline_color: String,
    pub background: String,
    pub outline_width: i32,
    pub shadow_offset: i32,
    pub alignment: i32,
    pub margin_v: i32,
    pub karaoke_style: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FilterGraphConfig {
    pub zoom_punch: bool,
    pub jump_cuts: bool,
    pub face_crop: bool,
    pub music_duck: bool,
    pub broll_overlay: bool,
    pub sfx_layer: bool,
    pub progress_bar: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Theme {
    pub primary: String,
    pub secondary: String,
    pub background: String,
    pub accent: String,
}

pub fn default_templates() -> Vec<Template> {
    vec![
        Template {
            id: "tiktok".to_string(),
            name: "TikTok".to_string(),
            ass_style_json: serde_json::json!({
                "font": "Montserrat Bold",
                "font_size": 72,
                "primary_color": "&HFFFFFF",
                "outline_color": "&H000000",
                "outline_width": 3,
                "shadow_offset": 2,
                "alignment": 2,
                "margin_v": 150,
                "karaoke_style": "pop_in"
            }).to_string(),
            filter_graph_json: serde_json::json!({
                "zoom_punch": true,
                "jump_cuts": true,
                "face_crop": true,
                "music_duck": true,
                "broll_overlay": true,
                "sfx_layer": true,
                "progress_bar": true
            }).to_string(),
            theme_json: serde_json::json!({
                "primary": "#FF0050",
                "secondary": "#00F2EA",
                "background": "#000000",
                "accent": "#FFF000"
            }).to_string(),
        },
        Template {
            id: "hormozi".to_string(),
            name: "Hormozi".to_string(),
            ass_style_json: serde_json::json!({
                "font": "Anton",
                "font_size": 80,
                "primary_color": "&HFFFF00",
                "outline_color": "&H000000",
                "outline_width": 4,
                "shadow_offset": 3,
                "alignment": 2,
                "margin_v": 180,
                "karaoke_style": "highlight"
            }).to_string(),
            filter_graph_json: serde_json::json!({
                "zoom_punch": true,
                "jump_cuts": true,
                "face_crop": true,
                "music_duck": true,
                "broll_overlay": false,
                "sfx_layer": true,
                "progress_bar": false
            }).to_string(),
            theme_json: serde_json::json!({
                "primary": "#FFFF00",
                "secondary": "#FFFFFF",
                "background": "#000000",
                "accent": "#FF0000"
            }).to_string(),
        },
        Template {
            id: "minimal".to_string(),
            name: "Minimal".to_string(),
            ass_style_json: serde_json::json!({
                "font": "Inter Medium",
                "font_size": 56,
                "primary_color": "&HFFFFFF",
                "outline_color": "&H00000000",
                "outline_width": 0,
                "shadow_offset": 0,
                "alignment": 2,
                "margin_v": 120,
                "karaoke_style": "fade"
            }).to_string(),
            filter_graph_json: serde_json::json!({
                "zoom_punch": false,
                "jump_cuts": true,
                "face_crop": true,
                "music_duck": true,
                "broll_overlay": false,
                "sfx_layer": false,
                "progress_bar": true
            }).to_string(),
            theme_json: serde_json::json!({
                "primary": "#FFFFFF",
                "secondary": "#CCCCCC",
                "background": "#00000000",
                "accent": "#007AFF"
            }).to_string(),
        },
    ]
}

// Tauri commands for templates module
#[tauri::command]
pub async fn list_templates(
    pool: tauri::State<'_, crate::database::DbPool>,
) -> Result<Vec<Template>, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, name, ass_style_json, filter_graph_json, theme_json, thumbnail_json, category, description FROM templates ORDER BY name"
    ).map_err(|e| e.to_string())?;

    let templates = stmt
        .query_map([], |row| {
            Ok(Template {
                id: row.get(0)?,
                name: row.get(1)?,
                ass_style_json: row.get(2)?,
                filter_graph_json: row.get(3)?,
                theme_json: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(templates)
}

#[tauri::command]
pub async fn get_template(
    pool: tauri::State<'_, crate::database::DbPool>,
    template_id: String,
) -> Result<Template, String> {
    let conn = pool.lock().map_err(|e| e.to_string())?;

    let template = conn.query_row(
        "SELECT id, name, ass_style_json, filter_graph_json, theme_json, thumbnail_json, category, description FROM templates WHERE id = ?",
        rusqlite::params![&template_id],
        |row| {
            Ok(Template {
                id: row.get(0)?,
                name: row.get(1)?,
                ass_style_json: row.get(2)?,
                filter_graph_json: row.get(3)?,
                theme_json: row.get(4)?,
            })
        },
    ).optional().map_err(|e| e.to_string())?
    .ok_or_else(|| "Template not found".to_string())?;

    Ok(template)
}

#[tauri::command]
pub async fn save_template(
    pool: tauri::State<'_, crate::database::DbPool>,
    name: String,
    ass_style: AssStyle,
    filter_graph: FilterGraphConfig,
    theme: Theme,
) -> Result<Template, String> {
    let id = name.to_lowercase().replace(' ', "-");
    let ass_style_json = serde_json::to_string(&ass_style).map_err(|e| e.to_string())?;
    let filter_graph_json = serde_json::to_string(&filter_graph).map_err(|e| e.to_string())?;
    let theme_json = serde_json::to_string(&theme).map_err(|e| e.to_string())?;

    let conn = pool.lock().map_err(|e| e.to_string())?;

    conn.execute(
        r#"
        INSERT INTO templates (id, name, ass_style_json, filter_graph_json, theme_json)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            ass_style_json = excluded.ass_style_json,
            filter_graph_json = excluded.filter_graph_json,
            theme_json = excluded.theme_json
        "#,
        rusqlite::params![&id, &name, &ass_style_json, &filter_graph_json, &theme_json],
    ).map_err(|e| e.to_string())?;

    Ok(Template {
        id,
        name,
        ass_style_json,
        filter_graph_json,
        theme_json,
    })
}

#[tauri::command]
pub async fn delete_template(
    pool: tauri::State<'_, crate::database::DbPool>,
    template_id: String,
) -> Result<(), String> {
    // Prevent deleting built-in templates
    let builtin = ["tiktok", "hormozi", "minimal"];
    if builtin.contains(&template_id.as_str()) {
        return Err("Cannot delete built-in template".to_string());
    }

    let conn = pool.lock().map_err(|e| e.to_string())?;

    conn.execute("DELETE FROM templates WHERE id = ?", rusqlite::params![&template_id])
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn import_template(
    pool: tauri::State<'_, crate::database::DbPool>,
    json: String,
) -> Result<Template, String> {
    let template: Template = serde_json::from_str(&json).map_err(|e| e.to_string())?;

    let conn = pool.lock().map_err(|e| e.to_string())?;

    conn.execute(
        r#"
        INSERT INTO templates (id, name, ass_style_json, filter_graph_json, theme_json)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            ass_style_json = excluded.ass_style_json,
            filter_graph_json = excluded.filter_graph_json,
            theme_json = excluded.theme_json
        "#,
        rusqlite::params![&template.id, &template.name, &template.ass_style_json, &template.filter_graph_json, &template.theme_json],
    ).map_err(|e| e.to_string())?;

    Ok(template)
}

#[tauri::command]
pub async fn export_template(
    pool: tauri::State<'_, crate::database::DbPool>,
    template_id: String,
) -> Result<String, String> {
    let template = get_template(pool, template_id).await?;
    serde_json::to_string_pretty(&template).map_err(|e| e.to_string())
}