"""
Heuristic clip analysis module for viral potential scoring.
Fast, deterministic filtering before LLM rerank.
"""
import re
import json
import logging
from pathlib import Path
from typing import List, Dict, Any, Optional, Tuple
from dataclasses import dataclass, asdict
from collections import Counter
import nltk
from nltk.sentiment import SentimentIntensityAnalyzer
import spacy

# Download required NLTK data
try:
    nltk.data.find('vader_lexicon')
except LookupError:
    nltk.download('vader_lexicon', quiet=True)

try:
    nltk.data.find('tokenizers/punkt')
except LookupError:
    nltk.download('punkt', quiet=True)

# Load spaCy model (small for speed)
try:
    nlp = spacy.load("en_core_web_sm")
except OSError:
    import subprocess
    subprocess.run(["python", "-m", "spacy", "download", "en_core_web_sm"], check=True)
    nlp = spacy.load("en_core_web_sm")

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

@dataclass
class ClipCandidate:
    """A candidate clip segment with metadata."""
    start: float
    end: float
    duration: float
    text: str
    words: List[Dict[str, Any]]
    # Heuristic scores (0-1)
    hook_score: float = 0.0
    pace_score: float = 0.0
    sentiment_score: float = 0.0
    containment_score: float = 0.0
    audio_energy_score: float = 0.0
    duration_score: float = 0.0
    # Combined heuristic score
    heuristic_score: float = 0.0
    # LLM rerank (filled later)
    viral_score: float = 0.0
    llm_reason: str = ""
    suggested_title: str = ""
    suggested_hook: str = ""

@dataclass
class HeuristicWeights:
    """Weights for heuristic scoring."""
    hook: float = 0.25
    pace: float = 0.20
    sentiment: float = 0.15
    containment: float = 0.15
    audio_energy: float = 0.15
    duration: float = 0.10

