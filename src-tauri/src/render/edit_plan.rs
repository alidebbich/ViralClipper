//! edit_plan.rs - pure editing logic (NO ffmpeg, NO file IO). Fully unit-tested.
//!
//! Everything here works on the CLIP timeline (seconds from the start of the
//! clip, 0.0 = first frame of the clip) and produces plans in OUTPUT time
//! (after jump-cuts). One `Timeline` is the single source of truth that maps
//! clip time -> output time; video, audio, captions, zooms and sfx all use it,
//! so nothing can drift out of sync after a cut.

use serde_json::Value;

pub const OUT_W: i32 = 1080;
pub const OUT_H: i32 = 1920;

// ───────────────────────── lenient EDL view ─────────────────────────
// We read only what we need from `clip.heuristic_json`, field by field, so
// one malformed field from the LLM never throws away the whole edit plan.

#[derive(Debug, Clone, Default)]
pub struct EffectView {
    pub t: f64,
    pub dur: f64,
    pub kind: String,
    pub intensity: f64,
}

#[derive(Debug, Clone, Default)]
pub struct SfxView {
    pub t: f64,
    pub kind: String,
}

#[derive(Debug, Clone, Default)]
pub struct EdlView {
    pub framing_mode: String,
    /// (t0, t1) in CLIP time
    pub cuts: Vec<(f64, f64)>,
    pub effects: Vec<EffectView>,
    pub sfx: Vec<SfxView>,
    pub hook_options: Vec<String>,
    pub hook_chosen: usize,
    pub caption_font: Option<String>,
    pub caption_rtl: Option<bool>,
    pub caption_position: String,
    pub sync_nudge_ms: i32,
    pub music_file: Option<String>,
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}
fn text(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.to_string())
}
fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

impl EdlView {
    pub fn parse(json: Option<&str>) -> Self {
        let mut out = EdlView::default();
        let Some(raw) = json else { return out };
        let Ok(v) = serde_json::from_str::<Value>(raw) else { return out };

        if let Some(m) = v.get("framing").and_then(|f| f.get("mode")).and_then(text) {
            out.framing_mode = m;
        }
        for c in arr(&v, "cuts") {
            if let (Some(a), Some(b)) = (c.get("t0").and_then(num), c.get("t1").and_then(num)) {
                if b > a {
                    out.cuts.push((a, b));
                }
            }
        }
        for e in arr(&v, "effects") {
            let kind = e.get("type").and_then(text).unwrap_or_default();
            if kind.is_empty() {
                continue;
            }
            out.effects.push(EffectView {
                t: e.get("t").and_then(num).unwrap_or(0.0),
                dur: e.get("dur").and_then(num).unwrap_or(0.0),
                kind,
                intensity: e.get("intensity").and_then(num).unwrap_or(0.0),
            });
        }
        if let Some(a) = v.get("audio") {
            for s in arr(a, "sfx") {
                if let (Some(t), Some(k)) = (s.get("t").and_then(num), s.get("type").and_then(text)) {
                    out.sfx.push(SfxView { t, kind: k });
                }
            }
            out.music_file = a.get("music").and_then(|m| m.get("file")).and_then(text);
        }
        if let Some(h) = v.get("hook") {
            for o in arr(h, "options") {
                if let Some(s) = o.as_str() {
                    if !s.trim().is_empty() {
                        out.hook_options.push(s.trim().to_string());
                    }
                }
            }
            out.hook_chosen = h.get("chosen").and_then(num).map(|x| x.max(0.0) as usize).unwrap_or(0);
        }
        if let Some(c) = v.get("captions") {
            out.caption_font = c.get("font").and_then(text).filter(|s| !s.is_empty());
            out.caption_rtl = c.get("rtl").and_then(|x| x.as_bool());
            out.caption_position = c.get("position").and_then(text).unwrap_or_default();
            out.sync_nudge_ms = c.get("sync_nudge_ms").and_then(num).unwrap_or(0.0) as i32;
        }
        out
    }

    pub fn chosen_hook(&self) -> Option<&str> {
        self.hook_options
            .get(self.hook_chosen)
            .or_else(|| self.hook_options.first())
            .map(|s| s.as_str())
    }
}

