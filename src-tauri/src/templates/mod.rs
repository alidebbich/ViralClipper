//! templates/mod.rs - types + built-in templates live in model.rs; this file keeps the Tauri commands.
mod model;
pub use model::*;

use rusqlite::OptionalExtension;

#[tauri::command]
pub async fn list_templates(
    pool: tauri::State<'_, crate::storage::DbPool>,
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
    pool: tauri::State<'_, crate::storage::DbPool>,
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

/// Built-in templates are re-created on every start, so edits to them would be lost.
/// Saving under a built-in name therefore creates "<id>-custom" instead.
fn user_id_for(name_or_id: &str) -> String {
    let id = name_or_id.to_lowercase().replace(' ', "-");
    if BUILTIN_IDS.contains(&id.as_str()) { format!("{}-custom", id) } else { id }
}

#[tauri::command]
pub async fn save_template(
    pool: tauri::State<'_, crate::storage::DbPool>,
    name: String,
    ass_style: AssStyle,
    filter_graph: FilterGraphConfig,
    theme: Theme,
) -> Result<Template, String> {
    let id = user_id_for(&name);
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

    Ok(Template { id, name, ass_style_json, filter_graph_json, theme_json })
}

#[tauri::command]
pub async fn delete_template(
    pool: tauri::State<'_, crate::storage::DbPool>,
    template_id: String,
) -> Result<(), String> {
    if BUILTIN_IDS.contains(&template_id.as_str()) {
        return Err("Cannot delete built-in template".to_string());
    }
    let conn = pool.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM templates WHERE id = ?", rusqlite::params![&template_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn import_template(
    pool: tauri::State<'_, crate::storage::DbPool>,
    json: String,
) -> Result<Template, String> {
    let mut template: Template = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    template.id = user_id_for(&template.id);

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
    pool: tauri::State<'_, crate::storage::DbPool>,
    template_id: String,
) -> Result<String, String> {
    let template = get_template(pool, template_id).await?;
    serde_json::to_string_pretty(&template).map_err(|e| e.to_string())
}
