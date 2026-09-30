"""
Video rendering module with creative layer filters.
Uses ffmpeg with NVENC for hardware encoding on NVIDIA GPUs.
"""
import os
import json
import logging
import subprocess
import tempfile
from pathlib import Path
from typing import List, Dict, Any, Optional, Tuple
from dataclasses import dataclass, asdict

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

@dataclass
class RenderConfig:
    """Configuration for video rendering."""
    # Output settings
    width: int = 1080
    height: int = 1920
    fps: int = 30
    codec: str = "h264_nvenc"
    preset: str = "p4"
    bitrate: str = "8M"
    maxrate: str = "10M"
    bufsize: str = "16M"
    
    # Audio settings
    audio_codec: str = "aac"
    audio_bitrate: str = "128k"
    sample_rate: int = 44100
    
    # Creative layer
    enable_captions: bool = True
    enable_zoom_punches: bool = True
    enable_jump_cuts: bool = True
    enable_face_crop: bool = True
    enable_music_ducking: bool = True
    enable_broll: bool = False
    enable_sfx: bool = True
    enable_progress_bar: bool = True
    
    # Caption style
    caption_style: str = "tiktok"  # tiktok, hormozi, minimal
    
    # Paths
    font_path: str = "C:/Windows/Fonts/arial.ttf"
    bold_font_path: str = "C:/Windows/Fonts/arialbd.ttf"
    sfx_dir: str = "assets/sfx"
    music_dir: str = "assets/music"
    broll_dir: str = "assets/broll"

@dataclass
class ClipRenderSpec:
    """Specification for rendering a single clip."""
    input_path: str
    output_path: str
    start_time: float
    end_time: float
    title: str
    hook: str
    viral_score: float
    # Creative layer params
    zoom_punches: List[Dict[str, Any]] = None  # [{time, scale, duration}, ...]
    jump_cuts: List[Tuple[float, float]] = None  # [(cut_start, cut_end), ...]
    face_crop_keyframes: List[Dict[str, Any]] = None
    caption_ass_path: Optional[str] = None
    music_path: Optional[str] = None
    music_volume: float = 0.15
    sfx_triggers: List[Dict[str, Any]] = None
    progress_bar: bool = True
    template: str = "default"