class HeuristicAnalyzer:
    """Analyze transcript segments for viral potential using heuristics."""
    
    # Hook patterns for first 3 seconds
    HOOK_PATTERNS = [
        (r'\b(stop|wait|don\'t|never|always)\b', 1.0),  # Command hooks
        (r'\b(you|your|yours)\b', 0.8),  # Direct address
        (r'\b(this|that|these|those)\b', 0.6),  # Demonstrative
        (r'\?', 0.9),  # Questions
        (r'\b(shocking|unbelievable|insane|crazy|wild|secret|hidden)\b', 1.0),  # Shock words
        (r'\b(how to|why|what if|imagine|picture this)\b', 0.9),  # Curiosity gaps
        (r'\b(\d+)\b', 0.7),  # Numbers
        (r'\b(mistake|wrong|failed|error)\b', 0.8),  # Negative framing
    ]
    
    # Filler words to detect for jump cuts
    FILLER_WORDS = {
        'um', 'uh', 'er', 'ah', 'like', 'you know', 'i mean', 'actually',
        'basically', 'literally', 'so', 'well', 'right', 'okay', 'ok'
    }
    
    # Emphasis words that correlate with audio peaks
    EMPHASIS_WORDS = {
        'never', 'always', 'must', 'need', 'critical', 'essential', 'vital',
        'important', 'key', 'secret', 'best', 'worst', 'top', 'ultimate',
        'guaranteed', 'proven', 'breaking', 'urgent', 'alert', 'warning'
    }
    
    def __init__(self, weights: Optional[HeuristicWeights] = None):
        self.weights = weights or HeuristicWeights()
        self.sia = SentimentIntensityAnalyzer()
        
        # Compile regex patterns
        self.hook_regexes = [(re.compile(pattern, re.IGNORECASE), weight) 
                            for pattern, weight in self.HOOK_PATTERNS]
    
    def analyze_transcript(
        self,
        segments: List[Dict[str, Any]],
        words: List[Dict[str, Any]],
        audio_energy: Optional[List[Tuple[float, float]]] = None,
        min_duration: float = 15.0,
        max_duration: float = 60.0,
        target_clips: int = 8,
    ) -> List[ClipCandidate]:
        """
        Analyze full transcript and extract top clip candidates.
        
        Args:
            segments: List of segment dicts with start, end, text
            words: List of word dicts with start, end, word
            audio_energy: Optional list of (timestamp, rms_energy) pairs
            min_duration: Minimum clip duration in seconds
            max_duration: Maximum clip duration in seconds
            target_clips: Number of top clips to return
            
        Returns:
            List of top ClipCandidate objects sorted by heuristic score
        """
        logger.info(f"Analyzing {len(segments)} segments for clip candidates")
        
        # Generate candidate windows using sliding window
        candidates = self._generate_candidates(
            segments, words, min_duration, max_duration
        )
        
        logger.info(f"Generated {len(candidates)} candidate windows")
        
        # Score each candidate
        for candidate in candidates:
            self._score_candidate(candidate, words, audio_energy)
        
        # Sort by heuristic score and take top N
        candidates.sort(key=lambda c: c.heuristic_score, reverse=True)
        top_candidates = candidates[:target_clips]
        
        logger.info(f"Top {len(top_candidates)} candidates selected")
        for i, c in enumerate(top_candidates):
            logger.info(f"  {i+1}. [{c.start:.1f}-{c.end:.1f}] Score: {c.heuristic_score:.3f} - {c.text[:60]}...")
        
        return top_candidates
    
    def _generate_candidates(
        self,
        segments: List[Dict],
        words: List[Dict],
        min_duration: float,
        max_duration: float,
    ) -> List[ClipCandidate]:
        """Generate candidate clip windows using sliding window over segments."""
        candidates = []
        
        # Strategy 1: Segment-based windows (1-3 segments)
        for i in range(len(segments)):
            for j in range(i, min(i + 3, len(segments))):
                start = segments[i]['start']
                end = segments[j]['end']
                duration = end - start
                
                if min_duration <= duration <= max_duration:
                    text = " ".join(s['text'] for s in segments[i:j+1])
                    candidate_words = [w for w in words if start <= w['start'] < end]
                    
                    candidates.append(ClipCandidate(
                        start=start,
                        end=end,
                        duration=duration,
                        text=text.strip(),
                        words=candidate_words,
                    ))
        
        # Strategy 2: Fixed sliding window (every 5 seconds)
        if segments:
            total_duration = segments[-1]['end']
            window_starts = [s for s in range(0, int(total_duration), 5)]
            
            for ws in window_starts:
                we = ws + min_duration
                if we <= total_duration:
                    # Find overlapping segments
                    overlapping = [s for s in segments if s['start'] < we and s['end'] > ws]
                    if overlapping:
                        text = " ".join(s['text'] for s in overlapping)
                        candidate_words = [w for w in words if ws <= w['start'] < we]
                        
                        candidates.append(ClipCandidate(
                            start=ws,
                            end=we,
                            duration=we - ws,
                            text=text.strip(),
                            words=candidate_words,
                        ))
        
        # Deduplicate similar windows (within 2 seconds)
        unique = []
        for c in candidates:
            if not any(abs(c.start - u.start) < 2 and abs(c.end - u.end) < 2 for u in unique):
                unique.append(c)
        
        return unique
    
    def _score_candidate(
        self,
        candidate: ClipCandidate,
        all_words: List[Dict],
        audio_energy: Optional[List[Tuple[float, float]]] = None,
    ):
        """Calculate all heuristic scores for a candidate."""
        
        # 1. Hook density (first 3 seconds)
        candidate.hook_score = self._calculate_hook_score(candidate)
        
        # 2. Pace (words/sec, cut density)
        candidate.pace_score = self._calculate_pace_score(candidate)
        
        # 3. Sentiment arc
        candidate.sentiment_score = self._calculate_sentiment_score(candidate)
        
        # 4. Self-containment (named entities resolved)
        candidate.containment_score = self._calculate_containment_score(candidate)
        
        # 5. Audio energy variance
        candidate.audio_energy_score = self._calculate_audio_energy_score(
            candidate, audio_energy
        )
        
        # 6. Duration compliance
        candidate.duration_score = self._calculate_duration_score(candidate)
        
        # Weighted combination
        w = self.weights
        candidate.heuristic_score = (
            w.hook * candidate.hook_score +
            w.pace * candidate.pace_score +
            w.sentiment * candidate.sentiment_score +
            w.containment * candidate.containment_score +
            w.audio_energy * candidate.audio_energy_score +
            w.duration * candidate.duration_score
        )
    
    def _calculate_hook_score(self, candidate: ClipCandidate) -> float:
        """Score based on hook patterns in first 3 seconds."""
        # Get words in first 3 seconds
        first_words = [w for w in candidate.words if w['start'] < candidate.start + 3]
        first_text = " ".join(w['word'] for w in first_words).lower()
        
        if not first_text:
            return 0.0
        
        score = 0.0
        for regex, weight in self.hook_regexes:
            matches = len(regex.findall(first_text))
            if matches > 0:
                score += weight * min(matches, 3) / 3  # Cap at 3 matches
        
        # Normalize to 0-1
        return min(score / 2.0, 1.0)
    
    def _calculate_pace_score(self, candidate: ClipCandidate) -> float:
        """Score based on speaking pace and silence ratio."""
        if not candidate.words:
            return 0.5
        
        # Words per second
        wps = len(candidate.words) / candidate.duration
        
        # Optimal range: 2.5-4 words/sec for Shorts
        if 2.5 <= wps <= 4.0:
            pace_score = 1.0
        elif 1.5 <= wps < 2.5 or 4.0 < wps <= 5.5:
            pace_score = 0.7
        else:
            pace_score = 0.3
        
        # Filler word penalty
        filler_count = sum(1 for w in candidate.words 
                          if w['word'].lower().strip('.,!?') in self.FILLER_WORDS)
        filler_ratio = filler_count / len(candidate.words) if candidate.words else 0
        filler_penalty = max(0, 1.0 - filler_ratio * 3)  # Heavy penalty for fillers
        
        return pace_score * filler_penalty
    
    def _calculate_sentiment_score(self, candidate: ClipCandidate) -> float:
        """Score based on sentiment arc (shift count + intensity)."""
        if not candidate.text:
            return 0.5
        
        # Split into sentences for arc analysis
        doc = nlp(candidate.text)
        sentences = [sent.text for sent in doc.sents]
        
        if len(sentences) < 2:
            return 0.5
        
        # Get sentiment for each sentence
        sentiments = [self.sia.polarity_scores(s)['compound'] for s in sentences]
        
        # Count sentiment shifts (polarity changes)
        shifts = 0
        for i in range(1, len(sentiments)):
            if sentiments[i] * sentiments[i-1] < 0:  # Sign change
                shifts += 1
        
        # Optimal: 1-3 shifts for a 15-60s clip
        shift_score = min(shifts / 2.0, 1.0) if shifts <= 3 else max(0, 1.0 - (shifts - 3) * 0.2)
        
        # Intensity (average absolute sentiment)
        intensity = sum(abs(s) for s in sentiments) / len(sentiments)
        intensity_score = min(intensity * 2, 1.0)
        
        return (shift_score + intensity_score) / 2
    
    def _calculate_containment_score(self, candidate: ClipCandidate) -> float:
        """Score based on self-containment (entities resolved within clip)."""
        if not candidate.text:
            return 0.5
        
        doc = nlp(candidate.text)
        
        # Count named entities
        entities = [(ent.text, ent.label_) for ent in doc.ents]
        
        if not entities:
            return 0.7  # No entities = self-contained by default
        
        # Check for unresolved references (pronouns without antecedents)
        pronouns = [token for token in doc if token.pos_ == "PRON" and token.text.lower() in 
                   {'he', 'she', 'it', 'they', 'this', 'that', 'these', 'those'}]
        
        # Simple heuristic: if many pronouns but few entities, likely unresolved
        unresolved_ratio = len(pronouns) / max(len(entities), 1)
        
        if unresolved_ratio > 2:
            return 0.3
        elif unresolved_ratio > 1:
            return 0.5
        else:
            return 0.8
    
    def _calculate_audio_energy_score(
        self, 
        candidate: ClipCandidate, 
        audio_energy: Optional[List[Tuple[float, float]]]
    ) -> float:
        """Score based on audio energy variance (peaks = emphasis)."""
        if not audio_energy:
            # Proxy: emphasis words in transcript
            emphasis_count = sum(1 for w in candidate.words 
                               if w['word'].lower().strip('.,!?') in self.EMPHASIS_WORDS)
            emphasis_ratio = emphasis_count / max(len(candidate.words), 1)
            return min(emphasis_ratio * 5, 1.0)
        
        # Filter energy points within clip window
        clip_energy = [e for t, e in audio_energy if candidate.start <= t <= candidate.end]
        
        if not clip_energy:
            return 0.5
        
        import statistics
        mean_energy = statistics.mean(clip_energy)
        stdev_energy = statistics.stdev(clip_energy) if len(clip_energy) > 1 else 0
        
        # Coefficient of variation - higher = more dynamic
        cv = stdev_energy / max(mean_energy, 1e-6)
        
        # Normalize: CV of 0.5-1.5 is good for engaging content
        if 0.5 <= cv <= 1.5:
            return 1.0
        elif 0.2 <= cv < 0.5 or 1.5 < cv <= 2.5:
            return 0.7
        else:
            return 0.4
    
    def _calculate_duration_score(self, candidate: ClipCandidate) -> float:
        """Score based on duration compliance (15-60s ideal)."""
        d = candidate.duration
        
        if 15 <= d <= 60:
            return 1.0
        elif 10 <= d < 15 or 60 < d <= 90:
            return 0.7
        elif 5 <= d < 10 or 90 < d <= 120:
            return 0.4
        else:
            return 0.1


