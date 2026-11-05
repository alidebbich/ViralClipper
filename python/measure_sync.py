#!/usr/bin/env python3
"""
measure_sync.py - Caption/Voice Sync Measurement Tool (Phase 1.1)

Measures the synchronization between burned/ASS captions and the actual spoken voice in the rendered audio.
Supports:
  1. Real clip mode:
     python python/measure_sync.py <clip.mp4> [clip.ass] [--language ar]
  2. TTS known-boundary fixture mode:
     python python/measure_sync.py --fixture [ar|fr|en]
"""

import sys, os, re, json, time, subprocess, tempfile, argparse, statistics, urllib.request, urllib.error, uuid, asyncio
from pathlib import Path
from typing import Optional, List, Dict, Any

# Ensure stdout handles UTF-8 on Windows
if sys.platform == "win32":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    except Exception:
        pass

PROJECT_ROOT = Path(__file__).resolve().parent.parent

def read_env_key(name: str) -> Optional[str]:
    val = os.environ.get(name, "").strip()
    if val:
        return val
    env_file = PROJECT_ROOT / ".env"
    if env_file.exists():
        for line in env_file.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line.startswith(f"{name}="):
                v = line[len(name)+1:].strip().strip('"').strip("'")
                if v:
                    return v
    return None

GROQ_API_KEY = read_env_key("GROQ_API_KEY")

def find_bin(name: str) -> str:
    local = os.environ.get("LOCALAPPDATA", "")
    wg = os.path.join(local, "Microsoft", "WinGet", "Links", name + ".exe")
    if os.path.exists(wg):
        return wg
    return name

FFMPEG = find_bin("ffmpeg")
FFPROBE = find_bin("ffprobe")

def extract_audio(clip_path: str, out_wav: str) -> float:
    r = subprocess.run(
        [FFMPEG, "-y", "-i", clip_path, "-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1", out_wav],
        capture_output=True, timeout=120
    )
    if r.returncode != 0:
        raise RuntimeError(f"FFmpeg failed: {r.stderr.decode('utf-8', 'ignore')[-300:]}")
    r2 = subprocess.run(
        [FFPROBE, "-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", out_wav],
        capture_output=True, text=True, timeout=15
    )
    return float(r2.stdout.strip()) if r2.returncode == 0 else 0.0

def compress_for_groq(wav: str, out_ogg: str) -> str:
    r = subprocess.run(
        [FFMPEG, "-y", "-i", wav, "-ac", "1", "-ar", "16000", "-c:a", "libopus", "-b:a", "32k", out_ogg],
        capture_output=True, timeout=120
    )
    if r.returncode == 0 and os.path.exists(out_ogg) and os.path.getsize(out_ogg) > 300:
        return out_ogg
    mp3 = out_ogg.replace(".ogg", ".mp3")
    subprocess.run(
        [FFMPEG, "-y", "-i", wav, "-ac", "1", "-ar", "16000", "-c:a", "libmp3lame", "-b:a", "32k", mp3],
        capture_output=True, timeout=120
    )
    return mp3

