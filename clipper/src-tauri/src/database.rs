// Database module using sqlx with sqlite-vec for vector search

use sqlx::{sqlite::SqlitePoolOptions, SqlitePool, Row};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub source_url: String,
    pub source_path: Option<String>,
    pub title: String,
    pub status: String,
    pub progress: f32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Clip {
    pub id: String,
    pub project_id: String,
    pub start_s: f32,
    pub end_s: f32,
    pub duration_s: f32,
    pub title: String,
    pub hook: String,
    pub viral_score: f32,
    pub heuristic_score: f32,
    pub heuristic_json: String,
    pub llm_reason: String,
    pub template_id: String,
    pub render_path: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Asset {
    pub id: String,
    pub asset_type: String, // broll, music, sfx, template
    pub path: String,
    pub tags: String, // JSON array
    pub tags_vec: Option<Vec<f32>>, // sqlite-vec embedding
    pub metadata_json: String,
    pub source_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub ass_style_json: String,
    pub filter_graph_json: String,
    pub theme_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Preference {
    pub key: String,
    pub value_json: String,
}

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(db_path: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&format!("sqlite://{}?mode=rwc", db_path))
            .await?;
        
        let db = Database { pool };
        db.migrate().await?;
        Ok(db)
    }
    
    async fn migrate(&self) -> Result<()> {
        // Enable sqlite-vec extension
        sqlx::query("SELECT load_extension('vec0')").execute(&self.pool).await.ok();
        
        // Projects table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                source_url TEXT NOT NULL,
                source_path TEXT,
                title TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'pending',
                progress REAL NOT NULL DEFAULT 0.0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                error TEXT
            )
        "#).execute(&self.pool).await?;
        
        // Clips table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS clips (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                start_s REAL NOT NULL,
                end_s REAL NOT NULL,
                duration_s REAL NOT NULL,
                title TEXT NOT NULL,
                hook TEXT NOT NULL,
                viral_score REAL NOT NULL DEFAULT 0.0,
                heuristic_score REAL NOT NULL DEFAULT 0.0,
                heuristic_json TEXT NOT NULL DEFAULT '{}',
                llm_reason TEXT NOT NULL DEFAULT '',
                template_id TEXT NOT NULL DEFAULT 'default',
                render_path TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
        "#).execute(&self.pool).await?;
        
        // Assets table with vector column
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                asset_type TEXT NOT NULL,
                path TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                tags_vec BLOB, -- sqlite-vec vector
                metadata_json TEXT NOT NULL DEFAULT '{}',
                source_url TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
        "#).execute(&self.pool).await?;
        
        // Create vector index for assets
        sqlx::query(r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS assets_vec USING vec0(
                tags_vec float[384]
            )
        "#).execute(&self.pool).await.ok(); // May fail if vec0 not loaded
        
        // Templates table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS templates (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                ass_style_json TEXT NOT NULL,
                filter_graph_json TEXT NOT NULL,
                theme_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
        "#).execute(&self.pool).await?;
        
        // Preferences table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS prefs (
                key TEXT PRIMARY KEY,
                value_json TEXT NOT NULL
            )
        "#).execute(&self.pool).await?;
        
        // Indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_clips_project ON clips(project_id)").execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_assets_type ON assets(asset_type)").execute(&self.pool).await?;
        
        // Insert default templates
        self.seed_default_templates().await?;
        
        Ok(())
    }
    
    async fn seed_default_templates(&self) -> Result<()> {
        let templates = vec![
            Template {
                id: "tiktok".to_string(),
                name: "TikTok Style".to_string(),
                ass_style_json: serde_json::to_string(&serde_json::json!({
                    "Fontname": "Arial",
                    "Fontsize": 28,
                    "PrimaryColour": "&H00FFFFFF",
                    "SecondaryColour": "&H0000FFFF",
                    "OutlineColour": "&H00000000",
                    "BackColour": "&H80000000",
                    "Bold": 1,
                    "Outline": 3,
                    "Shadow": 2,
                    "Alignment": 2,
                    "MarginV": 40,
                }))?,
                filter_graph_json: serde_json::to_string(&serde_json::json!({
                    "zoom_punch_intensity": 1.3,
                    "jump_cut_threshold": 0.5,
                    "progress_bar_color": "#00D4FF",
                }))?,
                theme_json: serde_json::to_string(&serde_json::json!({
                    "primary": "#00D4FF",
                    "secondary": "#FF0050",
                    "background": "#000000",
                    "caption_style": "karaoke_pop",
                }))?,
                created_at: Utc::now(),
            },
            Template {
                id: "hormozi".to_string(),
                name: "Hormozi Style".to_string(),
                ass_style_json: serde_json::to_string(&serde_json::json!({
                    "Fontname": "Arial Black",
                    "Fontsize": 32,
                    "PrimaryColour": "&H00FFFF00",
                    "SecondaryColour": "&H000000FF",
                    "OutlineColour": "&H00000000",
                    "BackColour": "&H00000000",
                    "Bold": 1,
                    "Outline": 4,
                    "Shadow": 0,
                    "Alignment": 2,
                    "MarginV": 60,
                }))?,
                filter_graph_json: serde_json::to_string(&serde_json::json!({
                    "zoom_punch_intensity": 1.2,
                    "jump_cut_threshold": 0.3,
                    "progress_bar_color": "#FFFF00",
                }))?,
                theme_json: serde_json::to_string(&serde_json::json!({
                    "primary": "#FFFF00",
                    "secondary": "#FF6B00",
                    "background": "#000000",
                    "caption_style": "bold_highlight",
                }))?,
                created_at: Utc::now(),
            },
            Template {
                id: "minimal".to_string(),
                name: "Minimal Clean".to_string(),
                ass_style_json: serde_json::to_string(&serde_json::json!({
                    "Fontname": "Inter",
                    "Fontsize": 24,
                    "PrimaryColour": "&H00FFFFFF",
                    "SecondaryColour": "&H00CCCCCC",
                    "OutlineColour": "&H00000000",
                    "BackColour": "&H00000000",
                    "Bold": 0,
                    "Outline": 1,
                    "Shadow": 1,
                    "Alignment": 2,
                    "MarginV": 30,
                }))?,
                filter_graph_json: serde_json::to_string(&serde_json::json!({
                    "zoom_punch_intensity": 1.15,
                    "jump_cut_threshold": 0.6,
                    "progress_bar_color": "#FFFFFF",
                }))?,
                theme_json: serde_json::to_string(&serde_json::json!({
                    "primary": "#FFFFFF",
                    "secondary": "#CCCCCC",
                    "background": "#000000",
                    "caption_style": "clean_fade",
                }))?,
                created_at: Utc::now(),
            },
        ];
        
        for template in templates {
            sqlx::query(r#"
                INSERT OR IGNORE INTO templates (id, name, ass_style_json, filter_graph_json, theme_json, created_at)
                VALUES (?, ?, ?, ?, ?, ?)
            "#)
            .bind(&template.id)
            .bind(&template.name)
            .bind(&template.ass_style_json)
            .bind(&template.filter_graph_json)
            .bind(&template.theme_json)
            .bind(template.created_at)
            .execute(&self.pool)
            .await?;
        }
        
        Ok(())
    }
    
    // Project operations
    pub async fn create_project(&self, project: &Project) -> Result<()> {
        sqlx::query(r#"
            INSERT INTO projects (id, source_url, source_path, title, status, progress, created_at, updated_at, error)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#)
        .bind(&project.id)
        .bind(&project.source_url)
        .bind(&project.source_path)
        .bind(&project.title)
        .bind(&project.status)
        .bind(project.progress)
        .bind(project.created_at)
        .bind(project.updated_at)
        .bind(&project.error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    
    pub async fn update_project(&self, project: &Project) -> Result<()> {
        sqlx::query(r#"
            UPDATE projects SET source_path = ?, title = ?, status = ?, progress = ?, updated_at = ?, error = ?
            WHERE id = ?
        "#)
        .bind(&project.source_path)
        .bind(&project.title)
        .bind(&project.status)
        .bind(project.progress)
        .bind(project.updated_at)
        .bind(&project.error)
        .bind(&project.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    
    pub async fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let row = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }
    
    pub async fn list_projects(&self, limit: i64, offset: i64) -> Result<Vec<Project>> {
        let rows = sqlx::query_as::<_, Project>(
            "SELECT * FROM projects ORDER BY created_at DESC LIMIT ? OFFSET ?"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
    
    // Clip operations
    pub async fn create_clip(&self, clip: &Clip) -> Result<()> {
        sqlx::query(r#"
            INSERT INTO clips (id, project_id, start_s, end_s, duration_s, title, hook, viral_score, heuristic_score, heuristic_json, llm_reason, template_id, render_path, status, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#)
        .bind(&clip.id)
        .bind(&clip.project_id)
        .bind(clip.start_s)
        .bind(clip.end_s)
        .bind(clip.duration_s)
        .bind(&clip.title)
        .bind(&clip.hook)
        .bind(clip.viral_score)
        .bind(clip.heuristic_score)
        .bind(&clip.heuristic_json)
        .bind(&clip.llm_reason)
        .bind(&clip.template_id)
        .bind(&clip.render_path)
        .bind(&clip.status)
        .bind(clip.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    
    pub async fn get_clips_by_project(&self, project_id: &str) -> Result<Vec<Clip>> {
        let rows = sqlx::query_as::<_, Clip>(
            "SELECT * FROM clips WHERE project_id = ? ORDER BY viral_score DESC"
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
    
    pub async fn update_clip_status(&self, clip_id: &str, status: &str, render_path: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE clips SET status = ?, render_path = ? WHERE id = ?")
            .bind(status)
            .bind(render_path)
            .bind(clip_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    
    // Asset operations
    pub async fn create_asset(&self, asset: &Asset) -> Result<()> {
        sqlx::query(r#"
            INSERT INTO assets (id, asset_type, path, tags, tags_vec, metadata_json, source_url, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#)
        .bind(&asset.id)
        .bind(&asset.asset_type)
        .bind(&asset.path)
        .bind(&asset.tags)
        .bind(asset.tags_vec.as_ref().map(|v| serde_json::to_string(v).unwrap()))
        .bind(&asset.metadata_json)
        .bind(&asset.source_url)
        .bind(asset.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    
    pub async fn search_assets(&self, asset_type: &str, query_embedding: &[f32], limit: i64) -> Result<Vec<Asset>> {
        // Vector similarity search using sqlite-vec
        // This is a simplified version - actual implementation depends on vec0 API
        let rows = sqlx::query_as::<_, Asset>(
            "SELECT * FROM assets WHERE asset_type = ? ORDER BY created_at DESC LIMIT ?"
        )
        .bind(asset_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
    
    pub async fn list_assets(&self, asset_type: Option<&str>, limit: i64, offset: i64) -> Result<Vec<Asset>> {
        let query = if let Some(t) = asset_type {
            "SELECT * FROM assets WHERE asset_type = ? ORDER BY created_at DESC LIMIT ? OFFSET ?"
        } else {
            "SELECT * FROM assets ORDER BY created_at DESC LIMIT ? OFFSET ?"
        };
        
        let mut q = sqlx::query_as::<_, Asset>(query);
        if asset_type.is_some() {
            q = q.bind(asset_type.unwrap());
        }
        q = q.bind(limit).bind(offset);
        
        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows)
    }
    
    // Template operations
    pub async fn get_template(&self, id: &str) -> Result<Option<Template>> {
        let row = sqlx::query_as::<_, Template>("SELECT * FROM templates WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }
    
    pub async fn list_templates(&self) -> Result<Vec<Template>> {
        let rows = sqlx::query_as::<_, Template>("SELECT * FROM templates ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }
    
    // Preferences
    pub async fn set_pref(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO prefs (key, value_json) VALUES (?, ?)")
            .bind(key)
            .bind(value.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    
    pub async fn get_pref(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value_json FROM prefs WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(v,)| serde_json::from_str(&v).unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    
    #[tokio::test]
    async fn test_database() {
        let tmp = NamedTempFile::new().unwrap();
        let db = Database::new(tmp.path().to_str().unwrap()).await.unwrap();
        
        let project = Project {
            id: Uuid::new_v4().to_string(),
            source_url: "https://youtube.com/watch?v=test".to_string(),
            source_path: None,
            title: "Test Project".to_string(),
            status: "pending".to_string(),
            progress: 0.0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            error: None,
        };
        
        db.create_project(&project).await.unwrap();
        let loaded = db.get_project(&project.id).await.unwrap().unwrap();
        assert_eq!(loaded.title, "Test Project");
    }
}