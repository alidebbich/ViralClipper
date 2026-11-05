//! captions.rs - ASS subtitle generation: hook, title chip, word-highlight captions.
//!
//! Key rules (see .agent/rules/arabic-rtl-captions.md):
//!  * text is ALWAYS passed to libass in logical order. Never reversed, never reshaped, never split
//!    inside a word. libass + HarfBuzz + FriBidi do the Arabic shaping.
//!  * colors are written as real ASS BGR. Templates store normal "#RRGGBB".
//!  * every event sets its own \an5\pos(x,y) so the position comes from the template, not margins.

use anyhow::Result;

use super::edit_plan::{EdlView, Timeline, OUT_H, OUT_W};
use crate::common::{Clip, TranscriptSegment};
use crate::templates::Template;

// ───────────────────────── small helpers ─────────────────────────

pub fn generate_ass_path(clip: &Clip, _template: &Template) -> String {
    let cache_base = std::env::var("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let cache_dir = cache_base.join("clipper").join("ass");
    let _ = std::fs::create_dir_all(&cache_dir);
    cache_dir.join(format!("{}.ass", clip.id)).to_string_lossy().to_string()
}

pub fn format_time_ass(seconds: f64) -> String {
    let cs_total = (seconds.max(0.0) * 100.0).round() as i64;
    let h = cs_total / 360_000;
    let m = (cs_total % 360_000) / 6_000;
    let s = (cs_total % 6_000) / 100;
    let cs = cs_total % 100;
    format!("{}:{:02}:{:02}.{:02}", h, m, s, cs)
}

pub fn check_malformed_tags(text: &str) {
    if has_stray_backslash(text) {
        tracing::warn!("Malformed ASS tag: backslash outside braces in: {}", text);
    }
}

/// true when a backslash appears outside `{...}` and is not \N \n \h
pub fn has_stray_backslash(text: &str) -> bool {
    let mut in_brace = false;
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        match chars[i] {
            '{' => in_brace = true,
            '}' => in_brace = false,
            '\\' if !in_brace => {
                let n = chars.get(i + 1).copied().unwrap_or(' ');
                if n != 'N' && n != 'n' && n != 'h' {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

pub fn contains_arabic(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
    })
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE00..=0xFE0F | 0x200D | 0x20E3 | 0x2190..=0x21FF)
}

/// Remove everything that must never reach libass: tags, emoji (libass cannot draw color emoji),
/// control and zero-width characters. Word order and letters are left untouched.
pub fn clean_ass_text(s: &str, upper: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '{' | '}') || c.is_control() || is_emoji(c) || matches!(c as u32, 0x200B | 0xFEFF) {
            if c == '\n' || c == '\r' || c == '\t' {
                out.push(' ');
            }
            continue;
        }
        out.push(c);
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if upper && !contains_arabic(&collapsed) {
        collapsed.to_uppercase()
    } else {
        collapsed
    }
}

fn parse_color(c: &str) -> (u8, u8, u8) {
    let t = c.trim();
    let hex = |s: &str| u8::from_str_radix(s, 16).unwrap_or(255);
    if let Some(h) = t.strip_prefix('#') {
        if h.len() >= 6 {
            return (hex(&h[0..2]), hex(&h[2..4]), hex(&h[4..6])); // R,G,B
        }
    } else if t.starts_with("&H") || t.starts_with("&h") {
        let h = t[2..].trim_end_matches('&');
        let h = if h.len() > 6 { &h[h.len() - 6..] } else { h };
        if h.len() == 6 {
            return (hex(&h[4..6]), hex(&h[2..4]), hex(&h[0..2])); // legacy value is already BBGGRR
        }
    }
    (255, 255, 255)
}

/// ASS style color: &HAABBGGRR (alpha 00 = opaque, FF = invisible)
pub fn style_color(c: &str, alpha: u8) -> String {
    let (r, g, b) = parse_color(c);
    format!("&H{:02X}{:02X}{:02X}{:02X}", alpha, b, g, r)
}

/// ASS override-tag color: \c&HBBGGRR&
pub fn tag_color(c: &str) -> String {
    let (r, g, b) = parse_color(c);
    format!("&H{:02X}{:02X}{:02X}&", b, g, r)
}

// ───────────────────────── words ─────────────────────────

#[derive(Debug, Clone)]
pub struct RelWord {
    /// seconds from the START OF THE CLIP
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Words of this clip with times relative to the clip start.
/// If the transcript has no word timestamps, words are spread evenly across each segment.
pub fn collect_clip_words(clip: &Clip, transcript: &[TranscriptSegment], is_clip_relative: bool) -> Vec<RelWord> {
    let rel = is_clip_relative || clip.is_clip_relative;
    let base = if rel { 0.0 } else { clip.start_s };
    let (lo, hi) = if rel { (-0.05, clip.duration_s + 0.05) } else { (clip.start_s - 0.05, clip.end_s + 0.05) };
    let mut out: Vec<RelWord> = Vec::new();

    for seg in transcript {
        for w in &seg.words {
            let t = w.text.trim();
            if t.is_empty() || w.end < lo || w.start > hi {
                continue;
            }
            let s = (w.start - base).max(0.0);
            let e = (w.end - base).max(s + 0.05).min(clip.duration_s.max(s + 0.05));
            out.push(RelWord { start: s, end: e, text: t.to_string() });
        }
    }
    if out.is_empty() {
        for seg in transcript {
            let (s0, s1) = (seg.start - base, seg.end - base);
            if s1 < 0.0 || s0 > clip.duration_s {
                continue;
            }
            let toks: Vec<&str> = seg.text.split_whitespace().collect();
            if toks.is_empty() {
                continue;
            }
            let (s0, s1) = (s0.max(0.0), s1.min(clip.duration_s));
            let step = (s1 - s0).max(0.1) / toks.len() as f64;
            for (i, t) in toks.iter().enumerate() {
                let s = s0 + step * i as f64;
                out.push(RelWord { start: s, end: s + step * 0.95, text: (*t).to_string() });
            }
        }
    }
    out.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    out
}

fn ends_phrase(t: &str) -> bool {
    t.ends_with(['.', '!', '?', ',', ';', '\u{060C}', '\u{061F}', '\u{061B}'])
}

pub fn chunk_words(words: &[RelWord], max_words: usize) -> Vec<Vec<RelWord>> {
    let max_words = max_words.clamp(1, 6);
    let mut chunks: Vec<Vec<RelWord>> = Vec::new();
    let mut cur: Vec<RelWord> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        cur.push(w.clone());
        let gap_next = words.get(i + 1).map(|n| n.start - w.end).unwrap_or(0.0);
        // Avoid 1-word flashes on simple commas unless there's a genuine breathing pause (>0.35s).
        let is_comma_pause = w.text.ends_with([',', ';', '\u{060C}', '\u{061B}']);
        let break_phrase = if is_comma_pause {
            cur.len() >= 2 || gap_next > 0.35
        } else {
            ends_phrase(&w.text)
        };
        if cur.len() >= max_words || break_phrase || gap_next > 0.6 {
            chunks.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

// ───────────────────────── main entry points ─────────────────────────

/// Backward compatible entry point (no jump cuts).
pub fn generate_ass_file(clip: &Clip, transcript: &[TranscriptSegment], template: &Template, is_clip_relative: bool) -> Result<String> {
    generate_ass_file_with_timeline(clip, transcript, template, is_clip_relative, &Timeline::identity(clip.duration_s))
}

// ───────────────────────── title & duplication helpers ─────────────────────────

pub fn normalize_for_comparison(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Checks whether a candidate title substantially duplicates the opening spoken speech.
/// Returns true if >50% of the candidate words match opening speech words or if exact match.
pub fn is_substantially_duplicate(candidate: &str, opening_words: &[String]) -> bool {
    let norm_cand = normalize_for_comparison(candidate);
    if norm_cand.is_empty() {
        return false;
    }
    let cand_tokens: Vec<&str> = norm_cand.split_whitespace().collect();
    if cand_tokens.is_empty() {
        return false;
    }

    let norm_speech = normalize_for_comparison(&opening_words.join(" "));
    let speech_tokens: Vec<&str> = norm_speech.split_whitespace().collect();
    if speech_tokens.is_empty() {
        return false;
    }

    // Exact match or prefix match
    if norm_speech.starts_with(&norm_cand) || norm_cand.starts_with(&norm_speech) {
        return true;
    }

    // Token overlap: if >= 50% of candidate words match opening speech words
    let mut matches = 0;
    for token in &cand_tokens {
        if speech_tokens.contains(token) {
            matches += 1;
        }
    }
    (matches as f64 / cand_tokens.len() as f64) >= 0.50
}

/// Intelligently wraps a title at word boundaries into 2 (or max 3) balanced lines using \N.
pub fn wrap_title_text(text: &str, max_chars_per_line: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 3 || text.len() <= max_chars_per_line {
        return text.to_string();
    }

    // Balanced 2-line wrap near the midpoint
    let total_chars: usize = words.iter().map(|w| w.len()).sum::<usize>() + words.len().saturating_sub(1);
    let half_chars = total_chars / 2;

    let mut best_split = 1;
    let mut best_diff = usize::MAX;
    let mut cur_chars = 0;

    for (i, w) in words.iter().enumerate().take(words.len() - 1) {
        cur_chars += w.len() + if i > 0 { 1 } else { 0 };
        let diff = if cur_chars > half_chars { cur_chars - half_chars } else { half_chars - cur_chars };
        if diff < best_diff {
            best_diff = diff;
            best_split = i + 1;
        }
    }

    if words.len() > 8 || total_chars > max_chars_per_line * 2 {
        let mut lines = Vec::new();
        let mut cur_line = Vec::new();
        let mut cur_len = 0;
        for w in words {
            if cur_len + w.len() + 1 > max_chars_per_line && !cur_line.is_empty() {
                lines.push(cur_line.join(" "));
                cur_line = vec![w];
                cur_len = w.len();
            } else {
                cur_len += w.len() + if cur_line.is_empty() { 0 } else { 1 };
                cur_line.push(w);
            }
        }
        if !cur_line.is_empty() {
            lines.push(cur_line.join(" "));
        }
        lines.join("\\N")
    } else {
        let line1 = words[..best_split].join(" ");
        let line2 = words[best_split..].join(" ");
        format!("{}\\N{}", line1, line2)
    }
}

/// Calculates responsive, prominent title font size based on output dimensions and text length.
/// Target: 8-12% of frame width on portrait 9:16 (approx 96-112px on 1080p).
pub fn calculate_responsive_title_size(
    configured_size: i32,
    video_w: i32,
    video_h: i32,
    raw_text: &str,
    is_rtl: bool,
    rtl_scale: f64,
) -> (i32, String) {
    let video_w = if video_w <= 0 { 1080 } else { video_w };
    let video_h = if video_h <= 0 { 1920 } else { video_h };

    let is_portrait = video_h > video_w;
    let is_landscape = video_w > video_h;

    let base_px = if configured_size > 0 {
        configured_size as f64
    } else {
        (video_w as f64 * 0.092).clamp(72.0, 115.0)
    };

    let aspect_factor = if is_portrait {
        (video_w as f64 / 1080.0).clamp(0.6, 2.0)
    } else if is_landscape {
        (video_h as f64 / 1080.0 * 0.72).clamp(0.5, 1.2)
    } else {
        (video_w as f64 / 1080.0 * 0.85).clamp(0.6, 1.4)
    };

    let base_scaled = base_px * aspect_factor;

    let max_chars = if is_portrait { 22 } else if is_landscape { 38 } else { 26 };
    let wrapped = wrap_title_text(raw_text, max_chars);
    let line_count = wrapped.split("\\N").count();

    let length_multiplier = match line_count {
        1 => 1.04,
        2 => 0.94,
        _ => 0.82,
    };

    let rtl_mult = if is_rtl { rtl_scale.clamp(1.1, 1.4) } else { 1.0 };
    let final_size = (base_scaled * length_multiplier * rtl_mult).round() as i32;
    let clamped_size = final_size.clamp(48, 140);

    (clamped_size, wrapped)
}

pub fn is_placeholder_title(title: &str) -> bool {
    let l = title.trim().to_lowercase();
    l.is_empty()
        || l.starts_with("clip ")
        || l.contains("key moment")
        || l.contains("untitled")
        || l.starts_with("moment ")
        || l == "test render"
}

pub fn generate_ass_file_with_timeline(
    clip: &Clip,
    transcript: &[TranscriptSegment],
    template: &Template,
    is_clip_relative: bool,
    tl: &Timeline,
) -> Result<String> {
    let content = build_ass(clip, transcript, template, is_clip_relative, tl)?;
    let path = generate_ass_path(clip, template);
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Pure string builder (unit-testable).
pub fn build_ass(clip: &Clip, transcript: &[TranscriptSegment], template: &Template, is_clip_relative: bool, tl: &Timeline) -> Result<String> {
    let st = template.ass_style().unwrap_or_default();
    let fg = template.filter_graph().unwrap_or_default();
    let edl = EdlView::parse(clip.timeline_data.as_deref().or(clip.heuristic_json.as_deref()));

    let words_rel = collect_clip_words(clip, transcript, is_clip_relative);
    let has_arabic = contains_arabic(&clip.title) || words_rel.iter().any(|w| contains_arabic(&w.text));
    let rtl = edl.caption_rtl.unwrap_or(has_arabic);
    let font = if rtl { st.rtl_font.clone() } else { st.font.clone() };
    let bold = if st.bold { -1 } else { 0 };
    let size_mul = if rtl { st.rtl_scale.clamp(1.0, 1.8) } else { 1.0 };
    let cap_size = (st.font_size as f64 * size_mul).round() as i32;
    let out_dur = tl.out_duration.max(0.5);

    let raw_title = clean_ass_text(&clip.title, false);
    let opening_words: Vec<String> = words_rel.iter().take(12).map(|w| clean_ass_text(&w.text, false)).collect();
    let is_placeholder = is_placeholder_title(&raw_title);
    let title_is_dup = is_substantially_duplicate(&raw_title, &opening_words);

    let edl_hook = edl.chosen_hook().map(|s| clean_ass_text(s, false));
    let edl_hook_is_dup = edl_hook.as_deref().map(|h| is_substantially_duplicate(h, &opening_words)).unwrap_or(false);

    // Pick non-duplicate title to show (BUG 1 & BUG 3 fix)
    let title_to_show: Option<String> = if !fg.hook && st.title_duration_s <= 0.0 {
        None
    } else if !raw_title.is_empty() && !is_placeholder && !title_is_dup {
        Some(raw_title.clone())
    } else if let Some(h) = edl_hook.filter(|h| !h.is_empty() && !edl_hook_is_dup && !is_placeholder_title(h)) {
        Some(h)
    } else if !raw_title.is_empty() && !title_is_dup && !raw_title.to_lowercase().starts_with("clip ") {
        Some(raw_title.clone())
    } else if !raw_title.is_empty() && title_is_dup {
        // Title duplicated the opening speech! Use a distinct topic hook
        if rtl {
            Some("شاهد للنهاية 🔥".to_string())
        } else {
            Some("WATCH TILL THE END 🔥".to_string())
        }
    } else if !raw_title.is_empty() && is_placeholder {
        if rtl {
            Some("لحظة مميزة 🔥".to_string())
        } else {
            Some("KEY MOMENT 🔥".to_string())
        }
    } else {
        None
    };

    // Calculate prominent, responsive title size (BUG 2 fix: 8-12% of width instead of 44px)
    let (title_size, wrapped_title) = if let Some(ref t) = title_to_show {
        let (sz, wr) = calculate_responsive_title_size(st.title_font_size, OUT_W, OUT_H, t, rtl, size_mul);
        (sz, if st.uppercase && !rtl { wr.to_uppercase() } else { wr })
    } else {
        (96, String::new())
    };

    let title_outline_w = st.title_outline_width.max(10);
    let title_shadow = st.title_shadow.max(3);
    let (title_border, title_outline_col) = if st.title_box {
        (3, style_color(&st.background, 0x30))
    } else {
        (1, style_color(&st.title_outline_color, 0))
    };

    // ── subtitle styles ──
    let box_alpha = 0x55u8;
    let (cap_border, cap_outline_col) = if st.box_style {
        (3, style_color(&st.background, box_alpha))
    } else {
        (1, style_color(&st.outline_color, 0))
    };
    let cap_outline_w = st.outline_width.max(0);

    let mut ass = String::new();
    ass.push_str(&format!(
        "[Script Info]\nTitle: {t}\nScriptType: v4.00+\nWrapStyle: 0\nScaledBorderAndShadow: yes\nYCbCr Matrix: TV.709\nPlayResX: {w}\nPlayResY: {h}\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
        t = raw_title.replace(',', " "),
        w = OUT_W,
        h = OUT_H
    ));
    // Spacing is ALWAYS 0: letter spacing breaks Arabic joining.
    ass.push_str(&format!(
        "Style: Caption,{font},{size},{base},{hl},{outc},{back},{bold},0,0,0,100,100,0,0,{bs},{ow},{sh},5,110,110,0,1\n",
        font = font,
        size = cap_size,
        base = style_color(&st.base_color, 0),
        hl = style_color(&st.highlight_color, 0),
        outc = cap_outline_col,
        back = style_color("#000000", 0x80),
        bold = bold,
        bs = cap_border,
        ow = cap_outline_w,
        sh = st.shadow_offset
    ));
    // Prominent, bold title overlay style (BUG 2 fix)
    ass.push_str(&format!(
        "Style: Title,{font},{size},{base},{base},{outc},{back},{bold},0,0,0,100,100,0,0,{bs},{ow},{sh},5,80,80,0,1\n",
        font = font,
        size = title_size,
        base = style_color(&st.title_color, 0),
        outc = title_outline_col,
        back = style_color("#000000", 0x80),
        bold = -1,
        bs = title_border,
        ow = title_outline_w,
        sh = title_shadow,
    ));
    // Hook alias to Title for backwards compatibility
    ass.push_str(&format!(
        "Style: Hook,{font},{size},{base},{base},{outc},{back},{bold},0,0,0,100,100,0,0,{bs},{ow},{sh},5,80,80,0,1\n\n",
        font = font,
        size = title_size,
        base = style_color(&st.title_color, 0),
        outc = title_outline_col,
        back = style_color("#000000", 0x80),
        bold = -1,
        bs = title_border,
        ow = title_outline_w,
        sh = title_shadow,
    ));

    ass.push_str("[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");

    let x = OUT_W / 2;
    let y_of = |frac: f64| -> i32 { (frac.clamp(0.08, 0.90) * OUT_H as f64).round() as i32 };
    let blur_tag = if st.blur > 0.0 { format!("\\blur{:.1}", st.blur) } else { String::new() };

    // ── Dedicated Title Overlay (prominent, responsive, non-duplicate: BUG 1, 2, 3 fixes) ──
    if let Some(ref _t) = title_to_show {
        if !wrapped_title.is_empty() {
            let title_dur = (st.title_duration_s).clamp(2.0, 5.5).min(out_dur * 0.45);
            let ty = y_of(st.title_y.clamp(0.08, 0.85));
            ass.push_str(&format!(
                "Dialogue: 3,{},{},Title,,0,0,0,,{{\\an5\\pos({},{}){}\\fad(120,240)}}{}\n",
                format_time_ass(0.0),
                format_time_ass(title_dur),
                x,
                ty,
                blur_tag,
                wrapped_title
            ));
        }
    }

    // ── captions (synchronous spoken words, independent from title) ──
    let nudge = ((if edl.sync_nudge_ms != 0 { edl.sync_nudge_ms } else { st.sync_nudge_ms }).clamp(-300, 300)) as f64 / 1000.0;
    let mut mapped: Vec<RelWord> = Vec::new();
    for w in &words_rel {
        let Some(s) = tl.remap(w.start) else { continue }; // word was cut away
        let e = tl.remap_clamped(w.end).max(s + 0.06);
        let txt = clean_ass_text(&w.text, st.uppercase);
        if txt.is_empty() {
            continue;
        }
        mapped.push(RelWord { start: (s + nudge).clamp(0.0, out_dur), end: (e + nudge).clamp(0.0, out_dur), text: txt });
    }
    let max_words = if st.words_per_line == 0 { 3 } else { st.words_per_line };
    let chunks = chunk_words(&mapped, max_words);
    let cy = y_of(st.caption_y.clamp(0.20, 0.84));
    let base_tag = tag_color(&st.base_color);
    let hl_tag = tag_color(&st.highlight_color);

    for (ci, ch) in chunks.iter().enumerate() {
        let next_chunk_start = chunks.get(ci + 1).and_then(|c| c.first()).map(|w| w.start);
        for (i, w) in ch.iter().enumerate() {
            let start = w.start;
            let end = if i + 1 < ch.len() {
                ch[i + 1].start.max(start + 0.05)
            } else {
                let hold = w.end + 0.18;
                let cap = next_chunk_start.map(|n| n - 0.02).unwrap_or(out_dur);
                hold.min(cap).min(out_dur)
            };
            if end <= start + 0.03 {
                continue;
            }
            let mut text = String::new();
            for (j, ww) in ch.iter().enumerate() {
                if j > 0 {
                    text.push(' ');
                }
                let col = if j == i && st.karaoke_style != "none" { &hl_tag } else { &base_tag };
                text.push_str(&format!("{{\\c{}}}{}", col, ww.text));
            }
            let anim = match st.karaoke_style.as_str() {
                "pop_in" if i == 0 => "\\fscx86\\fscy86\\t(0,120,\\fscx100\\fscy100)".to_string(),
                "fade" if i == 0 => "\\fad(90,0)".to_string(),
                _ => String::new(),
            };
            check_malformed_tags(&text);
            ass.push_str(&format!(
                "Dialogue: 1,{},{},Caption,,0,0,0,,{{\\an5\\pos({},{}){}{}}}{}\n",
                format_time_ass(start),
                format_time_ass(end),
                x,
                cy,
                blur_tag,
                anim,
                text
            ));
        }
    }
    Ok(ass)
}

// ───────────────────────────── tests ─────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Word;

    fn clip(title: &str) -> Clip {
        Clip {
            id: "t".into(), project_id: "p".into(), title: title.into(), start_s: 0.0, end_s: 6.0, duration_s: 6.0,
            heuristic_json: None, llm_reason: None, template_id: Some("tiktok".into()), render_path: None,
            status: "ready".into(), hook_segments: None, montage_order: None, hook_score: None, viral_score: None,
            is_clip_relative: true, caption_warning: None, timeline_data: None,
        }
    }
    fn seg(words: &[(&str, f64, f64)]) -> Vec<TranscriptSegment> {
        vec![TranscriptSegment {
            start: 0.0, end: 6.0, text: words.iter().map(|w| w.0).collect::<Vec<_>>().join(" "),
            words: words.iter().map(|w| Word { start: w.1, end: w.2, text: w.0.into(), confidence: 0.9 }).collect(),
        }]
    }
    fn tpl(id: &str) -> Template {
        crate::templates::default_templates().into_iter().find(|t| t.id == id).unwrap()
    }

    #[test]
    fn colors_are_bgr() {
        assert_eq!(style_color("#00F2EA", 0), "&H00EAF200"); // cyan: B=EA G=F2 R=00
        assert_eq!(tag_color("#FFE600"), "&H00E6FF&");
        assert_eq!(tag_color("&HFFFF00"), "&HFFFF00&"); // legacy values keep their meaning
    }

    #[test]
    fn arabic_text_stays_logical_and_clean() {
        let words = [("مرحبا", 0.2, 0.8), ("بكم", 0.9, 1.4), ("في", 1.5, 1.7), ("قناتنا", 1.8, 2.5)];
        let ass = build_ass(&clip("عنوان"), &seg(&words), &tpl("arabic_classic"), true, &Timeline::identity(6.0)).unwrap();
        assert!(!has_stray_backslash(
            &ass.lines().filter(|l| l.starts_with("Dialogue")).map(|l| l.splitn(10, ',').last().unwrap().to_string()).collect::<Vec<_>>().join("\n")
        ));
        let line = ass.lines().find(|l| l.contains("مرحبا") && l.contains("بكم")).expect("first chunk line");
        assert!(line.find("مرحبا").unwrap() < line.find("بكم").unwrap(), "logical order kept");
        assert!(ass.contains("Tajawal ExtraBold"), "Arabic font must be used");
        assert!(!ass.contains("Spacing: 1"));
        assert!(!ass.contains("لاحبرم"), "must not be reversed");
    }

    #[test]
    fn caption_position_comes_from_template() {
        let words = [("hello", 0.2, 0.6), ("world", 0.7, 1.1)];
        let a = build_ass(&clip("Title"), &seg(&words), &tpl("tiktok"), true, &Timeline::identity(6.0)).unwrap();
        let b = build_ass(&clip("Title"), &seg(&words), &tpl("caption_top"), true, &Timeline::identity(6.0)).unwrap();
        assert!(a.contains("\\pos(540,1190)"), "tiktok caption center at 62% of 1920");
        assert!(b.contains("\\pos(540,576)"), "caption_top at 30%");
    }

    #[test]
    fn cuts_remap_caption_times() {
        let words = [("one", 1.0, 1.4), ("two", 4.0, 4.4)];
        let tl = Timeline::from_cuts(6.0, &[(1.8, 3.6)]);
        let ass = build_ass(&clip("T"), &seg(&words), &tpl("minimal"), true, &tl).unwrap();
        // "two" starts at 4.0 in the clip = 4.0 - 1.8 = 2.2 in the edited video
        assert!(ass.contains("0:00:02.20"), "remapped start expected:\n{ass}");
    }

    #[test]
    fn emoji_and_tags_are_stripped() {
        assert_eq!(clean_ass_text("watch {this} \\N now 🔥", true), "WATCH THIS N NOW");
        assert_eq!(clean_ass_text("مرحبا 🔥", true), "مرحبا");
    }

    #[test]
    fn chunking_breaks_on_punctuation_and_pauses() {
        let w = |t: &str, s: f64| RelWord { start: s, end: s + 0.3, text: t.into() };
        let words = vec![w("a", 0.0), w("b.", 0.4), w("c", 0.9), w("d", 2.0), w("e", 2.4)];
        let c = chunk_words(&words, 3);
        assert_eq!(c.iter().map(|x| x.len()).collect::<Vec<_>>(), vec![2, 1, 2]);
    }

    #[test]
    fn time_format_never_rolls_over() {
        assert_eq!(format_time_ass(59.996), "0:01:00.00");
        assert_eq!(format_time_ass(3661.5), "1:01:01.50");
    }

    #[test]
    fn test_title_renders_with_prominent_style_and_separate_captions() {
        let words = [
            ("today", 0.2, 0.6),
            ("we", 0.7, 0.9),
            ("learn", 1.0, 1.4),
            ("editing", 1.5, 2.0),
        ];
        let c = clip("5 Pro Editing Tricks");
        let ass = build_ass(&c, &seg(&words), &tpl("tiktok"), false, &Timeline::identity(6.0)).unwrap();

        // 1. Prominent Title Style exists and has bold, large font size (>= 96px)
        let title_style_line = ass.lines().find(|l| l.starts_with("Style: Title,")).expect("Must define Style: Title");
        let parts: Vec<&str> = title_style_line.split(',').collect();
        let font_size: i32 = parts[2].parse().unwrap();
        assert!(font_size >= 96, "Title font size must be prominent (>= 96px), got {}", font_size);
        let bold_flag = parts[7];
        assert_eq!(bold_flag, "-1", "Title must be bold (-1 in ASS)");

        // 2. Title dialogue event rendered at title_y with smooth fade
        let title_dialogue = ass.lines().find(|l| l.starts_with("Dialogue: 3,") && l.contains(",Title,")).expect("Must emit title dialogue");
        assert!(title_dialogue.contains("\\pos(540,346)"), "Title must be positioned at title_y (18% of 1920)");
        assert!(title_dialogue.contains("\\fad(120,240)"), "Title must have smooth fade in/out");
        assert!(title_dialogue.contains("5 PRO EDITING TRICKS"), "Title must be rendered");

        // 3. Captions dialogue exists separately for spoken words
        let caption_dialogue = ass.lines().find(|l| l.starts_with("Dialogue: 1,") && l.contains(",Caption,")).expect("Must emit caption dialogue");
        assert!(caption_dialogue.contains("TODAY"), "Captions must independently display spoken words");
    }

    #[test]
    fn test_title_duplicate_detection_prevents_verbatim_repetition() {
        let words = [
            ("today", 0.2, 0.6),
            ("i", 0.7, 0.9),
            ("am", 1.0, 1.2),
            ("going", 1.3, 1.6),
            ("to", 1.7, 1.8),
            ("show", 1.9, 2.2),
            ("you", 2.3, 2.5),
        ];

        // Case A: Title duplicates the first spoken words verbatim
        let dup_clip = clip("Today I am going to show you");
        let ass_dup = build_ass(&dup_clip, &seg(&words), &tpl("tiktok"), false, &Timeline::identity(6.0)).unwrap();
        let title_dup_line = ass_dup.lines().find(|l| l.starts_with("Dialogue: 3,") && l.contains(",Title,")).expect("Must emit title");
        assert!(!title_dup_line.contains("TODAY I AM GOING"), "Duplicate title must NOT repeat spoken words");
        assert!(title_dup_line.contains("WATCH TILL") && title_dup_line.contains("THE END"), "Should fall back to curated topic hook, but got: {}", title_dup_line);

        // Case B: Title is distinct topic headline
        let clean_clip = clip("Secret AI Strategy");
        let ass_clean = build_ass(&clean_clip, &seg(&words), &tpl("tiktok"), false, &Timeline::identity(6.0)).unwrap();
        let title_clean_line = ass_clean.lines().find(|l| l.starts_with("Dialogue: 3,") && l.contains(",Title,")).expect("Must emit title");
        assert!(title_clean_line.contains("SECRET AI STRATEGY"), "Distinct title must be preserved");
    }

    #[test]
    fn test_responsive_title_sizing_and_wrapping() {
        // Portrait 9:16 (1080x1920)
        let (p_sz, p_wrap) = calculate_responsive_title_size(98, 1080, 1920, "Short Title", false, 1.0);
        assert!(p_sz >= 96 && p_sz <= 112, "Portrait font size should be 96-112px, got {}", p_sz);
        assert_eq!(p_wrap, "Short Title");

        // Long title wraps into multiple lines with \N
        let (_l_sz, l_wrap) = calculate_responsive_title_size(98, 1080, 1920, "5 Editing Tricks You Absolutely Need To Master Now", false, 1.0);
        assert!(l_wrap.contains("\\N"), "Long title must wrap using \\N: {}", l_wrap);

        // Landscape 16:9 (1920x1080)
        let (land_sz, _) = calculate_responsive_title_size(98, 1920, 1080, "Landscape Title", false, 1.0);
        assert!(land_sz >= 64 && land_sz <= 85, "Landscape title size must scale down to preserve vertical space, got {}", land_sz);

        // Square 1:1 (1080x1080)
        let (sq_sz, _) = calculate_responsive_title_size(98, 1080, 1080, "Square Title", false, 1.0);
        assert!(sq_sz >= 80 && sq_sz <= 94, "Square title size should be 80-94px, got {}", sq_sz);
    }

    #[test]
    fn test_empty_title_does_not_render_phantom_overlay() {
        let words = [("hello", 0.2, 0.6), ("world", 0.7, 1.1)];
        let empty_clip = clip("");
        let ass = build_ass(&empty_clip, &seg(&words), &tpl("tiktok"), false, &Timeline::identity(6.0)).unwrap();
        let has_title_event = ass.lines().any(|l| l.starts_with("Dialogue: 3,") && l.contains(",Title,"));
        assert!(!has_title_event, "Empty title should not render title dialogue");
    }
}
