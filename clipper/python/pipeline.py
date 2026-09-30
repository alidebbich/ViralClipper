"""
Main pipeline orchestrator - ties together download, transcribe, analyze, render.
Manages VRAM by running stages sequentially.
"""
import os
import json
import logging
import asyncio
import tempfile
import shutil
from pathlib import Path
from typing import List, Dict, Any, Optional, Callable
from dataclasses import dataclass, asdict, field
from datetime import datetime
from enum import Enum
import uuid

# Import our modules
import sys
sys.path.append(str(Path(__file__).parent))

from transcribe import Transcriber, TranscriptionResult
from heuristics import ClipCandidate, analyze_and_rank, HeuristicAnalyzer, LLMReranker
from render import VideoRenderer, ClipRenderSpec, RenderConfig, generate_zoom_punches_from_audio

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

class PipelineStage(Enum):
    PENDING = "pending"
    DOWNLOADING = "downloading"
    EXTRACTING_AUDIO = "extracting_audio"
    TRANSCRIBING = "transcribing"
    ANALYZING = "analyzing"
    RENDERING = "rendering"
    COMPLETED = "completed"
    FAILED = "failed"

@dataclass
class PipelineProject:
    id: str
    url: str
    title: str = ""
    status: PipelineStage = PipelineStage.PENDING
    progress: float = 0.0
    created_at: str = field(default_factory=lambda: datetime.now().isoformat())
    updated_at: str = field(default_factory=lambda: datetime.now().isoformat())
    
    # Paths
    video_path: str = ""
    audio_path: str = ""
    transcript_path: str = ""
    
    # Results
    clips: List[Dict[str, Any]] = field(default_factory=list)
    error: str = ""
    
    # Config
    template: str = "default"
    target_clips: int = 3
    context: str = ""