// ───────────────────────── timeline (cuts) ─────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub src_start: f64,
    pub src_end: f64,
    pub out_start: f64,
}
impl Piece {
    pub fn len(&self) -> f64 {
        self.src_end - self.src_start
    }
}

#[derive(Debug, Clone)]
pub struct Timeline {
    pub pieces: Vec<Piece>,
    pub src_duration: f64,
    pub out_duration: f64,
}

const MIN_PIECE: f64 = 0.20;

impl Timeline {
    pub fn identity(d: f64) -> Self {
        Timeline {
            pieces: vec![Piece { src_start: 0.0, src_end: d, out_start: 0.0 }],
            src_duration: d,
            out_duration: d,
        }
    }

    /// Build the timeline that REMAINS after removing `cuts` (clip time).
    pub fn from_cuts(src_duration: f64, cuts: &[(f64, f64)]) -> Self {
        let mut c: Vec<(f64, f64)> = cuts
            .iter()
            .map(|&(a, b)| (a.max(0.0), b.min(src_duration)))
            .filter(|&(a, b)| b - a >= 0.05)
            .collect();
        c.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (a, b) in c {
            if let Some(last) = merged.last_mut() {
                if a <= last.1 + 0.02 {
                    last.1 = last.1.max(b);
                    continue;
                }
            }
            merged.push((a, b));
        }
        let mut kept: Vec<(f64, f64)> = Vec::new();
        let mut cur = 0.0;
        for (a, b) in &merged {
            if *a - cur >= MIN_PIECE {
                kept.push((cur, *a));
            }
            cur = *b;
        }
        if src_duration - cur >= MIN_PIECE {
            kept.push((cur, src_duration));
        }
        if kept.is_empty() {
            return Timeline::identity(src_duration);
        }
        let mut pieces = Vec::new();
        let mut out = 0.0;
        for (a, b) in kept {
            pieces.push(Piece { src_start: a, src_end: b, out_start: out });
            out += b - a;
        }
        Timeline { pieces, src_duration, out_duration: out }
    }

    pub fn is_identity(&self) -> bool {
        self.pieces.len() == 1
            && self.pieces[0].src_start <= 0.001
            && (self.pieces[0].src_end - self.src_duration).abs() < 0.001
    }

    /// clip time -> output time. None when the instant was cut away.
    pub fn remap(&self, t: f64) -> Option<f64> {
        for p in &self.pieces {
            if t >= p.src_start - 1e-6 && t <= p.src_end + 1e-6 {
                return Some(p.out_start + (t - p.src_start).clamp(0.0, p.len()));
            }
        }
        None
    }

    /// Like `remap` but a time inside a cut snaps to the start of the next kept piece.
    pub fn remap_clamped(&self, t: f64) -> f64 {
        if let Some(x) = self.remap(t) {
            return x;
        }
        for p in &self.pieces {
            if p.src_start > t {
                return p.out_start;
            }
        }
        self.out_duration
    }

    /// Output times where one kept piece is glued to the next (the visible jump cuts).
    pub fn junctions(&self) -> Vec<f64> {
        self.pieces.iter().skip(1).map(|p| p.out_start).collect()
    }
}

// ───────────────────────── silence / dead-air cuts ─────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct CutRules {
    /// pauses at least this long between words get tightened
    pub min_gap: f64,
    /// breathing room kept on each side of a cut
    pub pad: f64,
    /// silence allowed before the first word
    pub lead_keep: f64,
    /// silence allowed after the last word
    pub tail_keep: f64,
    /// never remove more than this share of the clip
    pub max_ratio: f64,
}
impl Default for CutRules {
    fn default() -> Self {
        CutRules { min_gap: 0.55, pad: 0.14, lead_keep: 0.18, tail_keep: 0.45, max_ratio: 0.30 }
    }
}