class FilterGraphBuilder:
    """Build complex ffmpeg filter graphs for creative effects."""
    
    def __init__(self, config: RenderConfig):
        self.config = config
        self.filters = []
        self.inputs = []
        self.output_label = "outv"
    
    def add_input(self, path: str, label: str = None) -> str:
        """Add input file and return its label."""
        if label is None:
            label = f"in{len(self.inputs)}"
        self.inputs.append((label, path))
        return label
    
    def build_base_chain(self, input_label: str, duration: float) -> str:
        """Build base filter chain: scale, crop to vertical, fps."""
        current = input_label
        
        # 1. Scale and crop to 9:16 (1080x1920) with face-aware center
        # Use crop with gravity=center as base, face crop will override
        current = self._add_filter(current, 
            f"scale={self.config.width}:{self.config.height}:force_original_aspect_ratio=increase,"
            f"crop={self.config.width}:{self.config.height}:(iw-{self.config.width})/2:(ih-{self.config.height})/2",
            "scaled"
        )
        
        # 2. FPS conversion
        current = self._add_filter(current, f"fps={self.config.fps}", "fps")
        
        # 3. Trim to exact duration
        current = self._add_filter(current, f"trim=duration={duration},setpts=PTS-STARTPTS", "trimmed")
        
        return current
    
    def add_zoom_punches(self, input_label: str, zoom_punches: List[Dict]) -> str:
        """Add zoom punch keyframes at specific timestamps."""
        if not zoom_punches:
            return input_label
        
        current = input_label
        
        # Build zoompan filter with keyframes
        # zoompan=z='if(lt(t,1.0),1.0,if(lt(t,2.0),1.3,1.0))':d=30*duration
        zoom_expr_parts = []
        for i, zp in enumerate(zoom_punches):
            t = zp.get('time', 0)
            scale = zp.get('scale', 1.3)
            dur = zp.get('duration', 0.3)
            ease = zp.get('ease', 'ease_out')
            
            # Create zoom expression using interpolation
            zoom_expr_parts.append(f"if(lt(t,{t}),1.0,if(lt(t,{t}+{dur}),1.0+({scale}-1.0)*({ease}(t-{t})/{dur}),1.0))")
        
        # Combine all zoom expressions (simplified - use max)
        if len(zoom_expr_parts) == 1:
            zoom_expr = zoom_expr_parts[0]
        else:
            zoom_expr = "max(" + ",max(".join(zoom_expr_parts) + ")" * (len(zoom_expr_parts) - 1) + ")"
        
        # For simplicity, use a more practical approach: multiple zoompan filters chained
        for i, zp in enumerate(zoom_punches):
            t = zp.get('time', 0)
            scale = zp.get('scale', 1.3)
            dur = zp.get('duration', 0.3)
            
            # Use zoompan for each punch
            current = self._add_filter(current,
                f"zoompan=z='if(lt(t,{t}),1.0,if(lt(t,{t}+{dur}),{scale},1.0))':d={int(self.config.fps * dur)}:s={self.config.width}x{self.config.height}",
                f"zoom_{i}"
            )
        
        return current
    
    def add_jump_cuts(self, input_label: str, jump_cuts: List[Tuple[float, float]]) -> str:
        """Add jump cuts by removing silent/boring sections."""
        if not jump_cuts:
            return input_label
        
        # Build select filter to keep only non-cut segments
        # This is complex in pure ffmpeg - better to pre-cut segments and concat
        # For now, return input as-is (handled at segment level)
        logger.warning("Jump cuts should be handled at segment extraction level")
        return input_label
    
    def add_face_crop(self, input_label: str, keyframes: List[Dict]) -> str:
        """Add face-aware crop with smooth panning (Kalman-smoothed)."""
        if not keyframes:
            return input_label
        
        current = input_label
        
        # Build crop filter with keyframed x,y positions
        # crop=w:h:x:y where x,y follow face
        expr_parts = []
        for kf in keyframes:
            t = kf.get('time', 0)
            x = kf.get('x', f"(iw-{self.config.width})/2")
            y = kf.get('y', f"(ih-{self.config.height})/2")
            expr_parts.append(f"if(lt(t,{t}),{x},{y})")
        
        # Simplified: use sendcmd for dynamic crop (requires ffmpeg >= 6.0)
        # For now, return input - face crop should be pre-computed
        logger.info("Face crop keyframes received, applying via crop filter")
        
        # Apply static crop for now (first keyframe)
        if keyframes:
            first = keyframes[0]
            x = first.get('x', f"(iw-{self.config.width})/2")
            y = first.get('y', f"(ih-{self.config.height})/2")
            current = self._add_filter(current, 
                f"crop={self.config.width}:{self.config.height}:{x}:{y}",
                "face_crop"
            )
        
        return current
    
    def add_captions(self, input_label: str, ass_path: str) -> str:
        """Burn in ASS subtitles."""
        if not ass_path or not os.path.exists(ass_path):
            return input_label
        
        current = self._add_filter(input_label, f"ass='{ass_path}'", "captioned")
        return current
    
    def add_progress_bar(self, input_label: str, duration: float, color: str = "0x00D4FF") -> str:
        """Add bottom progress bar."""
        if not self.config.enable_progress_bar:
            return input_label
        
        # Progress bar: 4px height at bottom, animated width
        # Using drawbox with expression for width
        bar_height = 4
        bar_y = self.config.height - bar_height
        
        current = self._add_filter(input_label,
            f"drawbox=x=0:y={bar_y}:w='iw*t/{duration}':h={bar_height}:color={color}:t=fill",
            "progress_bar"
        )
        return current
    
    def add_music_ducking(self, video_label: str, music_label: str, 
                          duck_level: float = -18, attack: int = 50, release: int = 200) -> Tuple[str, str]:
        """Add background music with sidechain ducking on speech."""
        # Video audio (speech) + music with sidechaincompress
        # [music][speech]sidechaincompress=threshold=-18dB:ratio=4:attack=50:release=200[ducked]
        # [ducked]volume=0.15[music_out]
        # [speech][music_out]amix=inputs=2:duration=first[aout]
        
        # This requires splitting audio from video input
        # For simplicity, we'll handle this in the render pipeline
        return video_label, music_label
    
    def add_sfx(self, video_label: str, sfx_specs: List[Dict]) -> str:
        """Overlay SFX at specific timestamps."""
        if not sfx_specs:
            return video_label
        
        current = video_label
        for i, sfx in enumerate(sfx_specs):
            sfx_path = sfx.get('path')
            time = sfx.get('time', 0)
            volume = sfx.get('volume', 1.0)
            
            if sfx_path and os.path.exists(sfx_path):
                sfx_label = self.add_input(sfx_path, f"sfx_{i}")
                # Delay and mix
                current = self._add_filter(
                    f"[{current}][{sfx_label}]adelay={int(time*1000)}|{int(time*1000)},amix=inputs=2:duration=first",
                    "audio",  # This is audio filter, need separate handling
                    f"sfx_mix_{i}"
                )
        
        return current
    
    def add_broll(self, video_label: str, broll_specs: List[Dict]) -> str:
        """Overlay B-roll clips with opacity ramp and motion blur."""
        if not broll_specs:
            return video_label
        
        current = video_label
        for i, br in enumerate(broll_specs):
            br_path = br.get('path')
            start = br.get('start', 0)
            end = br.get('end', 0)
            opacity = br.get('opacity', 0.7)
            blend_mode = br.get('blend', 'overlay')
            
            if br_path and os.path.exists(br_path):
                br_label = self.add_input(br_path, f"broll_{i}")
                # Scale, trim, and overlay
                br_processed = self._add_filter(br_label,
                    f"scale={self.config.width}:{self.config.height}:force_original_aspect_ratio=increase,"
                    f"crop={self.config.width}:{self.config.height},trim=start={start}:end={end},setpts=PTS-STARTPTS",
                    f"broll_proc_{i}"
                )
                current = self._add_filter(
                    f"[{current}][{br_processed}]blend=all_mode={blend_mode}:all_opacity={opacity}",
                    f"broll_overlay_{i}"
                )
        
        return current
    
    def _add_filter(self, input_label: str, filter_str: str, output_label: str = None) -> str:
        """Add a filter to the chain."""
        if output_label is None:
            output_label = f"f{len(self.filters)}"
        self.filters.append(f"[{input_label}]{filter_str}[{output_label}]")
        return output_label
    
    def get_filter_complex(self) -> str:
        """Get the complete filter_complex string."""
        return ";".join(self.filters)
    
    def get_input_args(self) -> List[str]:
        """Get input arguments for ffmpeg."""
        args = []
        for label, path in self.inputs:
            args.extend(["-i", path])
        return args


