use anyhow::Result;
use crate::common::{Clip, Transcript, Word, HeuristicScores};

pub async fn analyze_and_score(
    transcript: &Transcript,
    max_clips: usize,
) -> Result<Vec<Clip>> {
    // Find candidate segments using sliding window
    let candidates = find_candidate_segments(transcript, max_clips * 3)?;

    // Score each candidate
    let mut scored_candidates = Vec::new();
    for (start, end) in candidates {
        let duration = end - start;
        if duration < 15.0 || duration > 60.0 {
            continue;
        }

        let heuristic = calculate_heuristics(transcript, start, end);
        let clip = Clip::new(
            "temp".to_string(), // project_id will be set later
            start,
            end,
            format!("Clip {}", scored_candidates.len() + 1),
        );
        
        scored_candidates.push((clip, heuristic));
    }

    // Sort by heuristic score descending
    scored_candidates.sort_by(|a, b| b.1.total.partial_cmp(&a.1.total).unwrap_or(std::cmp::Ordering::Equal));

    // Take top N
    let mut clips: Vec<Clip> = scored_candidates.into_iter()
        .take(max_clips)
        .map(|(mut clip, heuristic)| {
            clip.hook_score = Some(heuristic.hook_density);
            clip.heuristic_json = Some(serde_json::to_string(&heuristic).unwrap_or_default());
            clip
        })
        .collect();

    // Assign project_id (will be overridden by pipeline)
    for clip in &mut clips {
        clip.project_id = "temp".to_string();
    }

    Ok(clips)
}

fn find_candidate_segments(transcript: &Transcript, max_candidates: usize) -> Result<Vec<(f64, f64)>> {
    let mut candidates = Vec::new();
    let window_size = 30.0; // 30 second windows
    let step = 5.0; // 5 second steps

    let duration = transcript.duration;
    let mut start = 0.0;

    while start + window_size <= duration && candidates.len() < max_candidates {
        let end = start + window_size;
        candidates.push((start, end));
        start += step;
    }

    // Also add segments based on speech patterns (pauses)
    let words: Vec<&Word> = transcript.segments.iter()
        .flat_map(|s| s.words.iter())
        .collect();

    let mut last_end = 0.0;
    for word in words {
        if word.start - last_end > 2.0 { // 2 second pause
            if word.start - last_end >= 15.0 {
                candidates.push((last_end, word.start));
            }
        }
        last_end = word.end;
    }

    // Deduplicate and sort
    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    // Manual dedup since dedup_by requires mutable references
    let mut deduped = Vec::new();
    for candidate in candidates {
        if !deduped.iter().any(|c: &(f64, f64)| {
            (c.0 - candidate.0).abs() < 1.0 && (c.1 - candidate.1).abs() < 1.0
        }) {
            deduped.push(candidate);
        }
    }
    candidates = deduped;

    Ok(candidates)
}

fn calculate_heuristics(transcript: &Transcript, start: f64, end: f64) -> HeuristicScores {
    let segment_words: Vec<&Word> = transcript.segments.iter()
        .flat_map(|s| s.words.iter())
        .filter(|w| w.start >= start && w.end <= end)
        .collect();

    let duration = end - start;
    
    // 1. Hook density (first 3 seconds)
    let hook_density = calculate_hook_density(transcript, start);
    
    // 2. Pace (words per minute)
    let pace = if duration > 0.0 {
        (segment_words.len() as f64 / duration) * 60.0
    } else { 0.0 };
    
    // 3. Sentiment arc (simplified)
    let sentiment_arc = calculate_sentiment_arc(&segment_words);
    
    // 4. Self-containment (named entities resolved)
    let self_containment = calculate_self_containment(&segment_words);
    
    // 5. Audio energy variance (would need audio analysis, placeholder)
    let audio_energy_variance = 0.5;
    
    // 6. Duration compliance
    let duration_compliance = if duration >= 15.0 && duration <= 60.0 { 1.0 } else { 0.0 };

    // Weighted total (matching plan weights)
    let total = 
        hook_density * 0.25 +
        (pace / 180.0).min(1.0) * 0.20 +
        sentiment_arc * 0.15 +
        self_containment * 0.15 +
        audio_energy_variance * 0.15 +
        duration_compliance * 0.10;

    HeuristicScores {
        hook_density,
        pace,
        sentiment_arc,
        self_containment,
        audio_energy_variance,
        duration_compliance,
        total,
    }
}

fn calculate_hook_density(transcript: &Transcript, segment_start: f64) -> f64 {
    let hook_keywords = [
        "you", "stop", "wait", "watch", "look", "listen",
        "secret", "trick", "hack", "mistake", "wrong",
        "never", "always", "must", "need", "shocking",
        "unbelievable", "incredible", "amazing", "crazy",
        "insane", "wild", "mind", "blown", "game changer",
    ];

    let first_3_seconds = transcript.segments.iter()
        .filter(|s| s.start < segment_start + 3.0)
        .flat_map(|s| s.words.iter())
        .collect::<Vec<_>>();

    if first_3_seconds.is_empty() {
        return 0.0;
    }

    let hook_count = first_3_seconds.iter()
        .filter(|w| {
            let text = w.text.to_lowercase();
            hook_keywords.iter().any(|k| text.contains(k))
        })
        .count();

    hook_count as f64 / first_3_seconds.len() as f64
}

fn calculate_sentiment_arc(words: &[&Word]) -> f64 {
    // Simplified: count positive/negative words
    let positive = ["good", "great", "awesome", "amazing", "love", "best", "happy", "excited", "win", "success"];
    let negative = ["bad", "terrible", "awful", "hate", "worst", "sad", "angry", "fail", "wrong", "problem"];
    
    let text = words.iter()
        .map(|w| w.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    
    let pos_count = positive.iter().filter(|w| text.contains(*w)).count();
    let neg_count = negative.iter().filter(|w| text.contains(*w)).count();
    
    if pos_count + neg_count == 0 {
        0.5
    } else {
        pos_count as f64 / (pos_count + neg_count) as f64
    }
}

fn calculate_self_containment(words: &[&Word]) -> f64 {
    // Check for unresolved pronouns (he, she, it, they, this, that without context)
    let pronouns = ["he", "she", "it", "they", "them", "this", "that", "these", "those"];
    let text = words.iter()
        .map(|w| w.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    
    let pronoun_count = pronouns.iter().filter(|w| text.contains(*w)).count();
    let total_words = words.len().max(1);
    
    // Lower score if many pronouns (less self-contained)
    1.0 - (pronoun_count as f64 / total_words as f64).min(1.0)
}