/// Decide which silent stretches to remove. `words` are CLIP-time spans, sorted.
pub fn plan_silence_cuts(words: &[Span], clip_dur: f64, r: &CutRules) -> Vec<(f64, f64)> {
    if words.len() < 2 {
        return vec![];
    }
    let mut cuts: Vec<(f64, f64)> = Vec::new();
    let first = words.first().unwrap();
    if first.start - r.lead_keep >= 0.30 {
        cuts.push((0.0, first.start - r.lead_keep));
    }
    for w in words.windows(2) {
        let gap = w[1].start - w[0].end;
        if gap >= r.min_gap {
            let (s, e) = (w[0].end + r.pad, w[1].start - r.pad);
            if e - s >= 0.20 {
                cuts.push((s, e));
            }
        }
    }
    let last = words.last().unwrap();
    if clip_dur - last.end - r.tail_keep >= 0.30 {
        cuts.push((last.end + r.tail_keep, clip_dur));
    }
    let max_total = clip_dur * r.max_ratio;
    let total: f64 = cuts.iter().map(|c| c.1 - c.0).sum();
    if total > max_total {
        cuts.sort_by(|a, b| (b.1 - b.0).partial_cmp(&(a.1 - a.0)).unwrap());
        let mut kept = Vec::new();
        let mut acc = 0.0;
        for c in cuts {
            if acc + (c.1 - c.0) <= max_total {
                acc += c.1 - c.0;
                kept.push(c);
            }
        }
        cuts = kept;
    }
    cuts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    cuts
}

// ───────────────────────── zoom punches ─────────────────────────

#[derive(Debug, Clone)]
pub struct Punch {
    pub t0: f64,
    pub dur: f64,
    pub amount: f64,
    pub attack: f64,
    pub release: f64,
}

fn is_zoom_kind(k: &str) -> bool {
    matches!(k, "punch_in" | "zoom" | "zoom_ramp" | "push_in" | "zoom_in")
}

/// Zoom plan in OUTPUT time. `words_out` = word spans in output time.
/// `amount_scale` comes from the template (1.0 = normal, 1.3 = aggressive).
pub fn plan_punches(edl: &EdlView, tl: &Timeline, words_out: &[Span], enabled: bool, amount_scale: f64) -> Vec<Punch> {
    if !enabled {
        return vec![];
    }
    let out_dur = tl.out_duration;
    let mut ps: Vec<Punch> = Vec::new();

    for e in edl.effects.iter().filter(|e| is_zoom_kind(&e.kind)) {
        let t0 = tl.remap_clamped(e.t);
        let dur = if e.dur > 0.4 { e.dur.min(3.5) } else { 1.8 };
        let amount = (0.06 + 0.14 * e.intensity.clamp(0.0, 1.0)) * amount_scale;
        ps.push(Punch { t0, dur, amount: amount.clamp(0.05, 0.24), attack: 0.16, release: 0.28 });
    }

    if ps.len() < 2 {
        // automatic rhythm: hook punch + a push on phrase starts
        let hook_dur = 2.2f64.min(out_dur * 0.35);
        if hook_dur > 0.8 && !ps.iter().any(|p| p.t0 < 1.0) {
            ps.push(Punch { t0: 0.0, dur: hook_dur, amount: 0.10 * amount_scale, attack: 0.12, release: 0.35 });
        }
        let amounts = [0.13, 0.09, 0.16];
        let mut n = 0usize;
        let mut last_end = ps.iter().map(|p| p.t0 + p.dur).fold(0.0, f64::max);
        for (i, w) in words_out.iter().enumerate() {
            let phrase_start = i == 0 || (w.start - words_out[i - 1].end) >= 0.25;
            if !phrase_start || w.start < last_end + 2.6 {
                continue;
            }
            let dur = 2.0f64.min((out_dur - w.start - 0.3).max(0.0));
            if dur < 0.9 {
                continue;
            }
            ps.push(Punch {
                t0: w.start,
                dur,
                amount: amounts[n % amounts.len()] * amount_scale,
                attack: 0.14,
                release: 0.30,
            });
            n += 1;
            last_end = w.start + dur;
        }
    }

    ps.sort_by(|a, b| a.t0.partial_cmp(&b.t0).unwrap());
    let mut out: Vec<Punch> = Vec::new();
    for p in ps {
        if p.t0 + p.dur > out_dur - 0.15 {
            let d = out_dur - 0.15 - p.t0;
            if d < 0.6 {
                continue;
            }
            let mut q = p;
            q.dur = d;
            out.push(q);
            continue;
        }
        if let Some(prev) = out.last() {
            if p.t0 < prev.t0 + prev.dur + 0.35 {
                continue;
            }
        }
        out.push(p);
    }
    out
}