def transcribe_with_groq(audio_path: str, language: str = "ar") -> List[Dict[str, Any]]:
    key = read_env_key("GROQ_API_KEY")
    if not key:
        raise RuntimeError("GROQ_API_KEY not found in environment or .env")
    ext = os.path.splitext(audio_path)[1].lower()
    ct = {"ogg": "audio/ogg", "mp3": "audio/mpeg", "wav": "audio/wav"}.get(ext.lstrip("."), "audio/wav")
    boundary = uuid.uuid4().hex
    with open(audio_path, "rb") as f:
        audio_bytes = f.read()
    parts = [
        f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="audio{ext}"\r\nContent-Type: {ct}\r\n\r\n'.encode(),
        audio_bytes,
        f'\r\n--{boundary}\r\nContent-Disposition: form-data; name="model"\r\n\r\nwhisper-large-v3-turbo\r\n'.encode(),
        f'--{boundary}\r\nContent-Disposition: form-data; name="response_format"\r\n\r\nverbose_json\r\n'.encode(),
        f'--{boundary}\r\nContent-Disposition: form-data; name="timestamp_granularities[]"\r\n\r\nword\r\n'.encode(),
        f'--{boundary}\r\nContent-Disposition: form-data; name="timestamp_granularities[]"\r\n\r\nsegment\r\n'.encode(),
    ]
    if language and language != "auto":
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="language"\r\n\r\n{language}\r\n'.encode())
    parts.append(f'--{boundary}--\r\n'.encode())
    body = b"".join(parts)

    for attempt in range(3):
        try:
            req = urllib.request.Request(
                "https://api.groq.com/openai/v1/audio/transcriptions",
                data=body,
                headers={
                    "Authorization": f"Bearer {key}",
                    "Content-Type": f"multipart/form-data; boundary={boundary}",
                    "User-Agent": "ViralClipperSyncTool/1.0"
                },
                method="POST"
            )
            with urllib.request.urlopen(req, timeout=120) as resp:
                result = json.loads(resp.read().decode("utf-8"))
            words = result.get("words") or []
            if not words:
                for seg in result.get("segments", []):
                    words.extend(seg.get("words", []))
            return words
        except urllib.error.HTTPError as e:
            err = e.read().decode("utf-8", "ignore")
            if e.code == 429 and attempt < 2:
                time.sleep(2 ** (attempt + 1))
                continue
            raise RuntimeError(f"Groq HTTP {e.code}: {err[:250]}")
    return []

def ts_to_ms(ts: str) -> int:
    m = re.match(r"(\d+):(\d{2}):(\d{2})\.(\d{2})", ts)
    if not m:
        return 0
    h, mi, s, cs = int(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4))
    return (h * 3600 + mi * 60 + s) * 1000 + cs * 10

def ms_to_ts(ms: int) -> str:
    s, ms2 = divmod(ms, 1000)
    m, s = divmod(s, 60)
    h, m = divmod(m, 60)
    return f"{h}:{m:02}:{s:02}.{ms2//10:02}"

def parse_ass_events(ass_path: str) -> List[Dict[str, Any]]:
    content = Path(ass_path).read_text(encoding="utf-8", errors="replace")
    events = []
    for line in content.splitlines():
        if not line.startswith("Dialogue:"):
            continue
        parts = line.split(",", 9)
        if len(parts) < 10:
            continue
        style = parts[3].strip()
        if style.lower() == "hook":
            continue
        clean = re.sub(r"\{[^}]*\}", "", parts[9]).strip()
        if not clean:
            continue
        events.append({
            "style": style,
            "start_ms": ts_to_ms(parts[1].strip()),
            "end_ms": ts_to_ms(parts[2].strip()),
            "text": clean
        })
    return events

def probe_clip(mp4: str) -> Dict[str, Any]:
    r = subprocess.run(
        [FFPROBE, "-v", "quiet", "-print_format", "json", "-show_format", "-show_streams", mp4],
        capture_output=True, text=True, timeout=30
    )
    info = {}
    if r.returncode == 0:
        try:
            j = json.loads(r.stdout)
            fmt = j.get("format", {})
            info["container_start"] = float(fmt.get("start_time", 0))
            info["duration"] = float(fmt.get("duration", 0))
            for s in j.get("streams", []):
                if s.get("codec_type") == "video":
                    info["video_start_time"] = float(s.get("start_time", 0))
                elif s.get("codec_type") == "audio":
                    info["audio_start_time"] = float(s.get("start_time", 0))
                    info["audio_initial_padding"] = int(s.get("initial_padding", 0))
                    info["audio_sample_rate"] = int(s.get("sample_rate", 48000))
        except Exception as e:
            info["err"] = str(e)
    return info

