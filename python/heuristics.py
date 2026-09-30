import sys
import json
import os
import numpy as np
from typing import List, Tuple, Dict, Any

# Add local site-packages if needed
# sys.path.insert(0, os.path.join(os.path.dirname(__file__), 'venv', 'Lib', 'site-packages'))

def main():
    if len(sys.argv) < 4:
        print(json.dumps({"error": "Usage: python heuristics.py <transcript_json> <start_s> <end_s>"}))
        sys.exit(1)
    
    transcript_path = sys.argv[1]
    clip_start = float(sys.argv[2])
    clip_end = float(sys.argv[3])
    
    # Load transcript
    with open(transcript_path, 'r') as f:
        transcript = json.load(f)
    
    # Calculate heuristics
    heuristic = calculate_heuristics(transcript, clip_start, clip_end)
    print(json.dumps(heuristic))

def extract_words(transcript):
    words = []
    for segment in transcript.get('segments', []):
        for word in segment.get('words', []):
            words.append(word)
    return words

def calculate_heuristics(transcript, start: float, end: float) -> Dict[str, Any]:
    duration = end - start
    
    # Get words in range
    words = extract_words(transcript)
    words_in_range = [w for w in words if w['start'] >= start and w['end'] <= end]
    
    # 1. Hook density (first 3 seconds)
    hook_density = calculate_hook_density(transcript, start)
    
    # 2. Pace (words per minute)
    pace = calculate_pace(words_in_range, duration)
    
    # 3. Sentiment arc
    sentiment_arc = calculate_sentiment_arc(transcript, start, end)
    
    # 4. Self-containment
    self_containment = calculate_self_containment(transcript, start, end)
    
    # 5. Audio energy variance (placeholder - would need actual audio analysis)
    audio_energy_variance = 0.5  # Placeholder
    
    # 6. Duration compliance
    duration_compliance = 1.0 if 15.0 <= duration <= 60.0 else 0.0
    
    # Weighted total
    total = (
        hook_density * 0.25 +
        pace * 0.20 +
        sentiment_arc * 0.15 +
        self_containment * 0.15 +
        audio_energy_variance * 0.15 +
        duration_compliance * 0.10
    )
    
    return {
        "hook_density": hook_density,
        "pace": pace,
        "sentiment_arc": sentiment_arc,
        "self_containment": self_containment,
        "audio_energy_variance": audio_energy_variance,
        "duration_compliance": duration_compliance,
        "total": total
    }

def calculate_hook_density(transcript, clip_start: float) -> float:
    hook_keywords = [
        "you", "stop", "wait", "watch", "look", "listen",
        "secret", "trick", "hack", "mistake", "wrong",
        "never", "always", "must", "need", "shocking",
        "unbelievable", "incredible", "amazing", "crazy",
        "insane", "wild", "mind", "blown", "game changer",
        "?", "!", "how", "why", "what", "when", "who",
    ]
    
    first_3_seconds = []
    for segment in transcript.get('segments', []):
        if segment['start'] >= clip_start and segment['start'] < clip_start + 3.0:
            for word in segment.get('words', []):
                first_3_seconds.append(word)
    
    if not first_3_seconds:
        return 0.0
    
    hook_count = sum(1 for w in first_3_seconds 
                    if any(k in w['text'].lower() for k in hook_keywords))
    
    return min(1.0, hook_count / len(first_3_seconds))

def calculate_pace(words: List[Dict], duration: float) -> float:
    if not words or duration == 0.0:
        return 0.0
    
    wpm = len(words) / duration * 60.0
    
    # Optimal pace for Shorts: 150-200 WPM
    optimal = 175.0
    score = 1.0 - (abs(wpm - optimal) / optimal)
    
    return max(0.0, min(1.0, score))

def calculate_sentiment_arc(transcript, start: float, end: float) -> float:
    positive_words = [
        "good", "great", "amazing", "awesome", "love", "best", "perfect",
        "happy", "excited", "win", "success", "beautiful", "wonderful",
    ]
    
    negative_words = [
        "bad", "terrible", "awful", "hate", "worst", "wrong", "fail",
        "sad", "angry", "lose", "failure", "ugly", "horrible",
    ]
    
    sentiment_scores = []
    
    for segment in transcript.get('segments', []):
        if segment['start'] >= end or segment['end'] <= start:
            continue
        
        text = segment['text'].lower()
        words = text.split()
        
        if not words:
            continue
        
        pos_count = sum(1 for w in words if w in positive_words)
        neg_count = sum(1 for w in words if w in negative_words)
        
        score = (pos_count - neg_count) / len(words)
        sentiment_scores.append(max(-1.0, min(1.0, score)))
    
    if not sentiment_scores:
        return 0.5
    
    # Calculate variance (more shifts = more engaging)
    mean = sum(sentiment_scores) / len(sentiment_scores)
    variance = sum((s - mean) ** 2 for s in sentiment_scores) / len(sentiment_scores)
    
    return min(1.0, variance * 4.0)

def calculate_self_containment(transcript, start: float, end: float) -> float:
    pronoun_words = [
        "he", "she", "it", "they", "this", "that", "these", "those",
        "his", "her", "their", "its", "him", "them",
    ]
    
    entity_indicators = [
        "said", "says", "told", "explained", "mentioned", "called", "named",
        "is a", "was a", "who is", "which is", "known as",
    ]
    
    full_text = ""
    for segment in transcript.get('segments', []):
        if segment['start'] >= end or segment['end'] <= start:
            continue
        full_text += segment['text'] + " "
    
    full_text = full_text.lower()
    words = full_text.split()
    
    if not words:
        return 0.5
    
    pronoun_count = sum(1 for w in words if w in pronoun_words)
    entity_count = sum(1 for ind in entity_indicators if ind in full_text)
    
    if pronoun_count == 0:
        return 1.0
    
    resolution_rate = entity_count / pronoun_count
    return min(1.0, resolution_rate * 2.0)

if __name__ == "__main__":
    main()