/// Hard "alternating zoom" on every other piece. Hides jump cuts like pro talking-head edits.
pub fn plan_step_zooms(tl: &Timeline, amount: f64) -> Vec<Punch> {
    tl.pieces
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, p)| {
            let out_end = p.out_start + p.len();
            Punch { t0: p.out_start, dur: out_end - p.out_start, amount, attack: 0.001, release: 0.001 }
        })
        .collect()
}

/// ffmpeg expression (variable `t`) for the zoom factor, 1.0 = no zoom.
pub fn zoom_expr(punches: &[Punch]) -> String {
    if punches.is_empty() {
        return "1".to_string();
    }
    let smooth = |u: &str| format!("({u}*{u}*(3-2*{u}))");
    let terms: Vec<String> = punches
        .iter()
        .map(|p| {
            let a = p.attack.max(0.001);
            let r = p.release.max(0.001);
            let u_in = format!("clip((t-{:.3})/{:.3},0,1)", p.t0, a);
            let u_out = format!("clip(({:.3}-t)/{:.3},0,1)", p.t0 + p.dur, r);
            format!("{:.4}*min({},{})", p.amount, smooth(&u_in), smooth(&u_out))
        })
        .collect();
    format!("min(1.32,1+{})", terms.join("+"))
}

// ───────────────────────── face-follow pan ─────────────────────────

/// Clean raw face-center samples (t, cx in 0..1) into a few smooth keyframes.
/// The camera HOLDS still and only glides when the person really moves.
pub fn smooth_pan_keys(raw: &[(f64, f64)], max_points: usize, deadzone: f64) -> Vec<(f64, f64)> {
    let mut r: Vec<(f64, f64)> = raw.iter().copied().filter(|(t, c)| t.is_finite() && c.is_finite()).collect();
    if r.is_empty() {
        return vec![];
    }
    r.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    // median of 5 kills detector outliers
    let med: Vec<f64> = (0..r.len())
        .map(|i| {
            let lo = i.saturating_sub(2);
            let hi = (i + 3).min(r.len());
            let mut w: Vec<f64> = r[lo..hi].iter().map(|x| x.1).collect();
            w.sort_by(|a, b| a.partial_cmp(b).unwrap());
            w[w.len() / 2]
        })
        .collect();
    // exponential smoothing
    let mut sm = Vec::with_capacity(med.len());
    let mut prev = med[0];
    for m in &med {
        prev = prev * 0.6 + m * 0.4;
        sm.push(prev);
    }
    let mut keys: Vec<(f64, f64)> = vec![(r[0].0, sm[0])];
    let mut held = sm[0];
    for i in 1..r.len() {
        if (sm[i] - held).abs() >= deadzone {
            let glide_start = (r[i].0 - 0.5).max(keys.last().unwrap().0 + 0.001);
            if glide_start < r[i].0 {
                keys.push((glide_start, held));
            }
            keys.push((r[i].0, sm[i]));
            held = sm[i];
        }
    }
    let end_t = r.last().unwrap().0;
    if end_t > keys.last().unwrap().0 + 0.001 {
        keys.push((end_t, held));
    }
    if keys.len() > max_points && max_points >= 3 {
        let step = (keys.len() as f64 - 1.0) / (max_points as f64 - 1.0);
        keys = (0..max_points).map(|i| keys[((i as f64) * step).round() as usize]).collect();
    }
    keys
}

