//! presets.rs - builds the ffmpeg filter graph for one clip from an edit plan.
//!
//! WHY THE OLD ZOOM NEVER WORKED: ffmpeg's `crop` filter evaluates its `w`/`h` only ONCE at start,
//! so `between(t,...)` inside them is always 0 and the zoom factor stayed 1.0 for the whole video.
//! Here the zoom is done with `scale=...:eval=frame`, which is re-evaluated for every frame.

use anyhow::Result;

use super::edit_plan::*;
use crate::common::Clip;
use crate::templates::Template;

#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub w: i32,
    pub h: i32,
    pub has_audio: bool,
}

#[derive(Debug, Clone)]
pub struct ExtraInput {
    pub path: String,
    pub loop_forever: bool,
}

#[derive(Debug, Clone)]
pub struct EditGraph {
    pub filter_complex: String,
    /// inputs 1.. (input 0 is always the source clip)
    pub extra_inputs: Vec<ExtraInput>,
    pub map_v: String,
    pub map_a: Option<String>,
    pub out_duration: f64,
}

pub struct EditParams<'a> {
    pub template: &'a Template,
    pub ass_path: &'a str,
    pub fonts_dir: &'a str,
    pub src: SourceInfo,
    pub timeline: &'a Timeline,
    pub framing: Framing,
    pub pan_keys: &'a [(f64, f64)],
    pub punches: &'a [Punch],
    pub sfx: &'a [SfxEvent],
    /// (kind, file) for whoosh/pop/hit/ding
    pub sfx_files: &'a [(String, String)],
    pub bg_video: Option<&'a str>,
    pub music: Option<&'a str>,
}

fn esc_path(p: &str) -> String {
    p.replace('\\', "/").replace(':', "\\:")
}

fn even(v: f64) -> i32 {
    ((v / 2.0).round() as i32) * 2
}

/// Zoom the current stream by z(t) and crop back to the original size (center, slightly up for headroom).
fn zoom_stage(z: &str, w: i32, h: i32, label_in: &str, label_out: &str) -> String {
    if z == "1" {
        return format!("[{label_in}]null[{label_out}];");
    }
    format!(
        "[{label_in}]scale=w='trunc(iw*({z})/2)*2':h='trunc(ih*({z})/2)*2':eval=frame:flags=bicubic,\
crop={w}:{h}:x='(iw-ow)/2':y='(ih-oh)*0.40'[{label_out}];"
    )
}