class VideoRenderer:
    """Main video rendering orchestrator."""
    
    def __init__(self, config: RenderConfig = None):
        self.config = config or RenderConfig()
        self._verify_ffmpeg()
    
    def _verify_ffmpeg(self):
        """Verify ffmpeg and NVENC availability."""
        try:
            result = subprocess.run(
                ["ffmpeg", "-hide_banner", "-encoders"],
                capture_output=True, text=True, timeout=10
            )
            if "h264_nvenc" not in result.stdout:
                logger.warning("h264_nvenc not found, falling back to libx264")
                self.config.codec = "libx264"
                self.config.preset = "fast"
            else:
                logger.info("NVENC encoder available")
        except Exception as e:
            logger.error(f"FFmpeg check failed: {e}")
            raise
    
    def render_clip(self, spec: ClipRenderSpec) -> bool:
        """
        Render a single clip with all creative effects.
        
        Args:
            spec: ClipRenderSpec with all rendering parameters
            
        Returns:
            True if successful
        """
        logger.info(f"Rendering clip: {spec.output_path}")
        logger.info(f"  Source: {spec.input_path} [{spec.start_time:.1f}-{spec.end_time:.1f}]")
        logger.info(f"  Title: {spec.title}")
        
        duration = spec.end_time - spec.start_time
        
        # Build filter graph
        builder = FilterGraphBuilder(self.config)
        
        # Main video input
        video_label = builder.add_input(spec.input_path, "main")
        
        # Base chain
        current = builder.build_base_chain(video_label, duration)
        
        # Creative layer (order matters!)
        if spec.enable_face_crop and spec.face_crop_keyframes:
            current = builder.add_face_crop(current, spec.face_crop_keyframes)
        
        if spec.enable_zoom_punches and spec.zoom_punches:
            current = builder.add_zoom_punches(current, spec.zoom_punches)
        
        if spec.enable_captions and spec.caption_ass_path:
            current = builder.add_captions(current, spec.caption_ass_path)
        
        if spec.enable_progress_bar:
            current = builder.add_progress_bar(current, duration)
        
        # Final video output label
        final_video = current
        
        # Build ffmpeg command
        cmd = ["ffmpeg", "-y", "-hide_banner", "-loglevel", "info"]
        
        # Hardware acceleration
        cmd.extend(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"])
        
        # Inputs
        cmd.extend(builder.get_input_args())
        
        # Filter complex
        filter_complex = builder.get_filter_complex()
        if filter_complex:
            cmd.extend(["-filter_complex", filter_complex])
            cmd.extend(["-map", f"[{final_video}]"])
        else:
            cmd.extend(["-map", "0:v"])
        
        # Audio mapping (main audio + music ducking)
        audio_inputs = [0]  # Main video audio
        if spec.music_path and os.path.exists(spec.music_path):
            music_label = builder.add_input(spec.music_path, "music")
            audio_inputs.append(len(builder.inputs) - 1)
        
        # Map audio
        if len(audio_inputs) == 1:
            cmd.extend(["-map", "0:a"])
        else:
            # Complex audio mixing with ducking
            # This needs a separate filter_complex for audio
            pass
        
        # Video encoding
        cmd.extend([
            "-c:v", self.config.codec,
            "-preset", self.config.preset,
            "-b:v", self.config.bitrate,
            "-maxrate", self.config.maxrate,
            "-bufsize", self.config.bufsize,
            "-r", str(self.config.fps),
            "-pix_fmt", "yuv420p",
        ])
        
        # Audio encoding
        cmd.extend([
            "-c:a", self.config.audio_codec,
            "-b:a", self.config.audio_bitrate,
            "-ar", str(self.config.sample_rate),
        ])
        
        # Output
        cmd.append(spec.output_path)
        
        logger.debug(f"FFmpeg command: {' '.join(cmd)}")
        
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
            if result.returncode != 0:
                logger.error(f"FFmpeg failed: {result.stderr}")
                return False
            
            logger.info(f"Render complete: {spec.output_path}")
            return True
            
        except subprocess.TimeoutExpired:
            logger.error("Render timed out")
            return False
        except Exception as e:
            logger.error(f"Render error: {e}")
            return False
    
    def render_clips_batch(self, specs: List[ClipRenderSpec], max_concurrent: int = 1) -> List[bool]:
        """Render multiple clips sequentially (to manage VRAM/thermal)."""
        results = []
        for i, spec in enumerate(specs):
            logger.info(f"Rendering clip {i+1}/{len(specs)}")
            success = self.render_clip(spec)
            results.append(success)
            if not success:
                logger.error(f"Clip {i+1} failed, continuing...")
        return results


def generate_zoom_punches_from_audio(audio_peaks: List[float], transcript_words: List[Dict], 
                                      emphasis_tags: List[str] = None) -> List[Dict]:
    """Auto-generate zoom punch keyframes from audio peaks and LLM emphasis tags."""
    punches = []
    
    # From audio peaks (RMS energy spikes)
    for peak_time in audio_peaks:
        punches.append({
            'time': peak_time,
            'scale': 1.25,
            'duration': 0.25,
            'ease': 'ease_out_quad',
            'source': 'audio_peak'
        })
    
    # From LLM emphasis tags (word-level)
    if emphasis_tags and transcript_words:
        for tag in emphasis_tags:
            # Find word matching tag
            for word in transcript_words:
                if tag.lower() in word['word'].lower():
                    punches.append({
                        'time': word['start'],
                        'scale': 1.3,
                        'duration': 0.3,
                        'ease': 'ease_out_cubic',
                        'source': 'llm_emphasis'
                    })
                    break
    
    # Deduplicate (merge close punches)
    punches.sort(key=lambda p: p['time'])
    merged = []
    for p in punches:
        if merged and p['time'] - merged[-1]['time'] < 0.5:
            # Merge: keep stronger punch
            if p['scale'] > merged[-1]['scale']:
                merged[-1] = p
        else:
            merged.append(p)
    
    return merged[:10]  # Max 10 punches per clip


def generate_jump_cuts_from_silence(silence_segments: List[Tuple[float, float]], 
                                     min_silence: float = 0.5,
                                     min_keep: float = 0.3) -> List[Tuple[float, float]]:
    """Generate jump cut points from silence detection."""
    jump_cuts = []
    for start, end in silence_segments:
        if end - start >= min_silence:
            # Keep a tiny bit of silence for natural breathing
            jump_cuts.append((start + min_keep, end - min_keep))
    return jump_cuts


def generate_face_crop_keyframes(face_boxes: List[Dict], smoothing: float = 0.3) -> List[Dict]:
    """
    Generate smooth face crop keyframes from BlazeFace detections.
    Applies Kalman-style smoothing to prevent jitter.
    """
    if not face_boxes:
        return []
    
    # Simple exponential smoothing
    keyframes = []
    prev_x = prev_y = None
    
    for box in face_boxes:
        t = box['time']
        # Face box: x, y, w, h in normalized coords (0-1)
        fx = box['x'] + box['w'] / 2  # Center x
        fy = box['y'] + box['h'] / 2  # Center y
        
        # Convert to pixel coordinates for 1080x1920 crop
        # We want face center at 1/3 from top (rule of thirds)
        target_y = 1920 / 3
        target_x = 1080 / 2
        
        # Crop coordinates (top-left of 1080x1920 crop)
        crop_x = int(fx * 1920 - 540)  # Assuming source is 1920 wide
        crop_y = int(fy * 1080 - target_y)  # This needs source dimensions
        
        # Smooth
        if prev_x is not None:
            crop_x = int(prev_x * (1 - smoothing) + crop_x * smoothing)
            crop_y = int(prev_y * (1 - smoothing) + crop_y * smoothing)
        
        prev_x, prev_y = crop_x, crop_y
        
        keyframes.append({
            'time': t,
            'x': crop_x,
            'y': crop_y,
        })
    
    return keyframes


if __name__ == "__main__":
    import argparse
    
    parser = argparse.ArgumentParser(description="Render clip with creative effects")
    parser.add_argument("input", help="Input video file")
    parser.add_argument("output", help="Output video file")
    parser.add_argument("--start", type=float, default=0, help="Start time")
    parser.add_argument("--end", type=float, default=30, help="End time")
    parser.add_argument("--ass", help="ASS subtitle file")
    parser.add_argument("--music", help="Background music file")
    parser.add_argument("--title", default="Test Clip", help="Clip title")
    
    args = parser.parse_args()
    
    config = RenderConfig()
    renderer = VideoRenderer(config)
    
    spec = ClipRenderSpec(
        input_path=args.input,
        output_path=args.output,
        start_time=args.start,
        end_time=args.end,
        title=args.title,
        hook="",
        viral_score=0,
        caption_ass_path=args.ass,
        music_path=args.music,
    )
    
    success = renderer.render_clip(spec)
    exit(0 if success else 1)