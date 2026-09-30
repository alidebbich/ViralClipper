"""
Transcription module using faster-whisper (CTranslate2)
Optimized for mobile GPU (RTX 3050 4GB VRAM) with CPU offload
"""
import os
import json
import logging
from pathlib import Path
from typing import Optional, List, Dict, Any
from dataclasses import dataclass, asdict
from faster_whisper import WhisperModel
import torch

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

@dataclass
class Word:
    start: float
    end: float
    word: str
    probability: float

@dataclass
class Segment:
    id: int
    start: float
    end: float
    text: str
    words: List[Word]
    avg_logprob: float
    no_speech_prob: float

@dataclass
class TranscriptionResult:
    language: str
    language_probability: float
    duration: float
    segments: List[Segment]
    words: List[Word]

class Transcriber:
    def __init__(
        self,
        model_size: str = "large-v3",
        device: str = "auto",
        compute_type: str = "int8_float16",
        cpu_threads: int = 4,
        download_root: Optional[str] = None
    ):
        """
        Initialize faster-whisper model with VRAM-aware settings.
        
        Args:
            model_size: Whisper model size (tiny, base, small, medium, large-v3)
            device: "auto", "cuda", "cpu" - auto will use CUDA if available
            compute_type: "int8", "int8_float16", "float16", "float32"
                         int8_float16 is best for 4-6GB VRAM
            cpu_threads: Number of CPU threads for CPU offload
            download_root: Custom model cache directory
        """
        self.model_size = model_size
        self.device = device
        self.compute_type = compute_type
        self.cpu_threads = cpu_threads
        self.download_root = download_root
        
        # Determine device
        if device == "auto":
            self.device = "cuda" if torch.cuda.is_available() else "cpu"
        
        logger.info(f"Initializing Whisper model: {model_size} on {self.device} with {compute_type}")
        
        # Initialize model with CPU offload for VRAM efficiency
        self.model = WhisperModel(
            model_size_or_path=model_size,
            device=self.device,
            compute_type=compute_type,
            cpu_threads=cpu_threads,
            num_workers=1,  # Single worker to avoid VRAM spikes
            download_root=download_root,
        )
        
        # Warm up
        self._warmup()
    
    def _warmup(self):
        """Warm up model with a dummy transcription."""
        import numpy as np
        dummy_audio = np.zeros(16000, dtype=np.float32)  # 1 second of silence
        try:
            list(self.model.transcribe(dummy_audio, language="en", beam_size=1))
            logger.info("Model warmed up successfully")
        except Exception as e:
            logger.warning(f"Warmup failed: {e}")
    
    def transcribe(
        self,
        audio_path: str,
        language: Optional[str] = None,
        task: str = "transcribe",
        beam_size: int = 5,
        vad_filter: bool = True,
        vad_parameters: Optional[Dict] = None,
        word_timestamps: bool = True,
        condition_on_previous_text: bool = True,
        no_speech_threshold: float = 0.6,
        logprob_threshold: float = -1.0,
        compression_ratio_threshold: float = 2.4,
    ) -> TranscriptionResult:
        """
        Transcribe audio/video file.
        
        Args:
            audio_path: Path to audio/video file
            language: Language code (None for auto-detect)
            task: "transcribe" or "translate"
            beam_size: Beam size for decoding
            vad_filter: Enable voice activity detection
            vad_parameters: VAD parameters dict
            word_timestamps: Enable word-level timestamps
            condition_on_previous_text: Use previous text as context
            no_speech_threshold: Threshold for no-speech detection
            logprob_threshold: Minimum average logprob
            compression_ratio_threshold: Maximum compression ratio
            
        Returns:
            TranscriptionResult with segments and word timestamps
        """
        if not os.path.exists(audio_path):
            raise FileNotFoundError(f"Audio file not found: {audio_path}")
        
        logger.info(f"Transcribing: {audio_path}")
        
        # Default VAD parameters for speech detection
        if vad_parameters is None:
            vad_parameters = {
                "threshold": 0.5,
                "min_speech_duration_ms": 250,
                "max_speech_duration_s": 30,
                "min_silence_duration_ms": 2000,
                "speech_pad_ms": 400,
            }
        
        segments, info = self.model.transcribe(
            audio_path,
            language=language,
            task=task,
            beam_size=beam_size,
            vad_filter=vad_filter,
            vad_parameters=vad_parameters,
            word_timestamps=word_timestamps,
            condition_on_previous_text=condition_on_previous_text,
            no_speech_threshold=no_speech_threshold,
            logprob_threshold=logprob_threshold,
            compression_ratio_threshold=compression_ratio_threshold,
        )
        
        # Convert to dataclasses
        segment_list = []
        all_words = []
        
        for i, segment in enumerate(segments):
            words = []
            if segment.words:
                for w in segment.words:
                    word = Word(
                        start=w.start,
                        end=w.end,
                        word=w.word,
                        probability=w.probability
                    )
                    words.append(word)
                    all_words.append(word)
            
            seg = Segment(
                id=i,
                start=segment.start,
                end=segment.end,
                text=segment.text.strip(),
                words=words,
                avg_logprob=segment.avg_logprob,
                no_speech_prob=segment.no_speech_prob
            )
            segment_list.append(seg)
        
        result = TranscriptionResult(
            language=info.language,
            language_probability=info.language_probability,
            duration=info.duration,
            segments=segment_list,
            words=all_words
        )
        
        logger.info(f"Transcription complete: {len(segment_list)} segments, {len(all_words)} words, language: {info.language} ({info.language_probability:.2f})")
        return result
    
    def transcribe_to_json(self, audio_path: str, output_path: Optional[str] = None, **kwargs) -> str:
        """Transcribe and save as JSON."""
        result = self.transcribe(audio_path, **kwargs)
        
        # Convert to dict
        data = {
            "language": result.language,
            "language_probability": result.language_probability,
            "duration": result.duration,
            "segments": [
                {
                    "id": s.id,
                    "start": s.start,
                    "end": s.end,
                    "text": s.text,
                    "words": [{"start": w.start, "end": w.end, "word": w.word, "probability": w.probability} for w in s.words],
                    "avg_logprob": s.avg_logprob,
                    "no_speech_prob": s.no_speech_prob
                }
                for s in result.segments
            ]
        }
        
        json_str = json.dumps(data, ensure_ascii=False, indent=2)
        
        if output_path:
            with open(output_path, 'w', encoding='utf-8') as f:
                f.write(json_str)
            logger.info(f"Transcription saved to: {output_path}")
        
        return json_str
    
    def transcribe_to_srt(self, audio_path: str, output_path: Optional[str] = None, **kwargs) -> str:
        """Transcribe and save as SRT subtitles."""
        result = self.transcribe(audio_path, **kwargs)
        
        srt_lines = []
        for i, segment in enumerate(result.segments, 1):
            start = self._format_timestamp(segment.start)
            end = self._format_timestamp(segment.end)
            srt_lines.append(f"{i}")
            srt_lines.append(f"{start} --> {end}")
            srt_lines.append(segment.text)
            srt_lines.append("")
        
        srt_content = "\n".join(srt_lines)
        
        if output_path:
            with open(output_path, 'w', encoding='utf-8') as f:
                f.write(srt_content)
            logger.info(f"SRT saved to: {output_path}")
        
        return srt_content
    
    def transcribe_to_ass(self, audio_path: str, output_path: Optional[str] = None, style: Optional[Dict] = None, **kwargs) -> str:
        """Transcribe and save as ASS subtitles with karaoke-style word highlighting."""
        result = self.transcribe(audio_path, **kwargs)
        
        # Default ASS style for TikTok/Shorts captions
        default_style = {
            "Fontname": "Arial",
            "Fontsize": 24,
            "PrimaryColour": "&H00FFFFFF",  # White
            "SecondaryColour": "&H000000FF",  # Blue highlight
            "OutlineColour": "&H00000000",  # Black outline
            "BackColour": "&H80000000",  # Semi-transparent black
            "Bold": 1,
            "Italic": 0,
            "Underline": 0,
            "StrikeOut": 0,
            "ScaleX": 100,
            "ScaleY": 100,
            "Spacing": 0,
            "Angle": 0,
            "BorderStyle": 1,
            "Outline": 2,
            "Shadow": 1,
            "Alignment": 2,  # Bottom center
            "MarginL": 20,
            "MarginR": 20,
            "MarginV": 30,
            "Encoding": 1,
        }
        
        if style:
            default_style.update(style)
        
        ass_lines = [
            "[Script Info]",
            "Title: Clipper Transcription",
            "ScriptType: v4.00+",
            "WrapStyle: 0",
            "ScaledBorderAndShadow: yes",
            "YCbCr Matrix: None",
            "",
            "[V4+ Styles]",
            "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
            f"Style: Default,{default_style['Fontname']},{default_style['Fontsize']},{default_style['PrimaryColour']},{default_style['SecondaryColour']},{default_style['OutlineColour']},{default_style['BackColour']},{default_style['Bold']},{default_style['Italic']},{default_style['Underline']},{default_style['StrikeOut']},{default_style['ScaleX']},{default_style['ScaleY']},{default_style['Spacing']},{default_style['Angle']},{default_style['BorderStyle']},{default_style['Outline']},{default_style['Shadow']},{default_style['Alignment']},{default_style['MarginL']},{default_style['MarginR']},{default_style['MarginV']},{default_style['Encoding']}",
            "",
            "[Events]",
            "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
        ]
        
        for segment in result.segments:
            if not segment.words:
                # Fallback for segments without word timestamps
                start = self._format_timestamp_ass(segment.start)
                end = self._format_timestamp_ass(segment.end)
                ass_lines.append(f"Dialogue: 0,{start},{end},Default,,0,0,0,,{segment.text}")
            else:
                # Word-level karaoke timing
                for word in segment.words:
                    start = self._format_timestamp_ass(word.start)
                    end = self._format_timestamp_ass(word.end)
                    # {\k} tag for karaoke timing (centiseconds)
                    k_duration = int((word.end - word.start) * 100)
                    ass_lines.append(f"Dialogue: 0,{start},{end},Default,,0,0,0,{{\\k{k_duration}}}{word.word}")
        
        ass_content = "\n".join(ass_lines)
        
        if output_path:
            with open(output_path, 'w', encoding='utf-8') as f:
                f.write(ass_content)
            logger.info(f"ASS saved to: {output_path}")
        
        return ass_content
    
    @staticmethod
    def _format_timestamp(seconds: float) -> str:
        """Format seconds as SRT timestamp (HH:MM:SS,mmm)."""
        hours = int(seconds // 3600)
        minutes = int((seconds % 3600) // 60)
        secs = int(seconds % 60)
        millis = int((seconds % 1) * 1000)
        return f"{hours:02d}:{minutes:02d}:{secs:02d},{millis:03d}"
    
    @staticmethod
    def _format_timestamp_ass(seconds: float) -> str:
        """Format seconds as ASS timestamp (H:MM:SS.cc)."""
        hours = int(seconds // 3600)
        minutes = int((seconds % 3600) // 60)
        secs = int(seconds % 60)
        centiseconds = int((seconds % 1) * 100)
        return f"{hours}:{minutes:02d}:{secs:02d}.{centiseconds:02d}"
    
    def unload(self):
        """Unload model to free VRAM."""
        if hasattr(self, 'model'):
            del self.model
            if torch.cuda.is_available():
                torch.cuda.empty_cache()
            logger.info("Model unloaded, VRAM freed")


def main():
    """CLI entry point for testing."""
    import argparse
    
    parser = argparse.ArgumentParser(description="Transcribe audio/video with faster-whisper")
    parser.add_argument("input", help="Input audio/video file")
    parser.add_argument("-o", "--output", help="Output file (json/srt/ass)")
    parser.add_argument("-f", "--format", choices=["json", "srt", "ass"], default="json")
    parser.add_argument("-m", "--model", default="large-v3")
    parser.add_argument("-l", "--language", default=None)
    parser.add_argument("--device", default="auto", choices=["auto", "cuda", "cpu"])
    parser.add_argument("--compute-type", default="int8_float16", choices=["int8", "int8_float16", "float16", "float32"])
    
    args = parser.parse_args()
    
    transcriber = Transcriber(
        model_size=args.model,
        device=args.device,
        compute_type=args.compute_type,
    )
    
    try:
        if args.format == "json":
            result = transcriber.transcribe_to_json(args.input, args.output, language=args.language)
        elif args.format == "srt":
            result = transcriber.transcribe_to_srt(args.input, args.output, language=args.language)
        elif args.format == "ass":
            result = transcriber.transcribe_to_ass(args.input, args.output, language=args.language)
        
        if not args.output:
            print(result)
    finally:
        transcriber.unload()


if __name__ == "__main__":
    main()