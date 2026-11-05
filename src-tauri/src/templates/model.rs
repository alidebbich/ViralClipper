//! templates/model.rs - template data types + the built-in styles (pure, no Tauri).
//! `templates/mod.rs` re-exports everything here and keeps the Tauri commands.

use anyhow::Result;
use serde::{Deserialize, Serialize};

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

/// Colors are stored as normal web colors "#RRGGBB" (the captions code converts to the
/// BGR format subtitles need). Old templates saved as "&H..." keep working unchanged.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AssStyle {
    /// Latin font family. Must exist in assets/fonts (or be installed).
    pub font: String,
    /// Arabic font family, used automatically for Arabic text.
    pub rtl_font: String,
    pub bold: bool,
    pub font_size: i32,
    pub hook_font_size: i32,
    /// Arabic letters look smaller than Latin ones at the same size: multiply by this for Arabic text
    pub rtl_scale: f64,
    /// normal text color
    #[serde(alias = "primary_color")]
    pub base_color: String,
    /// color of the word being spoken
    #[serde(alias = "secondary_color")]
    pub highlight_color: String,
    pub outline_color: String,
    pub background: String,
    pub outline_width: i32,
    pub shadow_offset: i32,
    pub blur: f64,
    /// dark rounded-looking box behind the captions instead of an outline
    pub box_style: bool,
    pub uppercase: bool,
    pub words_per_line: usize,
    /// vertical CENTER of the caption block, 0 = top, 1 = bottom (safe zone is ~0.28..0.78)
    pub caption_y: f64,
    pub hook_y: f64,
    pub chip_y: f64,
    /// how long the title chip stays (0 = never)
    pub chip_seconds: f64,
    pub hook_box: bool,
    /// Dedicated Title Overlay Configuration
    pub title_font_size: i32,
    pub title_color: String,
    pub title_outline_color: String,
    pub title_outline_width: i32,
    pub title_shadow: i32,
    pub title_y: f64,
    pub title_duration_s: f64,
    pub title_box: bool,
    /// legacy (ignored by the new engine, kept so old templates still load)
    pub alignment: i32,
    pub margin_v: i32,
    /// pop_in | highlight | fade
    pub karaoke_style: String,
    pub sync_nudge_ms: i32,
}