pub fn build_edit_graph(p: &EditParams) -> Result<EditGraph> {
    let fg = p.template.filter_graph().unwrap_or_default();
    let theme = p.template.theme().unwrap_or_default();
    let tl = p.timeline;
    let out_dur = tl.out_duration;
    let has_a = p.src.has_audio;
    let mut g = String::new();
    let mut extra: Vec<ExtraInput> = Vec::new();

    // ── 1. jump cuts (video + audio) ──
    if tl.is_identity() {
        g.push_str("[0:v]fps=30,setpts=PTS-STARTPTS[vc];");
        if has_a {
            g.push_str("[0:a]aresample=48000,asetpts=PTS-STARTPTS[ac];");
        }
    } else {
        let mut ins = String::new();
        for (i, pc) in tl.pieces.iter().enumerate() {
            g.push_str(&format!(
                "[0:v]trim=start={:.3}:end={:.3},setpts=PTS-STARTPTS[v{i}];",
                pc.src_start, pc.src_end
            ));
            ins.push_str(&format!("[v{i}]"));
            if has_a {
                let len = pc.len();
                g.push_str(&format!(
                    "[0:a]atrim=start={:.3}:end={:.3},asetpts=PTS-STARTPTS,aresample=48000,afade=t=in:st=0:d=0.008,afade=t=out:st={:.3}:d=0.008[a{i}];",
                    pc.src_start,
                    pc.src_end,
                    (len - 0.008).max(0.0)
                ));
                ins.push_str(&format!("[a{i}]"));
            }
        }
        let n = tl.pieces.len();
        if has_a {
            g.push_str(&format!("{ins}concat=n={n}:v=1:a=1[vcat][ac];"));
        } else {
            g.push_str(&format!("{ins}concat=n={n}:v=1:a=0[vcat];"));
        }
        g.push_str("[vcat]fps=30[vc];");
    }

    // ── 2. framing + zoom ──
    let z = zoom_expr(p.punches);
    let landscape = p.src.w > p.src.h;
    let cx = pan_expr(p.pan_keys);
    match p.framing {
        Framing::BlurFit => {
            // fg_h: height of the foreground when scaled to 1080px wide (preserves aspect).
            // For a 16:9 source: 1080 * 1080/1920 = 607px — only 31% of portrait height.
            // We don't stretch it, but we DO want it to look good in portrait.
            // Strategy:
            //   • Foreground: scale to 1080 wide (keeps aspect ratio), then use a second
            //     scale pass to fill at least 60% of portrait height (min 1152px), capped
            //     at 1440px so it never exceeds the canvas.  Width is re-derived from height
            //     to keep the ratio exact; any pillarbox is cropped centered.
            //   • Background: use a 540×960 intermediate before blurring (5× better quality
            //     than the old 108px), then scale to 1080×1920.
            let fg_h_natural = even(1080.0 * p.src.h as f64 / p.src.w as f64).max(2);
            // Target foreground height: natural, but at least 60% of 1920 and at most 75%.
            let fg_h_target = fg_h_natural.max(even(1920.0 * 0.60)).min(even(1920.0 * 0.75));
            // Derived width at that target height (keep source aspect ratio).
            let fg_w_at_target = even(fg_h_target as f64 * p.src.w as f64 / p.src.h as f64);
            // If the width fits inside 1080 we just pad; if wider we crop centered.
            let crop_w = fg_w_at_target.min(1080);
            let crop_x = (fg_w_at_target - crop_w) / 2; // 0 when fit; positive when wider

            g.push_str("[vc]split=2[bg0][fg0];");
            // High-quality blurred background: downsample to 540×960 (not 108px) before blur.
            g.push_str(
                "[bg0]scale=540:960:force_original_aspect_ratio=increase:flags=lanczos,\
crop=540:960,boxblur=12:3,scale=1080:1920:flags=bilinear,\
eq=brightness=-0.18:saturation=0.55:contrast=0.85[bgb];",
            );
            // Foreground: scale to target height, crop to output width.
            g.push_str(&format!(
                "[fg0]scale={fg_w_at_target}:{fg_h_target}:flags=lanczos,\
crop={crop_w}:{fg_h_target}:{crop_x}:0[fgs];"
            ));
            g.push_str(&zoom_stage(&z, crop_w, fg_h_target, "fgs", "fgz"));
            // Overlay foreground centered vertically on the blurred background.
            let oy = (1920 - fg_h_target) / 2;
            let ox = (1080 - crop_w) / 2;
            g.push_str(&format!("[bgb][fgz]overlay={ox}:{oy}[vf];"));
        }
        Framing::FaceCrop => {
            if landscape {
                g.push_str("[vc]scale=-2:1920:flags=lanczos[big];");
                g.push_str(&format!(
                    "[big]crop=1080:1920:x='clip(({cx})*iw-ow/2,0,iw-ow)':y=0[fr];"
                ));
            } else {
                g.push_str("[vc]scale=1080:1920:force_original_aspect_ratio=increase:flags=lanczos,crop=1080:1920[fr];");
            }
            g.push_str(&zoom_stage(&z, 1080, 1920, "fr", "vf"));
        }
        Framing::Split => {
            let bg_idx = p.bg_video.map(|b| {
                extra.push(ExtraInput { path: b.to_string(), loop_forever: true });
                extra.len()
            });
            if landscape {
                g.push_str("[vc]scale=-2:960:flags=lanczos[bigt];");
                g.push_str(&format!(
                    "[bigt]crop=1080:960:x='clip(({cx})*iw-ow/2,0,iw-ow)':y=0[frt];"
                ));
            } else {
                g.push_str("[vc]scale=1080:960:force_original_aspect_ratio=increase:flags=lanczos,crop=1080:960[frt];");
            }
            g.push_str(&zoom_stage(&z, 1080, 960, "frt", "top"));
            match bg_idx {
                Some(i) => g.push_str(&format!(
                    "[{i}:v]scale=1080:960:force_original_aspect_ratio=increase,crop=1080:960,fps=30,setpts=PTS-STARTPTS[bot];"
                )),
                None => g.push_str("color=c=0x111111:s=1080x960:r=30[bot];"),
            }
            g.push_str("[top][bot]vstack=inputs=2:shortest=1[vf];");
        }
    }

    // ── 3. look: gentle grade + sharpen (the crop is upscaled, this brings back bite) ──
    g.push_str("[vf]eq=saturation=1.12:contrast=1.05:brightness=0.01,unsharp=5:5:0.45:5:5:0.0[vp];");

    // ── 4. optional progress bar (real time-based animation via overlay x) ──
    let mut last = "vp".to_string();
    if fg.progress_bar && out_dur > 1.0 {
        let hex = theme.primary.trim_start_matches('#');
        g.push_str(&format!("color=c=0x{hex}:s=1080x8:r=30[bar];"));
        g.push_str(&format!(
            "[vp][bar]overlay=x='-w+w*min(1,t/{:.3})':y=1912:shortest=1[vpb];",
            out_dur
        ));
        last = "vpb".to_string();
    }

    // ── 5. captions / hook / title (ASS) ──
    g.push_str(&format!(
        "[{last}]ass=filename='{}':fontsdir='{}':shaping=complex[vout];",
        esc_path(p.ass_path),
        esc_path(p.fonts_dir)
    ));

    // ── 6. audio: voice, loudness, music bed with ducking, sfx ──
    let mut map_a: Option<String> = None;
    if has_a {
        g.push_str(
            "[ac]aformat=sample_rates=48000:channel_layouts=stereo,highpass=f=70,loudnorm=I=-14:LRA=9:TP=-1.5,\
aformat=sample_rates=48000:channel_layouts=stereo[va];",
        );
        let mut labels: Vec<String> = vec!["[vm]".into()];
        let music_idx = if fg.music_duck {
            p.music.map(|m| {
                extra.push(ExtraInput { path: m.to_string(), loop_forever: true });
                extra.len()
            })
        } else {
            None
        };
        if let Some(mi) = music_idx {
            g.push_str("[va]asplit=2[vm][vs];");
            g.push_str(&format!(
                "[{mi}:a]aformat=sample_rates=48000:channel_layouts=stereo,volume=0.22,afade=t=in:st=0:d=0.8[mu];\
[mu][vs]sidechaincompress=threshold=0.03:ratio=12:attack=15:release=320:makeup=1[mud];"
            ));
            labels.push("[mud]".into());
        } else {
            g.push_str("[va]anull[vm];");
        }
        // sfx: one input per kind, split per use
        let mut n_sfx = 0usize;
        let mut kinds: Vec<&str> = p.sfx.iter().map(|e| e.kind.as_str()).collect();
        kinds.sort();
        kinds.dedup();
        for kind in kinds {
            let Some((_, file)) = p.sfx_files.iter().find(|(k, _)| k == kind) else { continue };
            let evs: Vec<&SfxEvent> = p.sfx.iter().filter(|e| e.kind == kind).collect();
            extra.push(ExtraInput { path: file.clone(), loop_forever: false });
            let idx = extra.len();
            let fmt = "aformat=sample_rates=48000:channel_layouts=stereo";
            let srcs: Vec<String> = if evs.len() > 1 {
                let outs: Vec<String> = (0..evs.len()).map(|i| format!("[s{idx}_{i}]")).collect();
                g.push_str(&format!("[{idx}:a]{fmt},asplit={}{};", evs.len(), outs.join("")));
                outs
            } else {
                g.push_str(&format!("[{idx}:a]{fmt}[s{idx}_0];"));
                vec![format!("[s{idx}_0]")]
            };
            for (e, s) in evs.iter().zip(srcs.iter()) {
                let ms = (e.t * 1000.0).round().max(0.0) as i64;
                g.push_str(&format!("{s}adelay={ms}|{ms},volume={:.2}[x{n_sfx}];", e.gain));
                labels.push(format!("[x{n_sfx}]"));
                n_sfx += 1;
            }
        }
        if labels.len() == 1 {
            g.push_str("[vm]alimiter=limit=0.89[aout];");
        } else {
            g.push_str(&format!(
                "{}amix=inputs={}:duration=first:normalize=0,alimiter=limit=0.89[aout];",
                labels.join(""),
                labels.len()
            ));
        }
        map_a = Some("[aout]".into());
    }

    let filter_complex = g.trim_end_matches(';').to_string();
    Ok(EditGraph { filter_complex, extra_inputs: extra, map_v: "[vout]".into(), map_a, out_duration: out_dur })
}