class LLMReranker:
    """Rerank top candidates using local LLM (Ollama)."""
    
    def __init__(
        self,
        model: str = "qwen2.5:7b-instruct-q4_K_M",
        base_url: str = "http://localhost:11434",
        timeout: int = 30,
    ):
        self.model = model
        self.base_url = base_url
        self.timeout = timeout
    
    def rerank(
        self,
        candidates: List[ClipCandidate],
        context: str = "",
        top_k: int = 3,
    ) -> List[ClipCandidate]:
        """
        Rerank candidates using LLM.
        
        Args:
            candidates: List of ClipCandidate from heuristic analysis
            context: Channel niche, audience info
            top_k: Number of top clips to return
            
        Returns:
            Top-k candidates with viral_score, llm_reason, suggested_title, suggested_hook
        """
        import requests
        
        if not candidates:
            return []
        
        # Build prompt
        prompt = self._build_prompt(candidates, context)
        
        try:
            response = requests.post(
                f"{self.base_url}/api/generate",
                json={
                    "model": self.model,
                    "prompt": prompt,
                    "stream": False,
                    "options": {
                        "temperature": 0.3,
                        "top_p": 0.9,
                        "num_predict": 1024,
                    }
                },
                timeout=self.timeout
            )
            response.raise_for_status()
            result = response.json()
            
            # Parse JSON response
            return self._parse_llm_response(candidates, result.get('response', ''), top_k)
            
        except Exception as e:
            logger.error(f"LLM rerank failed: {e}")
            # Fallback: return top heuristic candidates
            for i, c in enumerate(candidates[:top_k]):
                c.viral_score = c.heuristic_score * 10
                c.llm_reason = "LLM unavailable, using heuristic score"
                c.suggested_title = f"Clip {i+1}"
                c.suggested_hook = c.text[:50] + "..."
            return candidates[:top_k]
    
    def _build_prompt(self, candidates: List[ClipCandidate], context: str) -> str:
        """Build the LLM prompt for reranking."""
        clips_text = ""
        for i, c in enumerate(candidates, 1):
            clips_text += f"""
Clip {i}:
  Time: {c.start:.1f}s - {c.end:.1f}s ({c.duration:.1f}s)
  Text: {c.text[:300]}
  Heuristic Score: {c.heuristic_score:.3f}
  Hook Score: {c.hook_score:.2f}
  Pace Score: {c.pace_score:.2f}
  Sentiment Score: {c.sentiment_score:.2f}
  Containment Score: {c.containment_score:.2f}
  Audio Energy Score: {c.audio_energy_score:.2f}
"""
        
        return f"""You are a Shorts retention editor. Rank these {len(candidates)} clips by viral potential.

Context: {context or "General educational/entertainment content"}

{clips_text}

For each clip, provide:
1. Score 1-10 (viral potential)
2. One-line reason
3. Suggested title (catchy, <60 chars)
4. Suggested hook (first 3 seconds text, <80 chars)

Return ONLY valid JSON array:
[
  {{"clip_index": 1, "score": 8.5, "reason": "...", "title": "...", "hook": "..."}},
  ...
]

Rank from highest to lowest viral potential. Be strict - only the best get 9+."""
    
    def _parse_llm_response(
        self, 
        candidates: List[ClipCandidate], 
        response: str, 
        top_k: int
    ) -> List[ClipCandidate]:
        """Parse LLM JSON response and update candidates."""
        import json
        import re
        
        # Extract JSON from response
        json_match = re.search(r'\[.*\]', response, re.DOTALL)
        if not json_match:
            logger.warning("No JSON found in LLM response")
            return candidates[:top_k]
        
        try:
            rankings = json.loads(json_match.group())
        except json.JSONDecodeError as e:
            logger.warning(f"Failed to parse LLM JSON: {e}")
            return candidates[:top_k]
        
        # Map rankings back to candidates
        ranked = []
        for item in rankings:
            idx = item.get('clip_index', 1) - 1
            if 0 <= idx < len(candidates):
                c = candidates[idx]
                c.viral_score = float(item.get('score', 0))
                c.llm_reason = item.get('reason', '')
                c.suggested_title = item.get('title', '')
                c.suggested_hook = item.get('hook', '')
                ranked.append(c)
        
        # Sort by viral score and return top_k
        ranked.sort(key=lambda c: c.viral_score, reverse=True)
        return ranked[:top_k]