class PipelineOrchestrator:
    """Orchestrates the full clipper pipeline with VRAM-aware sequential execution."""
    
    def __init__(
        self,
        work_dir: str = "clipper_workspace",
        models_dir: str = "models",
        assets_dir: str = "assets",
        progress_callback: Optional[Callable[[str, PipelineStage, float], None]] = None,
    ):
        self.work_dir = Path(work_dir)
        self.models_dir = Path(models_dir)
        self.assets_dir = Path(assets_dir)
        self.progress_callback = progress_callback
        
        # Create directories
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.models_dir.mkdir(parents=True, exist_ok=True)
        self.assets_dir.mkdir(parents=True, exist_ok=True)
        (self.assets_dir / "sfx").mkdir(exist_ok=True)
        (self.assets_dir / "music").mkdir(exist_ok=True)
        (self.assets_dir / "broll").mkdir(exist_ok=True)
        (self.assets_dir / "templates").mkdir(exist_ok=True)
        (self.assets_dir / "fonts").mkdir(exist_ok=True)
        
        # Components (lazy loaded)
        self._transcriber: Optional[Transcriber] = None
        self._renderer: Optional[VideoRenderer] = None
        self._analyzer: Optional[HeuristicAnalyzer] = None
        self._reranker: Optional[LLMReranker] = None
        
        # State
        self.projects: Dict[str, PipelineProject] = {}
        self._current_project: Optional[str] = None
    
    def _report_progress(self, project_id: str, stage: PipelineStage, progress: float):
        """Report progress to callback."""
        if project_id in self.projects:
            self.projects[project_id].status = stage
            self.projects[project_id].progress = progress
            self.projects[project_id].updated_at = datetime.now().isoformat()
        
        if self.progress_callback:
            self.progress_callback(project_id, stage, progress)
    
    @property
    def transcriber(self) -> Transcriber:
        """Lazy load transcriber."""
        if self._transcriber is None:
            logger.info("Loading Whisper model...")
            self._transcriber = Transcriber(
                model_size="large-v3",
                device="auto",
                compute_type="int8_float16",
                cpu_threads=4,
                download_root=str(self.models_dir / "whisper"),
            )
        return self._transcriber
    
    @property
    def renderer(self) -> VideoRenderer:
        """Lazy load renderer."""
        if self._renderer is None:
            self._renderer = VideoRenderer(RenderConfig(
                sfx_dir=str(self.assets_dir / "sfx"),
                music_dir=str(self.assets_dir / "music"),
                broll_dir=str(self.assets_dir / "broll"),
            ))
        return self._renderer
    
    @property
    def analyzer(self) -> HeuristicAnalyzer:
        if self._analyzer is None:
            self._analyzer = HeuristicAnalyzer()
        return self._analyzer
    
    @property
    def reranker(self) -> LLMReranker:
        if self._reranker is None:
            self._reranker = LLMReranker()
        return self._reranker
    
    def unload_models(self):
        """Unload heavy models to free VRAM."""
        if self._transcriber:
            self._transcriber.unload()
            self._transcriber = None
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
        logger.info("Models unloaded, VRAM freed")
    
    def create_project(self, url: str, context: str = "", template: str = "default", target_clips: int = 3) -> PipelineProject:
        """Create a new pipeline project."""
        project = PipelineProject(
            id=str(uuid.uuid4())[:8],
            url=url,
            context=context,
            template=template,
            target_clips=target_clips,
        )
        self.projects[project.id] = project
        self._save_project(project)
        return project
    
    def _save_project(self, project: PipelineProject):
        """Save project state to disk."""
        project_dir = self.work_dir / project.id
        project_dir.mkdir(exist_ok=True)
        
        state_file = project_dir / "state.json"
        with open(state_file, 'w') as f:
            json.dump(asdict(project), f, indent=2)
    
    def load_project(self, project_id: str) -> Optional[PipelineProject]:
        """Load project from disk."""
        if project_id in self.projects:
            return self.projects[project_id]
        
        state_file = self.work_dir / project_id / "state.json"
        if state_file.exists():
            with open(state_file) as f:
                data = json.load(f)
            project = PipelineProject(**data)
            self.projects[project_id] = project
            return project
        return None
    
    async def run_pipeline(self, project_id: str) -> bool:
        """Run the full pipeline for a project."""
        project = self.projects.get(project_id) or self.load_project(project_id)
        if not project:
            logger.error(f"Project not found: {project_id}")
            return False
        
        self._current_project = project_id
        
        try:
            # Stage 1: Download
            self._report_progress(project_id, PipelineStage.DOWNLOADING, 5)
            await self._download_video(project)
            self._save_project(project)
            
            # Stage 2: Extract audio
            self._report_progress(project_id, PipelineStage.EXTRACTING_AUDIO, 15)
            await self._extract_audio(project)
            self._save_project(project)
            
            # Stage 3: Transcribe
            self._report_progress(project_id, PipelineStage.TRANSCRIBING, 25)
            await self._transcribe_video(project)
            self.unload_models()  # Free VRAM before LLM
            self._save_project(project)
            
            # Stage 4: Analyze
            self._report_progress(project_id, PipelineStage.ANALYZING, 55)
            await self._analyze_clips(project)
            self._save_project(project)
            
            # Stage 5: Render
            self._report_progress(project_id, PipelineStage.RENDERING, 70)
            await self._render_clips(project)
            self._save_project(project)
            
            # Complete
            self._report_progress(project_id, PipelineStage.COMPLETED, 100)
            logger.info(f"Pipeline completed for project {project_id}")
            return True
            
        except Exception as e:
            logger.error(f"Pipeline failed for {project_id}: {e}")
            project.error = str(e)
            project.status = PipelineStage.FAILED
            self._report_progress(project_id, PipelineStage.FAILED, 0)
            self._save_project(project)
            return False
    
    async def _download_video(self, project: PipelineProject):
        """Download video using yt-dlp."""
        project_dir = self.work_dir / project.id
        output_template = str(project_dir / "source.%(ext)s")
        
        cmd = [
            "yt-dlp",
            "-f", "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best",
            "-o", output_template,
            "--merge-output-format", "mp4",
            "--no-playlist",
            "--write-info-json",
            "--write-thumbnail",
            project.url,
        ]
        
        logger.info(f"Downloading: {project.url}")
        process = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await process.communicate()
        
        if process.returncode != 0:
            raise RuntimeError(f"Download failed: {stderr.decode()}")
        
        # Find downloaded video
        video_files = list(project_dir.glob("source.*"))
        video_files = [f for f in video_files if f.suffix in ['.mp4', '.mkv', '.webm', '.mov']]
        
        if not video_files:
            raise RuntimeError("No video file found after download")
        
        project.video_path = str(video_files[0])
        
        # Load metadata
        info_files = list(project_dir.glob("*.info.json"))
        if info_files:
            with open(info_files[0]) as f:
                info = json.load(f)
            project.title = info.get('title', 'Untitled')
    
    async def _extract_audio(self, project: PipelineProject):
        """Extract audio from video for transcription."""
        if not project.video_path:
            raise RuntimeError("No video path")
        
        project_dir = self.work_dir / project.id
        audio_path = project_dir / "audio.wav"
        
        cmd = [
            "ffmpeg", "-y", "-hide_banner", "-loglevel", "error",
            "-i", project.video_path,
            "-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1",
            str(audio_path),
        ]
        
        logger.info("Extracting audio...")
        process = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        await process.communicate()
        
        if process.returncode != 0:
            raise RuntimeError("Audio extraction failed")
        
        project.audio_path = str(audio_path)
    
    async def _transcribe_video(self, project: PipelineProject):
        """Transcribe audio using faster-whisper."""
        if not project.audio_path:
            raise RuntimeError("No audio path")
        
        project_dir = self.work_dir / project.id
        transcript_path = project_dir / "transcript.json"
        ass_path = project_dir / "captions.ass"
        srt_path = project_dir / "captions.srt"
        
        logger.info("Transcribing with faster-whisper...")
        
        # Run transcription in thread pool (blocking)
        loop = asyncio.get_event_loop()
        result = await loop.run_in_executor(
            None,
            lambda: self.transcriber.transcribe_to_json(
                project.audio_path,
                output_path=str(transcript_path),
                language=None,  # Auto-detect
                word_timestamps=True,
            )
        )
        
        # Also generate ASS for caption burning
        await loop.run_in_executor(
            None,
            lambda: self.transcriber.transcribe_to_ass(
                project.audio_path,
                output_path=str(ass_path),
                language=None,
            )
        )
        
        # Generate SRT as backup
        await loop.run_in_executor(
            None,
            lambda: self.transcriber.transcribe_to_srt(
                project.audio_path,
                output_path=str(srt_path),
                language=None,
            )
        )
        
        project.transcript_path = str(transcript_path)
    
    async def _analyze_clips(self, project: PipelineProject):
        """Analyze transcript and select best clips."""
        if not project.transcript_path:
            raise RuntimeError("No transcript path")
        
        logger.info("Analyzing clips with heuristics + LLM...")
        
        # Load transcript
        with open(project.transcript_path) as f:
            transcript = json.load(f)
        
        # Extract audio energy for scoring (optional)
        audio_energy = await self._extract_audio_energy(project.audio_path)
        
        # Run analysis
        loop = asyncio.get_event_loop()
        clips = await loop.run_in_executor(
            None,
            lambda: analyze_and_rank(
                project.transcript_path,
                audio_energy=audio_energy,
                context=project.context,
                top_k=project.target_clips,
                use_llm=True,
            )
        )
        
        # Convert to dict
        project.clips = []
        for i, clip in enumerate(clips):
            clip_dict = {
                "id": f"{project.id}_clip_{i+1}",
                "index": i + 1,
                "start": clip.start,
                "end": clip.end,
                "duration": clip.duration,
                "title": clip.suggested_title or f"Clip {i+1}",
                "hook": clip.suggested_hook or clip.text[:80],
                "viral_score": clip.viral_score,
                "heuristic_score": clip.heuristic_score,
                "reason": clip.llm_reason,
                "breakdown": {
                    "hook": clip.hook_score,
                    "pace": clip.pace_score,
                    "sentiment": clip.sentiment_score,
                    "containment": clip.containment_score,
                    "audio_energy": clip.audio_energy_score,
                    "duration": clip.duration_score,
                },
                "text": clip.text,
                "status": "pending",
            }
            project.clips.append(clip_dict)
        
        logger.info(f"Selected {len(project.clips)} clips")
    
    async def _extract_audio_energy(self, audio_path: str) -> List[Tuple[float, float]]:
        """Extract RMS energy over time for audio energy scoring."""
        import librosa
        
        try:
            y, sr = librosa.load(audio_path, sr=16000)
            # Compute RMS energy in 100ms windows
            frame_length = int(0.1 * sr)
            hop_length = frame_length // 2
            rms = librosa.feature.rms(y=y, frame_length=frame_length, hop_length=hop_length)[0]
            times = librosa.frames_to_time(range(len(rms)), sr=sr, hop_length=hop_length)
            return list(zip(times, rms.tolist()))
        except Exception as e:
            logger.warning(f"Audio energy extraction failed: {e}")
            return []
    
    async def _render_clips(self, project: PipelineProject):
        """Render selected clips with creative effects."""
        if not project.clips:
            raise RuntimeError("No clips to render")
        
        project_dir = self.work_dir / project.id
        output_dir = project_dir / "output"
        output_dir.mkdir(exist_ok=True)
        
        render_specs = []
        
        for clip in project.clips:
            clip_id = clip['id']
            clip_output = output_dir / f"{clip_id}.mp4"
            
            # Generate creative layer specs
            zoom_punches = generate_zoom_punches_from_audio(
                audio_peaks=[],  # Would come from audio analysis
                transcript_words=[],  # From transcript
                emphasis_tags=[],  # From LLM
            )
            
            spec = ClipRenderSpec(
                input_path=project.video_path,
                output_path=str(clip_output),
                start_time=clip['start'],
                end_time=clip['end'],
                title=clip['title'],
                hook=clip['hook'],
                viral_score=clip['viral_score'],
                zoom_punches=zoom_punches,
                caption_ass_path=str(project_dir / "captions.ass"),
                progress_bar=True,
            )
            render_specs.append(spec)
        
        # Render sequentially (VRAM/thermal management)
        logger.info(f"Rendering {len(render_specs)} clips...")
        results = self.renderer.render_clips_batch(render_specs, max_concurrent=1)
        
        # Update clip status
        for clip, success in zip(project.clips, results):
            clip['status'] = 'completed' if success else 'error'
            if success:
                clip['render_path'] = str(output_dir / f"{clip['id']}.mp4")
    
    def get_project_status(self, project_id: str) -> Optional[Dict]:
        """Get project status for UI."""
        project = self.projects.get(project_id) or self.load_project(project_id)
        if not project:
            return None
        
        return {
            "id": project.id,
            "url": project.url,
            "title": project.title,
            "status": project.status.value,
            "progress": project.progress,
            "clips": project.clips,
            "error": project.error,
            "created_at": project.created_at,
            "updated_at": project.updated_at,
        }
    
    def list_projects(self) -> List[Dict]:
        """List all projects."""
        projects = []
        for project_dir in self.work_dir.iterdir():
            if project_dir.is_dir():
                state_file = project_dir / "state.json"
                if state_file.exists():
                    with open(state_file) as f:
                        data = json.load(f)
                    projects.append({
                        "id": data['id'],
                        "url": data['url'],
                        "title": data['title'],
                        "status": data['status'],
                        "progress": data['progress'],
                        "created_at": data['created_at'],
                    })
        return sorted(projects, key=lambda p: p['created_at'], reverse=True)


# For running as a service
async def main():
    """Example usage."""
    orchestrator = PipelineOrchestrator()
    
    # Create project
    project = orchestrator.create_project(
        url="https://youtube.com/watch?v=...",
        context="Tech education, programming tutorials",
        target_clips=3,
    )
    
    print(f"Created project: {project.id}")
    
    # Run pipeline
    success = await orchestrator.run_pipeline(project.id)
    
    if success:
        status = orchestrator.get_project_status(project.id)
        print(f"Completed! Clips: {len(status['clips'])}")
        for clip in status['clips']:
            print(f"  - {clip['title']} ({clip['viral_score']}/10)")
    else:
        print(f"Failed: {project.error}")


if __name__ == "__main__":
    import torch
    asyncio.run(main())