/// Backward-compatible entry point used by older callers/bins: video-only graph, supports cuts and punches.
pub fn build_filter_graph(clip: &Clip, template: &Template, ass_path: &str) -> Result<String> {
    let edl = EdlView::parse(clip.timeline_data.as_deref().or(clip.heuristic_json.as_deref()));
    let fg = template.filter_graph().unwrap_or_default();
    let tl = if fg.jump_cuts && !edl.cuts.is_empty() {
        Timeline::from_cuts(clip.duration_s, &edl.cuts)
    } else {
        Timeline::identity(clip.duration_s)
    };
    let punches = plan_punches(&edl, &tl, &[], fg.zoom_punch, fg.zoom_amount);
    let params = EditParams {
        template,
        ass_path,
        fonts_dir: "C:/Windows/Fonts",
        src: SourceInfo { w: 1920, h: 1080, has_audio: false },
        timeline: &tl,
        framing: Framing::BlurFit,
        pan_keys: &[],
        punches: &punches,
        sfx: &[],
        sfx_files: &[],
        bg_video: None,
        music: None,
    };
    Ok(build_edit_graph(&params)?.filter_complex)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tpl() -> Template {
        crate::templates::default_templates().remove(0)
    }

    fn params<'a>(t: &'a Template, tl: &'a Timeline, punches: &'a [Punch], framing: Framing) -> EditParams<'a> {
        EditParams {
            template: t, ass_path: "C:\\Users\\x\\a.ass", fonts_dir: "C:/fonts", src: SourceInfo { w: 1920, h: 1080, has_audio: true },
            timeline: tl, framing, pan_keys: &[(0.0, 0.4), (3.0, 0.6)], punches, sfx: &[], sfx_files: &[], bg_video: None, music: None,
        }
    }

    #[test]
    fn zoom_is_per_frame_never_inside_crop_size() {
        let t = tpl();
        let tl = Timeline::identity(10.0);
        let punches = vec![Punch { t0: 1.0, dur: 2.0, amount: 0.12, attack: 0.15, release: 0.3 }];
        let g = build_edit_graph(&params(&t, &tl, &punches, Framing::FaceCrop)).unwrap().filter_complex;
        assert!(g.contains("eval=frame"));
        // the old bug: a time-based expression in crop's w/h
        for part in g.split("crop=") {
            let head: String = part.chars().take(24).collect();
            assert!(!head.contains("between(t"), "time-based expr inside crop size: {head}");
        }
        assert!(g.contains("ass=filename='C\\:/Users/x/a.ass'"));
        assert!(g.contains("shaping=complex"));
    }

    #[test]
    fn cuts_build_trim_concat_for_video_and_audio() {
        let t = tpl();
        let tl = Timeline::from_cuts(10.0, &[(2.0, 3.0)]);
        let g = build_edit_graph(&params(&t, &tl, &[], Framing::BlurFit)).unwrap();
        assert!(g.filter_complex.contains("concat=n=2:v=1:a=1"));
        assert!(g.filter_complex.contains("atrim=start=0.000:end=2.000"));
        assert!((g.out_duration - 9.0).abs() < 1e-9);
        assert_eq!(g.map_a.as_deref(), Some("[aout]"));
    }

    #[test]
    fn music_and_sfx_add_inputs_in_order() {
        let t = tpl();
        let tl = Timeline::identity(10.0);
        let sfx = vec![
            SfxEvent { t: 1.0, kind: "pop".into(), gain: 0.5 },
            SfxEvent { t: 3.0, kind: "whoosh".into(), gain: 0.5 },
            SfxEvent { t: 6.0, kind: "whoosh".into(), gain: 0.5 },
        ];
        let files = vec![("pop".to_string(), "p.wav".to_string()), ("whoosh".to_string(), "w.wav".to_string())];
        let mut p = params(&t, &tl, &[], Framing::FaceCrop);
        p.sfx = &sfx;
        p.sfx_files = &files;
        p.music = Some("m.mp3");
        let g = build_edit_graph(&p).unwrap();
        assert_eq!(g.extra_inputs.len(), 3);
        assert!(g.extra_inputs[0].loop_forever && g.extra_inputs[0].path == "m.mp3");
        assert!(g.filter_complex.contains("sidechaincompress"));
        assert!(g.filter_complex.contains("amix=inputs=5"));
        assert!(g.filter_complex.contains("asplit=2[s3_0][s3_1]"));
    }
}