def analyze_and_rank(
    transcript_json: str,
    audio_energy: Optional[List[Tuple[float, float]]] = None,
    context: str = "",
    top_k: int = 3,
    use_llm: bool = True,
) -> List[ClipCandidate]:
    """
    Main entry point: analyze transcript and return ranked clips.
    
    Args:
        transcript_json: Path to transcription JSON or JSON string
        audio_energy: Optional audio energy data
        context: Channel/audience context for LLM
        top_k: Number of final clips
        use_llm: Whether to use LLM rerank
        
    Returns:
        List of top ClipCandidate objects
    """
    import json
    
    # Load transcript
    if isinstance(transcript_json, str) and transcript_json.endswith('.json'):
        with open(transcript_json, 'r') as f:
            data = json.load(f)
    else:
        data = json.loads(transcript_json)
    
    segments = data.get('segments', [])
    words = []
    for seg in segments:
        words.extend(seg.get('words', []))
    
    # Heuristic analysis
    analyzer = HeuristicAnalyzer()
    candidates = analyzer.analyze_transcript(
        segments, words, audio_energy, target_clips=8
    )
    
    # LLM rerank
    if use_llm and candidates:
        reranker = LLMReranker()
        candidates = reranker.rerank(candidates, context, top_k)
    else:
        candidates = candidates[:top_k]
        for i, c in enumerate(candidates):
            c.viral_score = c.heuristic_score * 10
            c.suggested_title = f"Clip {i+1}"
            c.suggested_hook = c.text[:50] + "..."
    
    return candidates