impl Default for AssStyle {
    fn default() -> Self {
        Self {
            font: "Poppins ExtraBold".into(),
            rtl_font: "Tajawal ExtraBold".into(),
            bold: false,
            font_size: 92,
            hook_font_size: 100,
            rtl_scale: 1.3,
            base_color: "#FFFFFF".into(),
            highlight_color: "#FFE600".into(),
            outline_color: "#000000".into(),
            background: "#000000".into(),
            outline_width: 7,
            shadow_offset: 0,
            blur: 0.0,
            box_style: false,
            uppercase: true,
            words_per_line: 3,
            caption_y: 0.62,
            hook_y: 0.26,
            chip_y: 0.17,
            chip_seconds: 4.0,
            hook_box: false,
            title_font_size: 98,
            title_color: "#FFFFFF".into(),
            title_outline_color: "#000000".into(),
            title_outline_width: 12,
            title_shadow: 4,
            title_y: 0.18,
            title_duration_s: 3.8,
            title_box: false,
            alignment: 2,
            margin_v: 300,
            karaoke_style: "pop_in".into(),
            sync_nudge_ms: 0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct FilterGraphConfig {
    /// zoom punches on phrase starts
    pub zoom_punch: bool,
    /// tighten silences (and alternate zoom on the cuts)
    pub jump_cuts: bool,
    /// prefer face-following 9:16 crop
    pub face_crop: bool,
    pub music_duck: bool,
    pub broll_overlay: bool,
    pub sfx_layer: bool,
    pub progress_bar: bool,
    /// auto | blur_fit | face_crop | split
    pub framing: String,
    /// multiplies zoom strength (1.0 normal, 1.3 aggressive)
    pub zoom_amount: f64,
    /// show the big hook text in the first seconds
    pub hook: bool,
}

impl Default for FilterGraphConfig {
    fn default() -> Self {
        Self {
            zoom_punch: true,
            jump_cuts: true,
            face_crop: true,
            music_duck: true,
            broll_overlay: false,
            sfx_layer: true,
            progress_bar: false,
            framing: "auto".into(),
            zoom_amount: 1.0,
            hook: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Theme {
    pub primary: String,
    pub secondary: String,
    pub background: String,
    pub accent: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary: "#FF0050".to_string(),
            secondary: "#00F2EA".to_string(),
            background: "#000000".to_string(),
            accent: "#FFF000".to_string(),
        }
    }
}

// ───────────────────────── built-in templates ─────────────────────────

pub const BUILTIN_IDS: [&str; 8] = [
    "tiktok", "hormozi", "cinematic", "minimal", "neon_gaming", "arabic_classic", "caption_top", "split_stack",
];

pub struct BuiltinTemplate {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub ass: AssStyle,
    pub filter: FilterGraphConfig,
    pub theme: Theme,
}

fn th(primary: &str, secondary: &str, accent: &str) -> Theme {
    Theme { primary: primary.into(), secondary: secondary.into(), background: "#000000".into(), accent: accent.into() }
}

pub fn builtin_templates() -> Vec<BuiltinTemplate> {
    let d = AssStyle::default;
    let f = FilterGraphConfig::default;
    vec![
        BuiltinTemplate {
            id: "tiktok",
            name: "Viral Bold",
            category: "video",
            description: "Big white caps, cyan active word, pop-in, face-follow crop, zoom punches, sfx",
            ass: AssStyle { highlight_color: "#00F2EA".into(), ..d() },
            filter: FilterGraphConfig { progress_bar: true, ..f() },
            theme: th("#FF2D55", "#00F2EA", "#FFE600"),
        },
        BuiltinTemplate {
            id: "hormozi",
            name: "Hormozi Style",
            category: "video",
            description: "Two huge words at a time, Anton font, yellow active word, aggressive punches",
            ass: AssStyle {
                font: "Anton".into(),
                font_size: 108,
                hook_font_size: 92,
                highlight_color: "#FFD60A".into(),
                outline_width: 8,
                words_per_line: 2,
                caption_y: 0.60,
                ..d()
            },
            filter: FilterGraphConfig { zoom_amount: 1.25, ..f() },
            theme: th("#FFD60A", "#FFFFFF", "#FF0000"),
        },
        BuiltinTemplate {
            id: "cinematic",
            name: "Cinematic Framed",
            category: "video",
            description: "Whole picture visible over a soft blurred backdrop, calm captions, no zoom punches",
            ass: AssStyle {
                font: "Poppins Medium".into(),
                font_size: 62,
                hook_font_size: 64,
                highlight_color: "#FFD27A".into(),
                outline_width: 3,
                shadow_offset: 2,
                uppercase: false,
                words_per_line: 4,
                caption_y: 0.72,
                karaoke_style: "fade".into(),
                ..d()
            },
            filter: FilterGraphConfig {
                zoom_punch: false,
                jump_cuts: false,
                face_crop: false,
                sfx_layer: false,
                framing: "blur_fit".into(),
                ..f()
            },
            theme: th("#8B5CF6", "#00D4FF", "#FFFFFF"),
        },
        BuiltinTemplate {
            id: "minimal",
            name: "Clean Minimal",
            category: "video",
            description: "Quiet soft-shadow captions, light touch editing, authentic look",
            ass: AssStyle {
                font: "Poppins Medium".into(),
                font_size: 58,
                hook_font_size: 60,
                highlight_color: "#7DD3FC".into(),
                outline_width: 0,
                shadow_offset: 3,
                uppercase: false,
                words_per_line: 4,
                caption_y: 0.66,
                karaoke_style: "highlight".into(),
                chip_seconds: 0.0,
                ..d()
            },
            filter: FilterGraphConfig {
                zoom_punch: false,
                jump_cuts: false,
                sfx_layer: false,
                music_duck: false,
                zoom_amount: 0.6,
                ..f()
            },
            theme: th("#FFFFFF", "#CCCCCC", "#007AFF"),
        },
        BuiltinTemplate {
            id: "neon_gaming",
            name: "Neon Gaming",
            category: "video",
            description: "Bebas caps with neon glow, whole gameplay frame kept, fast cuts, hard zooms, hit sounds",
            ass: AssStyle {
                font: "Bebas Neue".into(),
                font_size: 118,
                hook_font_size: 104,
                highlight_color: "#39FF14".into(),
                outline_color: "#3A0CA3".into(),
                outline_width: 6,
                blur: 1.3,
                caption_y: 0.64,
                ..d()
            },
            filter: FilterGraphConfig { framing: "blur_fit".into(), zoom_amount: 1.4, progress_bar: true, ..f() },
            theme: th("#39FF14", "#3A0CA3", "#FF00E5"),
        },
        BuiltinTemplate {
            id: "arabic_classic",
            name: "Arabic Classic",
            category: "video",
            description: "Arabic-first: Tajawal font, gold active word, soft dark caption box",
            ass: AssStyle {
                font: "Tajawal ExtraBold".into(),
                font_size: 92,
                hook_font_size: 84,
                highlight_color: "#FFC83D".into(),
                outline_color: "#000000".into(),
                outline_width: 14,
                box_style: true,
                uppercase: false,
                caption_y: 0.64,
                ..d()
            },
            filter: f(),
            theme: th("#FFC83D", "#FFFFFF", "#2DD4BF"),
        },
        BuiltinTemplate {
            id: "caption_top",
            name: "Captions on Top",
            category: "video",
            description: "Captions in the upper third, hook low: keeps the action and faces uncovered",
            ass: AssStyle { caption_y: 0.30, hook_y: 0.72, chip_y: 0.62, title_y: 0.72, ..d() },
            filter: f(),
            theme: th("#FF2D55", "#FFE600", "#FFFFFF"),
        },
        BuiltinTemplate {
            id: "split_stack",
            name: "Split Screen Stack",
            category: "video",
            description: "Speaker on top, looping background video below (add yours in assets/backgrounds)",
            ass: AssStyle { caption_y: 0.50, hook_y: 0.20, chip_y: 0.12, ..d() },
            filter: FilterGraphConfig { framing: "split".into(), ..f() },
            theme: th("#7C3AED", "#FFE600", "#FFFFFF"),
        },
    ]
}

/// Compatibility: the old API returned plain `Template`s.
pub fn default_templates() -> Vec<Template> {
    builtin_templates()
        .into_iter()
        .map(|b| Template {
            id: b.id.to_string(),
            name: b.name.to_string(),
            ass_style_json: serde_json::to_string(&b.ass).unwrap_or_default(),
            filter_graph_json: serde_json::to_string(&b.filter).unwrap_or_default(),
            theme_json: serde_json::to_string(&b.theme).unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_complete_and_distinct() {
        let b = builtin_templates();
        assert_eq!(b.len(), BUILTIN_IDS.len());
        for t in &b {
            assert!(BUILTIN_IDS.contains(&t.id));
            assert!(t.ass.caption_y > 0.2 && t.ass.caption_y < 0.8, "{} caption_y outside safe zone", t.id);
            assert!(t.ass.base_color.starts_with('#') && t.ass.highlight_color.starts_with('#'));
        }
        let mut fonts: Vec<&str> = b.iter().map(|t| t.ass.font.as_str()).collect();
        fonts.sort();
        fonts.dedup();
        assert!(fonts.len() >= 4, "templates must really look different");
    }

    #[test]
    fn legacy_json_still_loads() {
        let old = r##"{"font":"Anton","font_size":78,"primary_color":"&HFFFF00","secondary_color":"&HFFFFFF",
            "outline_color":"&H000000","outline_width":4,"shadow_offset":3,"alignment":2,"margin_v":300,"karaoke_style":"highlight"}"##;
        let s: AssStyle = serde_json::from_str(old).unwrap();
        assert_eq!(s.base_color, "&HFFFF00");
        assert_eq!(s.highlight_color, "&HFFFFFF");
        assert_eq!(s.caption_y, 0.62); // new fields get defaults
        let f: FilterGraphConfig = serde_json::from_str(r#"{"zoom_punch":false}"#).unwrap();
        assert!(!f.zoom_punch && f.hook);
    }
}