/// ffmpeg expression (variable `t`) returning the normalized horizontal face center 0..1.
pub fn pan_expr(keys: &[(f64, f64)]) -> String {
    match keys.len() {
        0 => "0.5".to_string(),
        1 => format!("{:.4}", keys[0].1),
        n => {
            let mut e = format!("{:.4}", keys[n - 1].1);
            for i in (0..n - 1).rev() {
                let (t0, c0) = keys[i];
                let (t1, c1) = keys[i + 1];
                let seg = if t1 - t0 < 1e-3 {
                    format!("{:.4}", c1)
                } else {
                    format!("{:.4}+({:.4})*(t-{:.3})/{:.3}", c0, c1 - c0, t0, t1 - t0)
                };
                e = format!("if(lt(t,{:.3}),{},{})", t1, seg, e);
            }
            format!("if(lt(t,{:.3}),{:.4},{})", keys[0].0, keys[0].1, e)
        }
    }
}

// ───────────────────────── framing choice ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// whole picture visible, blurred copy fills the rest
    BlurFit,
    /// 9:16 crop that follows the face (or stays centered when no face keys)
    FaceCrop,
    /// speaker on top, background clip below
    Split,
}

#[derive(Debug, Clone, Default)]
pub struct FaceStats {
    pub keys: Vec<(f64, f64)>,
    pub found_ratio: f64,
    pub spread_ratio: f64,
}

pub fn choose_framing(mode: &str, template_default: &str, src_w: i32, src_h: i32, face: &FaceStats, has_bg: bool) -> Framing {
    if src_h >= src_w {
        return Framing::FaceCrop; // vertical source: fill the screen
    }
    // A face track is usable if we found a dominant face in at least 25% of sampled frames.
    // We don't discard face crop just because another person or picture appears in frame (spread_ratio),
    // because face_track.py already locks onto the primary speaking face.
    let faces_ok = face.found_ratio >= 0.25 && !face.keys.is_empty();
    let pick = |m: &str| -> Option<Framing> {
        match m {
            "blur_fit" | "framed_blur" | "blur_fill" | "keep_screen" | "letterbox" => Some(Framing::BlurFit),
            "face_crop" | "smart_crop" | "full_zoom" | "center_crop" => Some(Framing::FaceCrop),
            "split" | "split_stack" | "facecam_split" => Some(if has_bg { Framing::Split } else { Framing::FaceCrop }),
            _ => None,
        }
    };
    // the template decides unless the Editor Brain asked for something specific AND it is safe
    if let Some(f) = pick(mode) {
        if f == Framing::FaceCrop && !faces_ok && face.found_ratio < 0.15 {
            return Framing::BlurFit; // asked for crop but nobody is in frame
        }
        return f;
    }
    if let Some(f) = pick(template_default) {
        return f;
    }
    if faces_ok {
        Framing::FaceCrop
    } else {
        Framing::BlurFit
    }
}

// ───────────────────────── sound effects ─────────────────────────

#[derive(Debug, Clone)]
pub struct SfxEvent {
    pub t: f64,
    pub kind: String,
    pub gain: f64,
}

fn norm_sfx(k: &str) -> Option<&'static str> {
    match k {
        "whoosh" | "swoosh" | "transition" => Some("whoosh"),
        "pop" | "click" | "tick" => Some("pop"),
        "hit" | "impact" | "boom" | "thud" => Some("hit"),
        "ding" | "chime" | "success" => Some("ding"),
        _ => None,
    }
}

pub fn plan_sfx(edl: &EdlView, tl: &Timeline, punches: &[Punch], enabled: bool) -> Vec<SfxEvent> {
    if !enabled {
        return vec![];
    }
    let mut ev: Vec<SfxEvent> = Vec::new();
    for s in &edl.sfx {
        if let Some(k) = norm_sfx(&s.kind) {
            ev.push(SfxEvent { t: tl.remap_clamped(s.t), kind: k.into(), gain: 0.55 });
        }
    }
    if ev.is_empty() {
        ev.push(SfxEvent { t: 0.06, kind: "pop".into(), gain: 0.5 });
        for (i, j) in tl.junctions().into_iter().enumerate() {
            if i % 2 == 0 {
                ev.push(SfxEvent { t: (j - 0.06).max(0.0), kind: "whoosh".into(), gain: 0.5 });
            }
        }
        for p in punches.iter().filter(|p| p.attack > 0.01 && p.amount >= 0.12 && p.t0 > 0.5) {
            ev.push(SfxEvent { t: p.t0, kind: "hit".into(), gain: 0.5 });
        }
    }
    ev.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    let cap = ((tl.out_duration / 3.0).ceil() as usize).max(2);
    let mut out: Vec<SfxEvent> = Vec::new();
    for e in ev {
        if e.t > tl.out_duration - 0.2 {
            continue;
        }
        if let Some(l) = out.last() {
            if e.t - l.t < 0.8 {
                continue;
            }
        }
        out.push(e);
        if out.len() >= cap {
            break;
        }
    }
    out
}