def match_words_to_events(words: List[Dict[str, Any]], events: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    rows = []
    for w in words:
        word_text = w.get("word", w.get("text", "")).strip()
        w_ms = int(round(w.get("start", 0) * 1000))
        best_ev, best_dist = None, float("inf")
        for ev in events:
            dist = abs(ev["start_ms"] - w_ms)
            if dist < best_dist:
                best_dist = dist
                best_ev = ev
        if best_ev and best_dist < 3000:
            rows.append({
                "word": word_text,
                "audio_ms": w_ms,
                "caption_ms": best_ev["start_ms"],
                "offset": best_ev["start_ms"] - w_ms,
                "dist": best_dist,
                "matched": True
            })
        else:
            rows.append({
                "word": word_text,
                "audio_ms": w_ms,
                "caption_ms": -1,
                "offset": 0,
                "dist": -1,
                "matched": False
            })
    return rows

def compute_stats(rows: List[Dict[str, Any]]) -> Dict[str, Any]:
    matched = [r["offset"] for r in rows if r["matched"]]
    if not matched:
        return {"count": 0, "pass_median": False, "pass_p95": False}
    abs_m = sorted([abs(o) for o in matched])
    n = len(matched)
    median_val = round(statistics.median(matched), 1)
    median_abs = round(statistics.median(abs_m), 1)
    p95_abs = round(abs_m[int(n * 0.95)] if n > 0 else 0, 1)
    mean_val = round(statistics.mean(matched), 1)
    return {
        "count": n,
        "median_ms": median_val,
        "median_abs_ms": median_abs,
        "p95_abs_ms": p95_abs,
        "mean_ms": mean_val,
        "min_ms": min(matched),
        "max_ms": max(matched),
        "late_pct": round(100.0 * sum(1 for o in matched if o > 80) / n, 1),
        "early_pct": round(100.0 * sum(1 for o in matched if o < -80) / n, 1),
        "pass_median": abs(median_val) < 80,
        "pass_p95": p95_abs < 150,
    }

async def generate_tts_fixture(voice: str = "ar-TN-HediNeural", text: Optional[str] = None) -> Dict[str, Any]:
    import edge_tts
    if text is None:
        if "ar" in voice:
            text = "مرحبا بكم في هذا المقطع التجريبي. نحن نقوم باختبار مزامنة الكلمات الصوتية بدقة متناهية."
        elif "fr" in voice:
            text = "Bienvenue dans ce test de synchronisation. Nous vérifions la précision des sous-titres et du son."
        else:
            text = "Welcome to this test fixture. We are testing precise word synchronization and timing."
    
    comm = edge_tts.Communicate(text, voice, boundary="WordBoundary")
    word_boundaries = []
    audio_chunks = []
    async for chunk in comm.stream():
        if chunk["type"] == "WordBoundary":
            word_boundaries.append({
                "word": chunk["text"],
                "start": chunk["offset"] / 10_000_000.0,
                "end": (chunk["offset"] + chunk["duration"]) / 10_000_000.0,
                "start_ms": int(chunk["offset"] / 10_000),
                "end_ms": int((chunk["offset"] + chunk["duration"]) / 10_000),
            })
        elif chunk["type"] == "audio":
            audio_chunks.append(chunk["data"])

    return {
        "text": text,
        "voice": voice,
        "words": word_boundaries,
        "audio_bytes": b"".join(audio_chunks)
    }

def create_ass_from_words(words: List[Dict[str, Any]], ass_path: str, is_arabic: bool = True):
    font_name = "Cairo" if is_arabic else "Arial"
    encoding = 178 if is_arabic else 1
    lines = [
        "[Script Info]",
        "ScriptType: v4.00+",
        "PlayResX: 1080",
        "PlayResY: 1920",
        "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        f"Style: Caption,{font_name},64,&H00FFFFFF,&H0000FFFF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,3,1,2,40,40,150,{encoding}",
        "",
        "[Events]",
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    ]
    for w in words:
        start_ts = ms_to_ts(w["start_ms"])
        end_ts = ms_to_ts(w["end_ms"])
        lines.append(f"Dialogue: 0,{start_ts},{end_ts},Caption,,0,0,0,,{w['word']}")
    Path(ass_path).write_text("\n".join(lines), encoding="utf-8")

def run_fixture_sync_test(voice: str = "ar-TN-HediNeural"):
    print(f"\n{'='*70}\nTTS KNOWN-BOUNDARY FIXTURE SYNC TEST ({voice})\n{'='*70}")
    with tempfile.TemporaryDirectory() as tmp:
        fixture_data = asyncio.run(generate_tts_fixture(voice=voice))
        known_words = fixture_data["words"]
        print(f"Generated TTS audio: {len(fixture_data['audio_bytes'])} bytes, {len(known_words)} known word boundaries.")
        for kw in known_words:
            print(f"  Ground truth: {kw['start_ms']:5d}ms - {kw['end_ms']:5d}ms [{kw['word']}]")

        mp3_path = os.path.join(tmp, "tts.mp3")
        with open(mp3_path, "wb") as f:
            f.write(fixture_data["audio_bytes"])

        ass_path = os.path.join(tmp, "tts.ass")
        create_ass_from_words(known_words, ass_path, is_arabic=("ar" in voice))

        # Render test video
        video_path = os.path.join(tmp, "tts_test.mp4")
        dur = (known_words[-1]["end_ms"] / 1000.0) + 1.0
        escaped_ass = ass_path.replace("\\", "/").replace(":", "\\:")
        render_cmd = [
            FFMPEG, "-y",
            "-f", "lavfi", "-i", f"color=c=0x111827:s=1080x1920:r=30:d={dur:.2f}",
            "-i", mp3_path,
            "-vf", f"ass='{escaped_ass}'",
            "-c:v", "libx264", "-preset", "ultrafast",
            "-c:a", "aac", "-b:a", "128k", "-ar", "48000",
            "-shortest", video_path
        ]
        res = subprocess.run(render_cmd, capture_output=True, timeout=60)
        if res.returncode != 0:
            raise RuntimeError(f"FFmpeg render failed: {res.stderr.decode('utf-8', 'ignore')[-300:]}")
        print(f"Rendered test video: {os.path.getsize(video_path)//1024} KB")

        # Now transcribe the rendered video's audio with Groq
        wav_path = os.path.join(tmp, "rendered.wav")
        extract_audio(video_path, wav_path)
        comp = compress_for_groq(wav_path, os.path.join(tmp, "comp.ogg"))
        print("Transcribing with Groq Whisper...")
        lang = "ar" if "ar" in voice else ("fr" if "fr" in voice else "en")
        groq_words = transcribe_with_groq(comp, language=lang)
        print(f"Groq returned {len(groq_words)} words:")
        for gw in groq_words:
            print(f"  Groq: {int(gw['start']*1000):5d}ms - {int(gw['end']*1000):5d}ms [{gw.get('word','?')}]")

        # Compare Groq vs Known TTS Ground Truth
        print("\n--- 1. Groq Word Timestamps vs Known TTS Ground Truth ---")
        rows_gw_vs_tts = []
        for kw in known_words:
            k_ms = kw["start_ms"]
            best_gw, best_d = None, float("inf")
            for gw in groq_words:
                g_ms = int(gw["start"] * 1000)
                d = abs(g_ms - k_ms)
                if d < best_d:
                    best_d = d
                    best_gw = gw
            if best_gw and best_d < 3000:
                rows_gw_vs_tts.append({
                    "word": kw["word"],
                    "ground_truth_ms": k_ms,
                    "groq_ms": int(best_gw["start"] * 1000),
                    "offset": int(best_gw["start"] * 1000) - k_ms,
                    "matched": True
                })
        print(f"  {'Word':<15} {'GroundTruth ms':>15} {'Groq ms':>10} {'Δ ms':>8}")
        for r in rows_gw_vs_tts:
            print(f"  {r['word']:<15} {r['ground_truth_ms']:>15} {r['groq_ms']:>10} {r['offset']:>+8}")

        # Compare Groq vs Burned ASS Captions
        ass_events = parse_ass_events(ass_path)
        rows_sync = match_words_to_events(groq_words, ass_events)
        stats = compute_stats(rows_sync)
        print("\n--- 2. End-to-End Fixture Sync (Groq vs ASS) ---")
        print(f"  Median |offset|: {stats.get('median_abs_ms')} ms (target < 80 ms: {'PASS' if stats.get('pass_median') else 'FAIL'})")
        print(f"  p95   |offset|: {stats.get('p95_abs_ms')} ms (target < 150 ms: {'PASS' if stats.get('pass_p95') else 'FAIL'})")
        return stats

def run_clip_sync_test(clip_path: str, ass_path: Optional[str] = None, language: str = "ar") -> Dict[str, Any]:
    clip_id = Path(clip_path).stem
    print(f"\n{'='*70}\n[PLUMBING / SELF-CONSISTENCY CHECK — NOT PROOF OF ACOUSTIC SYNC]\nMEASURING SYNC FOR CLIP: {clip_id}\n{'='*70}")

    if not ass_path:
        # Default locations
        candidates = [
            Path(clip_path).with_suffix(".ass"),
            Path(os.environ.get("LOCALAPPDATA", "")) / "clipper" / "ass" / f"{clip_id}.ass",
        ]
        for c in candidates:
            if c.exists():
                ass_path = str(c)
                break
    if not ass_path or not os.path.exists(ass_path):
        raise FileNotFoundError(f"ASS file not found for {clip_path}")

    print(f"Clip: {clip_path}")
    print(f"ASS:  {ass_path}")

    print("\n[1] Probing rendered container & streams...")
    probe = probe_clip(clip_path)
    sr = probe.get("audio_sample_rate", 48000)
    pad = probe.get("audio_initial_padding", 0)
    aac_delay_ms = (pad / sr * 1000) if sr else 0.0
    print(f"  container start_time: {probe.get('container_start', 0.0)} s")
    print(f"  video start_time:     {probe.get('video_start_time', 0.0)} s")
    print(f"  audio start_time:     {probe.get('audio_start_time', 0.0)} s")
    print(f"  audio initial_padding: {pad} samples @ {sr} Hz ({aac_delay_ms:.1f} ms AAC delay)")
    print(f"  duration:             {probe.get('duration', 0.0):.2f} s")

    with tempfile.TemporaryDirectory() as tmp:
        wav_path = os.path.join(tmp, "audio.wav")
        print("\n[2] Extracting audio from rendered clip...")
        dur = extract_audio(clip_path, wav_path)
        print(f"  Extracted {dur:.2f}s WAV ({os.path.getsize(wav_path)//1024} KB)")

        print("\n[3] Transcribing rendered audio via Groq (whisper-large-v3-turbo)...")
        comp = compress_for_groq(wav_path, os.path.join(tmp, "audio.ogg"))
        groq_words = transcribe_with_groq(comp, language=language)
        print(f"  Received {len(groq_words)} words from Groq Whisper")

    print("\n[4] Parsing ASS events...")
    ass_events = parse_ass_events(ass_path)
    print(f"  Loaded {len(ass_events)} dialogue events from ASS")

    print("\n[5] Matching audio words with caption events...")
    rows = match_words_to_events(groq_words, ass_events)
    stats = compute_stats(rows)

    print(f"\n[6] Per-Word Table (first 30 words):")
    print(f"  {'Word':<18} {'Audio ms':>10} {'Caption ms':>12} {'Δ ms':>8}  Status")
    print(f"  {'-'*18} {'-'*10} {'-'*12} {'-'*8}  ------")
    shown = 0
    for r in rows:
        if not r["matched"]:
            continue
        flag = "OK" if abs(r["offset"]) < 80 else ("LATE ◄" if r["offset"] > 0 else "EARLY ◄")
        print(f"  {r['word'][:18]:<18} {r['audio_ms']:>10} {r['caption_ms']:>12} {r['offset']:>+8}  {flag}")
        shown += 1
        if shown >= 30:
            if len(rows) > 30:
                print(f"  ... (+{len(rows) - 30} more words)")
            break

    print(f"\n{'='*70}\n[PLUMBING / SELF-CONSISTENCY RESULTS]\n{'='*70}")
    print(f"  Matched words:    {stats.get('count')}")
    print(f"  Median offset:    {stats.get('median_ms'):+g} ms")
    print(f"  Median |offset|:  {stats.get('median_abs_ms')} ms  (Target < 80 ms:  {'PASS' if stats.get('pass_median') else 'FAIL'})")
    print(f"  p95   |offset|:   {stats.get('p95_abs_ms')} ms  (Target < 150 ms: {'PASS' if stats.get('pass_p95') else 'FAIL'})")
    print(f"  Mean offset:      {stats.get('mean_ms'):+g} ms")
    print(f"  Min / Max:        {stats.get('min_ms'):+g} ms / {stats.get('max_ms'):+g} ms")
    print(f"  Late (>+80ms):    {stats.get('late_pct')}%")
    print(f"  Early (<-80ms):   {stats.get('early_pct')}%")
    print(f"{'='*70}")

    evidence_dir = PROJECT_ROOT / "docs" / "evidence"
    evidence_dir.mkdir(parents=True, exist_ok=True)
    report_file = evidence_dir / f"sync_report_{clip_id}.json"
    csv_file = evidence_dir / f"sync_{clip_id}.csv"

    out_data = {
        "check_type": "plumbing_self_consistency",
        "clip_id": clip_id,
        "clip_path": clip_path,
        "ass_path": ass_path,
        "probe": probe,
        "stats": stats,
        "rows": rows
    }
    report_file.write_text(json.dumps(out_data, ensure_ascii=False, indent=2), encoding="utf-8")
    with open(csv_file, "w", encoding="utf-8") as f:
        f.write("word,audio_ms,caption_ms,offset_ms,matched\n")
        for r in rows:
            f.write(f'"{r["word"]}",{r["audio_ms"]},{r["caption_ms"]},{r["offset"]},{r["matched"]}\n')
    print(f"Saved JSON report: {report_file}")
    print(f"Saved CSV report:  {csv_file}")
    return stats

def run_independent_sync_test(clip_path: str, ass_path: Optional[str] = None, language: str = "ar") -> Dict[str, Any]:
    clip_id = Path(clip_path).stem
    print(f"\n{'='*70}\n[INDEPENDENT SYNC CHECK - FASTER-WHISPER ALIGNER]\nCLIP: {clip_id}\n{'='*70}")

    if not ass_path:
        candidates = [
            Path(clip_path).with_suffix(".ass"),
            Path(os.environ.get("LOCALAPPDATA", "")) / "clipper" / "ass" / f"{clip_id}.ass",
        ]
        for c in candidates:
            if c.exists():
                ass_path = str(c)
                break
    if not ass_path or not os.path.exists(ass_path):
        raise FileNotFoundError(f"ASS file not found for {clip_path}")

    from faster_whisper import WhisperModel
    model_path = os.path.join(os.path.expanduser("~"), r".cache\huggingface\hub\models--Systran--faster-whisper-base\snapshots\ebe41f70d5b6dfa9166e2c581c45c9c0cfc57b66")
    if not os.path.exists(model_path):
        model_path = "base"

    with tempfile.TemporaryDirectory() as tmp:
        wav_path = os.path.join(tmp, "audio.wav")
        extract_audio(clip_path, wav_path)
        print("Running independent local faster-whisper base aligner on CPU...")
        t0 = time.time()
        model = WhisperModel(model_path, device="cpu", compute_type="int8")
        segments, info = model.transcribe(wav_path, word_timestamps=True, language=language if language != "auto" else None)
        fw_words = []
        for s in segments:
            for w in (s.words or []):
                fw_words.append({
                    "word": w.word.strip(),
                    "start": w.start,
                    "end": w.end,
                    "start_ms": int(round(w.start * 1000)),
                    "end_ms": int(round(w.end * 1000)),
                })
        print(f"Faster-whisper aligned {len(fw_words)} words in {time.time()-t0:.1f}s")

    ass_events = parse_ass_events(ass_path)
    rows = []
    for fw in fw_words:
        f_ms = fw["start_ms"]
        best_ev, best_d = None, float("inf")
        for ev in ass_events:
            d = abs(ev["start_ms"] - f_ms)
            if d < best_d:
                best_d = d
                best_ev = ev
        if best_ev and best_d < 3000:
            rows.append({
                "word": fw["word"],
                "audio_ms": f_ms,
                "caption_ms": best_ev["start_ms"],
                "offset": best_ev["start_ms"] - f_ms,
                "dist": best_d,
                "matched": True
            })
        else:
            rows.append({
                "word": fw["word"],
                "audio_ms": f_ms,
                "caption_ms": -1,
                "offset": 0,
                "dist": -1,
                "matched": False
            })

    stats = compute_stats(rows)
    outliers = [r for r in rows if r["matched"] and abs(r["offset"]) >= 150]

    print(f"\n{'='*70}\nINDEPENDENT ALIGNMENT RESULTS\n{'='*70}")
    print(f"  Matched words:    {stats.get('count')}")
    print(f"  Median offset:    {stats.get('median_ms'):+g} ms")
    print(f"  Median |offset|:  {stats.get('median_abs_ms')} ms  (Target < 80 ms:  {'PASS' if stats.get('pass_median') else 'FAIL'})")
    print(f"  p95   |offset|:   {stats.get('p95_abs_ms')} ms  (Target < 150 ms: {'PASS' if stats.get('pass_p95') else 'FAIL'})")
    print(f"  Mean offset:      {stats.get('mean_ms'):+g} ms")
    print(f"  Min / Max:        {stats.get('min_ms'):+g} ms / {stats.get('max_ms'):+g} ms")
    print(f"  Per-word outliers (|Δ| >= 150ms): {len(outliers)}/{stats.get('count')} ({len(outliers)*100.0/max(stats.get('count',1),1):.1f}%)")
    for r in outliers[:8]:
        print(f"    Outlier [{r['word']}]: FW={r['audio_ms']}ms, ASS={r['caption_ms']}ms, Δ={r['offset']:+d}ms")
    print(f"{'='*70}")

    evidence_dir = PROJECT_ROOT / "docs" / "evidence"
    evidence_dir.mkdir(parents=True, exist_ok=True)
    report_file = evidence_dir / f"independent_sync_report_{clip_id}.json"
    with open(report_file, "w", encoding="utf-8") as f:
        json.dump({
            "check_type": "independent_aligner_faster_whisper",
            "clip_id": clip_id,
            "clip_path": clip_path,
            "ass_path": ass_path,
            "stats": stats,
            "outliers": outliers,
            "rows": rows
        }, f, ensure_ascii=False, indent=2)
    print(f"Saved independent report: {report_file}")
    return stats

def main():
    parser = argparse.ArgumentParser(description="Caption/Voice Sync Measurement Tool")
    parser.add_argument("clip", nargs="?", help="Path to rendered MP4 video")
    parser.add_argument("ass", nargs="?", help="Path to ASS subtitle file")
    parser.add_argument("--language", default="ar", help="Spoken language code (e.g. ar, en, fr, auto)")
    parser.add_argument("--fixture", nargs="?", const="ar-TN-HediNeural", help="Run with synthetic TTS fixture (voice name or 'all')")
    parser.add_argument("--independent", action="store_true", help="Run independent sync check with local faster-whisper aligner")
    args = parser.parse_args()

    if args.fixture:
        if args.fixture.lower() == "all":
            print("\n>>> Running Tunisian Arabic TTS Ground-Truth Fixture:")
            run_fixture_sync_test(voice="ar-TN-HediNeural")
            print("\n>>> Running French TTS Ground-Truth Fixture:")
            run_fixture_sync_test(voice="fr-FR-DeniseNeural")
        else:
            run_fixture_sync_test(voice=args.fixture)
        return

    clip = args.clip
    let_local = os.environ.get("LOCALAPPDATA", os.path.expanduser("~"))
    if not clip:
        clip = os.path.join(let_local, r"clipper\renders\00d85522-7dea-4f5d-a1ac-b4afbd73bd12\a14e7214-eb87-4fdc-bf07-e07ade862053.mp4")
    ass = args.ass
    if not ass:
        ass = os.path.join(let_local, r"clipper\ass\a14e7214-eb87-4fdc-bf07-e07ade862053.ass")

    if args.independent:
        run_independent_sync_test(clip, ass, language=args.language)
    else:
        run_clip_sync_test(clip, ass, language=args.language)

if __name__ == "__main__":
    main()

