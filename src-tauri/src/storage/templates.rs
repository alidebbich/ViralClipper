use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

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