/// An LLM-suggested cut may start or end INSIDE a spoken word. Shrink it to the nearest word gap so no
/// word is ever clipped in half. Returns None when nothing useful remains.
pub fn snap_cut_to_words(cut: (f64, f64), words: &[Span]) -> Option<(f64, f64)> {
    let (mut a, mut b) = cut;
    for w in words {
        if w.start < a && a < w.end {
            a = w.end;
        }
        if w.start < b && b < w.end {
            b = w.start;
        }
    }
    if b - a >= 0.15 {
        Some((a, b))
    } else {
        None
    }
}

// ───────────────────────────── tests ─────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn sp(a: f64, b: f64) -> Span {
        Span { start: a, end: b }
    }

    #[test]
    fn timeline_remap_after_cuts() {
        let tl = Timeline::from_cuts(20.0, &[(2.0, 3.0), (10.0, 12.0)]);
        assert_eq!(tl.pieces.len(), 3);
        assert!((tl.out_duration - 17.0).abs() < 1e-9);
        assert!((tl.remap(1.0).unwrap() - 1.0).abs() < 1e-9);
        assert!(tl.remap(2.5).is_none());
        assert!((tl.remap(5.0).unwrap() - 4.0).abs() < 1e-9);
        assert!((tl.remap(15.0).unwrap() - 12.0).abs() < 1e-9);
        assert!((tl.remap_clamped(2.5) - 2.0).abs() < 1e-9);
        assert_eq!(tl.junctions().len(), 2);
    }

    #[test]
    fn timeline_merges_overlapping_cuts_and_drops_tiny_pieces() {
        let tl = Timeline::from_cuts(10.0, &[(1.0, 3.0), (2.5, 4.0), (4.1, 6.0)]);
        // 4.0..4.1 is a 0.1 s sliver -> dropped, so everything 1.0..6.0 is gone
        assert_eq!(tl.pieces.len(), 2);
        assert!((tl.out_duration - 5.0).abs() < 1e-9);
        assert!(Timeline::from_cuts(10.0, &[(0.0, 10.0)]).is_identity());
    }

    #[test]
    fn silence_cuts_keep_breathing_room() {
        let words = vec![sp(0.9, 1.2), sp(1.3, 1.6), sp(3.0, 3.4), sp(3.5, 3.9)];
        let cuts = plan_silence_cuts(&words, 8.0, &CutRules::default());
        // lead: 0.0..0.72, gap 1.6..3.0 -> 1.74..2.86, tail 3.9+0.45=4.35..8.0 (ratio-limited)
        assert!(cuts.iter().any(|c| (c.0 - 1.74).abs() < 0.01 && (c.1 - 2.86).abs() < 0.01));
        let total: f64 = cuts.iter().map(|c| c.1 - c.0).sum();
        assert!(total <= 8.0 * 0.30 + 1e-9, "ratio limit respected, got {total}");
        for c in &cuts {
            for w in &words {
                assert!(c.1 <= w.start + 1e-9 || c.0 >= w.end - 1e-9, "a cut must never overlap a word");
            }
        }
    }

    #[test]
    fn zoom_expr_is_well_formed() {
        assert_eq!(zoom_expr(&[]), "1");
        let e = zoom_expr(&[Punch { t0: 1.0, dur: 2.0, amount: 0.12, attack: 0.15, release: 0.3 }]);
        assert!(e.starts_with("min(1.32,1+0.1200*min("));
        assert_eq!(e.matches('(').count(), e.matches(')').count());
        assert!(!e.contains('\''));
    }

    #[test]
    fn pan_expr_interpolates() {
        let e = pan_expr(&[(0.0, 0.4), (2.0, 0.6)]);
        assert_eq!(e.matches('(').count(), e.matches(')').count());
        assert!(e.contains("0.4000+(0.2000)*(t-0.000)/2.000"));
        assert_eq!(pan_expr(&[]), "0.5");
    }

    #[test]
    fn pan_keys_hold_then_glide() {
        let mut raw = vec![];
        for i in 0..30 {
            raw.push((i as f64 * 0.33, if i < 15 { 0.40 } else { 0.62 }));
        }
        let k = smooth_pan_keys(&raw, 12, 0.04);
        assert!(k.len() >= 2 && k.len() <= 12);
        assert!((k[0].1 - 0.40).abs() < 0.02);
        assert!(k.last().unwrap().1 > 0.5);
        assert!(k.windows(2).all(|w| w[1].0 >= w[0].0));
    }

    #[test]
    fn auto_punches_do_not_overlap() {
        let tl = Timeline::identity(30.0);
        let words: Vec<Span> = (0..40).map(|i| sp(i as f64 * 0.7, i as f64 * 0.7 + 0.4)).collect();
        let ps = plan_punches(&EdlView::default(), &tl, &words, true, 1.0);
        assert!(ps.len() >= 3, "expected several punches, got {}", ps.len());
        for w in ps.windows(2) {
            assert!(w[1].t0 >= w[0].t0 + w[0].dur);
        }
        assert!(plan_punches(&EdlView::default(), &tl, &words, false, 1.0).is_empty());
    }

    #[test]
    fn edl_parse_is_lenient() {
        let j = r#"{"framing":{"mode":"smart_crop"},"cuts":[{"t0":"1.5","t1":2.5},{"t0":"x"}],
            "effects":[{"type":"punch_in","t":3,"dur":2,"intensity":0.8},{"type":""}],
            "hook":{"options":["a","b"],"chosen":1},"audio":{"sfx":[{"t":1,"type":"whoosh"}]},
            "captions":{"rtl":true,"sync_nudge_ms":40}}"#;
        let e = EdlView::parse(Some(j));
        assert_eq!(e.framing_mode, "smart_crop");
        assert_eq!(e.cuts, vec![(1.5, 2.5)]);
        assert_eq!(e.effects.len(), 1);
        assert_eq!(e.chosen_hook(), Some("b"));
        assert_eq!(e.sfx.len(), 1);
        assert_eq!(e.caption_rtl, Some(true));
        assert_eq!(EdlView::parse(Some("not json")).cuts.len(), 0);
    }

    #[test]
    fn llm_cuts_never_split_a_word() {
        let words = vec![sp(1.0, 1.5), sp(2.0, 2.6)];
        // starts inside word 1, ends inside word 2 -> shrunk to the gap between them
        let c = snap_cut_to_words((1.2, 2.3), &words).unwrap();
        assert!((c.0 - 1.5).abs() < 1e-9 && (c.1 - 2.0).abs() < 1e-9);
        assert!(snap_cut_to_words((1.1, 1.3), &words).is_none());
    }

    #[test]
    fn framing_decisions() {
        let none = FaceStats::default();
        let good = FaceStats { keys: vec![(0.0, 0.5)], found_ratio: 0.9, spread_ratio: 0.0 };
        assert_eq!(choose_framing("", "auto", 1920, 1080, &good, false), Framing::FaceCrop);
        assert_eq!(choose_framing("", "auto", 1920, 1080, &none, false), Framing::BlurFit);
        assert_eq!(choose_framing("", "auto", 1080, 1920, &none, false), Framing::FaceCrop);
        assert_eq!(choose_framing("", "blur_fit", 1920, 1080, &good, false), Framing::BlurFit);
        assert_eq!(choose_framing("smart_crop", "auto", 1920, 1080, &none, false), Framing::BlurFit);
        assert_eq!(choose_framing("", "split", 1920, 1080, &good, true), Framing::Split);
    }
}