if __name__ == "__main__":
    import argparse
    
    parser = argparse.ArgumentParser(description="Analyze transcript for viral clips")
    parser.add_argument("transcript", help="Transcription JSON file")
    parser.add_argument("-c", "--context", default="", help="Channel context")
    parser.add_argument("-k", "--top-k", type=int, default=3, help="Top clips to return")
    parser.add_argument("--no-llm", action="store_true", help="Skip LLM rerank")
    parser.add_argument("-o", "--output", help="Output JSON file")
    
    args = parser.parse_args()
    
    results = analyze_and_rank(
        args.transcript,
        context=args.context,
        top_k=args.top_k,
        use_llm=not args.no_llm,
    )
    
    output = {
        "clips": [
            {
                "start": c.start,
                "end": c.end,
                "duration": c.duration,
                "title": c.suggested_title,
                "hook": c.suggested_hook,
                "viral_score": c.viral_score,
                "heuristic_score": c.heuristic_score,
                "reason": c.llm_reason,
                "breakdown": {
                    "hook": c.hook_score,
                    "pace": c.pace_score,
                    "sentiment": c.sentiment_score,
                    "containment": c.containment_score,
                    "audio_energy": c.audio_energy_score,
                    "duration": c.duration_score,
                }
            }
            for c in results
        ]
    }
    
    json_str = json.dumps(output, indent=2)
    
    if args.output:
        with open(args.output, 'w') as f:
            f.write(json_str)
    else:
        print(json_str)