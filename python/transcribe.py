"""
transcribe.py — Viral Clipper transcription worker
===================================================
Primary:  Groq Whisper API (whisper-large-v3-turbo) — fast cloud, 32kbps mono compressed
Fallback: faster-whisper "small" local, CPU int8, VAD-filtered, chunked

Protocol: JSON lines to stdout.
  Progress line: {"type":"progress","stage":"transcribing","pct":42.0,"msg":"..."}
  Done line:     {"type":"done","transcript":{...}}
  Error line:    {"type":"error","msg":"..."}

Never write anything else to stdout. Use stderr for debug only.
"""

import sys
import json
import os
import time
import argparse
import hashlib
import tempfile
import struct
import subprocess
import threading
import concurrent.futures

os.environ["HF_HUB_DISABLE_SYMLINKS_WARNING"] = "1"

# Force UTF-8 on Windows
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8")


# ── helpers ──────────────────────────────────────────────────────────────────

def emit(obj: dict):
    """Write a single JSON line to stdout and flush immediately."""
    print(json.dumps(obj, ensure_ascii=False), flush=True)

def progress(pct: float, msg: str):
    emit({"type": "progress", "stage": "transcribing", "pct": round(pct, 1), "msg": msg})

def done(transcript: dict):
    emit({"type": "done", "transcript": transcript})

def error(msg: str):
    emit({"type": "error", "msg": msg})


def rss_mb() -> float:
    """Current process RSS in MB."""
    try:
        import psutil
        return psutil.Process().memory_info().rss / 1024 / 1024
    except Exception:
        return 0.0


def find_ffmpeg() -> str:
    """Locate ffmpeg on Windows."""
    local_appdata = os.environ.get("LOCALAPPDATA", "")
    candidates = [
        os.path.join(local_appdata, "Microsoft", "WinGet", "Links", "ffmpeg.exe"),
        r"C:\ffmpeg\bin\ffmpeg.exe",
        "ffmpeg",
    ]
    for c in candidates:
        if os.path.exists(c):
            return c
    return "ffmpeg"


def wav_duration(path: str) -> float:
    """Read WAV header to get duration without loading audio data."""
    try:
        with open(path, "rb") as f:
            riff = f.read(12)
            if riff[:4] != b"RIFF":
                return 0.0
            f.read(8)  # fmt chunk header size + id
            fmt_data = f.read(16)
            num_ch, sample_rate, byte_rate, block_align, bits = struct.unpack_from("<HHIIHH", fmt_data)
            # skip to data chunk
            while True:
                chunk_id = f.read(4)
                if len(chunk_id) < 4:
                    break
                chunk_size = struct.unpack("<I", f.read(4))[0]
                if chunk_id == b"data":
                    return chunk_size / byte_rate
                f.seek(chunk_size, 1)
    except Exception:
        pass
    return 0.0


def get_audio_duration(path: str) -> float:
    """Get audio duration in seconds using WAV header or ffprobe fallback."""
    d = wav_duration(path)
    if d > 0:
        return d
    try:
        ffmpeg_bin = find_ffmpeg()
        ffprobe_bin = ffmpeg_bin.replace("ffmpeg.exe", "ffprobe.exe").replace("ffmpeg", "ffprobe")
        if not os.path.exists(ffprobe_bin):
            ffprobe_bin = "ffprobe"
        res = subprocess.run(
            [ffprobe_bin, "-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", path],
            capture_output=True, text=True, timeout=10
        )
        if res.returncode == 0:
            return float(res.stdout.strip())
    except Exception:
        pass
    return 0.0


class ProgressHeartbeat:
    """Emits periodic heartbeat progress messages every interval_sec if no update was sent."""
    def __init__(self, stage: str, get_msg_fn, interval_sec: float = 4.0):
        self.stage = stage
        self.get_msg_fn = get_msg_fn
        self.interval = interval_sec
        self.stop_event = threading.Event()
        self.thread = None

    def __enter__(self):
        self.stop_event.clear()
        def loop():
            while not self.stop_event.wait(self.interval):
                msg, pct = self.get_msg_fn()
                if msg:
                    progress(pct, msg)
        self.thread = threading.Thread(target=loop, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop_event.set()
        if self.thread:
            self.thread.join(timeout=2.0)


# ── Language Mapping ─────────────────────────────────────────────────────────

LANGUAGE_TO_ISO = {
    "english": "en",
    "arabic": "ar",
    "french": "fr",
    "spanish": "es",
    "german": "de",
    "italian": "it",
    "portuguese": "pt",
    "russian": "ru",
    "chinese": "zh",
    "japanese": "ja",
    "korean": "ko",
    "hindi": "hi",
    "turkish": "tr",
    "dutch": "nl",
    "polish": "pl",
    "swedish": "sv",
    "indonesian": "id",
    "vietnamese": "vi",
}

def to_iso_lang(lang_str: str) -> str:
    """Normalize full language name (e.g. 'English') to ISO 639-1 code ('en')."""
    if not lang_str:
        return "en"
    cleaned = lang_str.strip().lower()
    if cleaned in LANGUAGE_TO_ISO:
        return LANGUAGE_TO_ISO[cleaned]
    if len(cleaned) == 2:
        return cleaned
    return cleaned[:2]


# ── Groq provider ────────────────────────────────────────────────────────────

GROQ_CHUNK_SECS = 2400.0   # 40 minutes per chunk (at 32kbps mono, ~10 MB, well under 25 MB limit)
GROQ_OVERLAP_SECS = 2.0    # 2 seconds overlap between chunks for word continuity


def compress_audio_chunk(audio_path: str, start: float, duration: float, out_path: str, ffmpeg_bin: str) -> bool:
    """
    Compress audio slice to 32kbps mono Opus in Ogg container for fast upload.
    Falls back to 32kbps mono MP3 if Opus is unavailable.
    """
    cmd = [
        ffmpeg_bin, "-y",
        "-ss", f"{start:.3f}",
        "-t", f"{duration:.3f}",
        "-i", audio_path,
        "-ac", "1",
        "-ar", "16000",
        "-c:a", "libopus",
        "-b:a", "32k",
        out_path
    ]
    res = subprocess.run(cmd, capture_output=True, timeout=180)
    if res.returncode == 0 and os.path.exists(out_path) and os.path.getsize(out_path) > 300:
        return True

    # Fallback to MP3
    mp3_path = out_path.replace(".ogg", ".mp3")
    cmd_mp3 = [
        ffmpeg_bin, "-y",
        "-ss", f"{start:.3f}",
        "-t", f"{duration:.3f}",
        "-i", audio_path,
        "-ac", "1",
        "-ar", "16000",
        "-c:a", "libmp3lame",
        "-b:a", "32k",
        mp3_path
    ]
    res2 = subprocess.run(cmd_mp3, capture_output=True, timeout=180)
    if res2.returncode == 0 and os.path.exists(mp3_path) and os.path.getsize(mp3_path) > 300:
        if mp3_path != out_path:
            try:
                os.replace(mp3_path, out_path)
            except Exception:
                pass
        return True
    return False


def split_audio_for_groq(audio_path: str, total_duration: float) -> list[tuple[str, float]]:
    """
    Compress audio into one or more 32kbps chunks ≤ 40 min each using find_ffmpeg().
    Returns list of (temp_file_path, start_offset_secs).
    Caller is responsible for cleaning up temporary files.
    """
    ffmpeg_bin = find_ffmpeg()

    if total_duration <= GROQ_CHUNK_SECS:
        tmp = tempfile.NamedTemporaryFile(suffix=".ogg", delete=False)
        tmp.close()
        if compress_audio_chunk(audio_path, 0.0, total_duration, tmp.name, ffmpeg_bin):
            return [(tmp.name, 0.0)]
        else:
            return [(audio_path, 0.0)]

    chunks = []
    start = 0.0
    while start < total_duration:
        tmp = tempfile.NamedTemporaryFile(suffix=".ogg", delete=False)
        tmp.close()
        end = min(start + GROQ_CHUNK_SECS, total_duration)
        duration = end - start
        if compress_audio_chunk(audio_path, start, duration, tmp.name, ffmpeg_bin):
            chunks.append((tmp.name, start))
        
        # Avoid tiny duplicate chunks: break immediately once we reached the end
        if end >= total_duration:
            break
        start += GROQ_CHUNK_SECS - GROQ_OVERLAP_SECS

    return chunks if chunks else [(audio_path, 0.0)]


def map_words_to_segments(segments: list, words: list, offset: float = 0.0) -> list:
    """
    Map word timestamps into segment structures with offset support.
    If segments is empty or missing, synthesizes segments from words by splitting
    on sentence punctuation (. ! ?) or silence gaps > 0.8s.
    """
    out_segs = []
    
    if not segments and words:
        current_words = []
        seg_start = round(words[0].get("start", 0.0) + offset, 3)
        last_word_end = round(words[0].get("end", 0.0) + offset, 3)

        for w in words:
            w_start = round(w.get("start", 0.0) + offset, 3)
            w_end = round(w.get("end", 0.0) + offset, 3)
            w_text = w.get("word", "").strip()

            # Check gap > 0.8s or punctuation or max 12 words
            gap = w_start - last_word_end
            if current_words and (gap > 0.8 or len(current_words) >= 12 or current_words[-1]["text"].endswith((".", "!", "?"))):
                out_segs.append({
                    "start": seg_start,
                    "end": last_word_end,
                    "text": " ".join(cw["text"] for cw in current_words),
                    "words": current_words,
                })
                current_words = []
                seg_start = w_start

            current_words.append({
                "start": w_start,
                "end": w_end,
                "text": w_text,
                "confidence": 1.0,
            })
            last_word_end = w_end

        if current_words:
            out_segs.append({
                "start": seg_start,
                "end": last_word_end,
                "text": " ".join(cw["text"] for cw in current_words),
                "words": current_words,
            })
        return out_segs

    w_idx = 0
    num_words = len(words)

    for seg in (segments or []):
        s_start = round(seg.get("start", 0.0) + offset, 3)
        s_end = round(seg.get("end", 0.0) + offset, 3)
        text = seg.get("text", "").strip()
        seg_words = []

        while w_idx < num_words:
            w = words[w_idx]
            w_start = round(w.get("start", 0.0) + offset, 3)
            w_end = round(w.get("end", 0.0) + offset, 3)

            # If word is past segment end + 0.2s tolerance, break to next segment
            if w_start > s_end + 0.2 and len(seg_words) > 0:
                break

            seg_words.append({
                "start": w_start,
                "end": w_end,
                "text": w.get("word", "").strip(),
                "confidence": 1.0,
            })
            w_idx += 1

        out_segs.append({
            "start": s_start,
            "end": s_end,
            "text": text,
            "words": seg_words,
        })

    # Any remaining words append to last segment
    if w_idx < num_words and out_segs:
        for i in range(w_idx, num_words):
            w = words[i]
            out_segs[-1]["words"].append({
                "start": round(w.get("start", 0.0) + offset, 3),
                "end": round(w.get("end", 0.0) + offset, 3),
                "text": w.get("word", "").strip(),
                "confidence": 1.0,
            })

    return out_segs


def transcribe_one_groq_chunk(
    chunk_path: str,
    offset: float,
    chunk_idx: int,
    total_chunks: int,
    api_key: str,
    language: str
) -> tuple[list, list, str, str | None]:
    """
    Transcribe a single compressed audio chunk via Groq API.
    Returns (segments, words, detected_language, error_msg).
    """
    import urllib.request, urllib.error
    import uuid

    boundary = uuid.uuid4().hex
    with open(chunk_path, "rb") as cf:
        audio_bytes = cf.read()

    # Determine filename & content-type
    ext = os.path.splitext(chunk_path)[1].lower()
    content_type = "audio/ogg" if ext == ".ogg" else "audio/mpeg" if ext == ".mp3" else "audio/wav"
    filename = f"audio{ext}"

    lang_part = language if language != "auto" else ""
    body_parts = [
        f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{filename}"\r\nContent-Type: {content_type}\r\n\r\n'.encode(),
        audio_bytes,
        f'\r\n--{boundary}\r\nContent-Disposition: form-data; name="model"\r\n\r\nwhisper-large-v3-turbo\r\n'.encode(),
        f'--{boundary}\r\nContent-Disposition: form-data; name="response_format"\r\n\r\nverbose_json\r\n'.encode(),
        f'--{boundary}\r\nContent-Disposition: form-data; name="timestamp_granularities[]"\r\n\r\nword\r\n'.encode(),
        f'--{boundary}\r\nContent-Disposition: form-data; name="timestamp_granularities[]"\r\n\r\nsegment\r\n'.encode(),
    ]
    if lang_part:
        body_parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="language"\r\n\r\n{lang_part}\r\n'.encode())
    body_parts.append(f'--{boundary}--\r\n'.encode())
    body = b"".join(body_parts)

    url = "https://api.groq.com/openai/v1/audio/transcriptions"
    retries = 3

    for attempt in range(retries):
        try:
            req = urllib.request.Request(
                url,
                data=body,
                headers={
                    "User-Agent": "ViralClipper/1.0",
                    "Authorization": f"Bearer {api_key}",
                    "Content-Type": f"multipart/form-data; boundary={boundary}",
                },
                method="POST"
            )
            with urllib.request.urlopen(req, timeout=120) as resp:
                result = json.loads(resp.read().decode("utf-8"))

            # Log raw response keys to stderr once
            if not getattr(transcribe_one_groq_chunk, "_logged_raw_keys", False):
                transcribe_one_groq_chunk._logged_raw_keys = True
                print(f"[groq] raw response keys: {list(result.keys())}", file=sys.stderr)

            detected_lang = to_iso_lang(result.get("language", "en"))
            words = result.get("words") or []
            segs = result.get("segments") or []
            return segs, words, detected_lang, None

        except urllib.error.HTTPError as e:
            body_text = e.read().decode("utf-8", errors="ignore")
            lowered = body_text.lower()
            # Check for hard hourly/daily quota limits
            if "audio seconds per hour" in lowered or "audio seconds per day" in lowered or "quota" in lowered:
                err_msg = f"Groq hourly quota exceeded ({body_text[:120]})"
                print(f"[groq] {err_msg}", file=sys.stderr)
                return [], [], "en", err_msg
            elif e.code == 429 and attempt < retries - 1:
                wait = (2 ** attempt) * 2
                progress(22.0, f"Groq rate limit, retrying in {wait}s...")
                time.sleep(wait)
                continue
            err_msg = f"HTTP {e.code}: {body_text[:120]}"
            print(f"[groq] {err_msg}", file=sys.stderr)
            return [], [], "en", err_msg
        except Exception as ex:
            if attempt < retries - 1:
                time.sleep(2)
                continue
            err_msg = str(ex)
            print(f"[groq] error: {err_msg}", file=sys.stderr)
            return [], [], "en", err_msg

    return [], [], "en", "Groq request failed after retries"


def transcribe_groq(audio_path: str, api_key: str, language: str) -> tuple[dict | None, str]:
    """
    Transcribe using Groq Whisper API.
    Compresses audio, processes chunks in parallel with ThreadPoolExecutor(3 workers),
    and attaches word timestamps to segments.
    Returns (transcript_dict, "") on success, or (None, error_msg) on failure.
    """
    total_duration = get_audio_duration(audio_path)
    total_bytes = os.path.getsize(audio_path)

    progress(21.0, f"Groq: preparing audio ({total_bytes//1024//1024} MB WAV, {total_duration:.0f}s)")

    # Split & compress into 32kbps chunks
    chunks_with_offsets = split_audio_for_groq(audio_path, total_duration)
    chunk_count = len(chunks_with_offsets)

    all_chunk_results = [None] * chunk_count
    groq_error = ""

    try:
        def worker(idx, item):
            c_path, offset = item
            pct = 21.0 + (idx / chunk_count) * 55.0
            progress(pct, f"Groq: transcribing chunk {idx+1}/{chunk_count}...")
            return idx, offset, transcribe_one_groq_chunk(c_path, offset, idx, chunk_count, api_key, language)

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            future_to_idx = {
                executor.submit(worker, idx, item): idx
                for idx, item in enumerate(chunks_with_offsets)
            }
            for future in concurrent.futures.as_completed(future_to_idx):
                idx, offset, (segs_raw, words_raw, detected_lang, err) = future.result()
                if err:
                    groq_error = err
                else:
                    mapped_segs = map_words_to_segments(segs_raw, words_raw, offset=offset)
                    all_chunk_results[idx] = (mapped_segs, detected_lang)

    finally:
        # Guarantee all temporary compressed chunk files are deleted even on failure
        for c_path, _ in chunks_with_offsets:
            if c_path != audio_path and os.path.exists(c_path):
                try:
                    os.unlink(c_path)
                except Exception:
                    pass

    if groq_error:
        return None, groq_error

    all_segments = []
    final_lang = "en"
    for item in all_chunk_results:
        if item is None:
            return None, "Chunk transcription failed"
        segs, lang = item
        all_segments.extend(segs)
        if lang:
            final_lang = lang

    # Treat zero segments as failure
    if not all_segments:
        return None, "Groq returned 0 segments"

    # Deduplicate overlapping segments (merge by time)
    all_segments.sort(key=lambda s: s["start"])
    deduped = []
    last_end = -1.0
    for seg in all_segments:
        if seg["start"] < last_end - 0.1:
            continue
        deduped.append(seg)
        last_end = seg["end"]

    return {
        "language": final_lang,
        "duration": total_duration,
        "segments": deduped,
        "provider": "groq",
    }, ""


# ── Local faster-whisper provider ────────────────────────────────────────────

def ensure_local_model(models_dir: str) -> str:
    """
    Ensure faster-whisper small model is present as real files (no symlinks).
    Returns path to model directory.
    If missing or corrupt, downloads cleanly with visible heartbeat progress.
    """
    os.environ["HF_HUB_DISABLE_SYMLINKS_WARNING"] = "1"
    import shutil

    if not models_dir:
        appdata = os.environ.get("APPDATA", os.path.expanduser("~"))
        models_dir = os.path.join(appdata, "ViralClipper", "models")

    os.makedirs(models_dir, exist_ok=True)

    candidates = [
        os.path.join(models_dir, "small"),
        models_dir,
        os.path.join(os.environ.get("APPDATA", ""), "ViralClipper", "models", "small"),
    ]

    for c in candidates:
        if not c or not os.path.isdir(c):
            continue
        bin_path = os.path.join(c, "model.bin")
        if os.path.isfile(bin_path):
            try:
                is_link = os.path.islink(bin_path)
                size = os.path.getsize(bin_path)
                if not is_link and size > 400 * 1024 * 1024:
                    print(f"[local] Found valid standalone model at {c} ({size // 1024 // 1024} MB)", file=sys.stderr)
                    return c
                elif is_link:
                    # Windows symlink — resolve to real file if target exists
                    target = os.path.realpath(bin_path)
                    if os.path.isfile(target) and os.path.getsize(target) > 400 * 1024 * 1024:
                        print(f"[local] Converting symlink to real file at {bin_path}...", file=sys.stderr)
                        os.remove(bin_path)
                        shutil.copyfile(target, bin_path)
                        return c
                    else:
                        print(f"[local] Broken symlink at {bin_path}, cleaning up {c}...", file=sys.stderr)
                        shutil.rmtree(c, ignore_errors=True)
                else:
                    print(f"[local] Incomplete model file at {bin_path} ({size} bytes), cleaning up...", file=sys.stderr)
                    shutil.rmtree(c, ignore_errors=True)
            except Exception as e:
                print(f"[local] Model check error at {c}: {e}", file=sys.stderr)

    # Model missing: download cleanly without symlinks with active heartbeat
    target_dir = os.path.join(models_dir, "small")
    os.makedirs(target_dir, exist_ok=True)

    progress(21.0, "Downloading Whisper model (small ~480MB, one-time setup)...")
    print(f"[local] Downloading model to {target_dir}...", file=sys.stderr)

    expected_bytes = 485 * 1024 * 1024  # ~485MB total
    dl_err = []

    def do_download():
        try:
            from huggingface_hub import snapshot_download
            snapshot_download(
                "Systran/faster-whisper-small",
                local_dir=target_dir,
                allow_patterns=[
                    "config.json",
                    "preprocessor_config.json",
                    "model.bin",
                    "tokenizer.json",
                    "vocabulary.*",
                ],
            )
        except Exception as ex:
            dl_err.append(ex)

    dl_thread = threading.Thread(target=do_download, daemon=True)
    dl_thread.start()

    last_emit = 0.0
    while dl_thread.is_alive():
        dl_thread.join(timeout=1.5)
        now = time.time()
        if now - last_emit >= 4.0:
            last_emit = now
            current_bytes = sum(
                os.path.getsize(os.path.join(root, f))
                for root, _, files in os.walk(target_dir)
                for f in files
            )
            pct = min(99.0, (current_bytes / expected_bytes) * 100.0)
            mb = current_bytes // 1024 // 1024
            progress(21.0, f"Downloading Whisper model... {pct:.0f}% ({mb}/~485 MB)")

    if dl_err:
        raise RuntimeError(f"Failed to download Whisper model: {dl_err[0]}")

    bin_path = os.path.join(target_dir, "model.bin")
    if not os.path.isfile(bin_path) or os.path.getsize(bin_path) < 400 * 1024 * 1024:
        raise RuntimeError(f"Model download incomplete: {bin_path}")

    print(f"[local] Model ready at {target_dir}", file=sys.stderr)
    return target_dir


def transcribe_local(audio_path: str, language: str, models_dir: str) -> dict:
    """
    Transcribe locally with faster-whisper small, CPU int8.
    Streams segments to stdout as they are produced (true progress) with heartbeat.
    """
    try:
        from faster_whisper import WhisperModel
    except ImportError:
        raise RuntimeError("faster-whisper not installed. Run: pip install faster-whisper")

    total_duration = get_audio_duration(audio_path)
    model_dir = ensure_local_model(models_dir)

    progress(22.0, "Local whisper: loading model (small/int8)...")
    print(f"[local] RSS before model load: {rss_mb():.0f} MB", file=sys.stderr)

    t_load = time.time()
    def load_heartbeat():
        elapsed = time.time() - t_load
        return f"Local whisper: loading model into memory ({elapsed:.0f}s)...", 23.0

    with ProgressHeartbeat("transcribing", load_heartbeat, interval_sec=4.0):
        model = WhisperModel(
            model_dir,
            device="cpu",
            compute_type="int8",
            cpu_threads=min(8, os.cpu_count() or 4),
            num_workers=1,
        )

    print(f"[local] RSS after model load: {rss_mb():.0f} MB", file=sys.stderr)
    progress(28.0, f"Local whisper: transcribing {total_duration:.0f}s audio...")

    lang_arg = None if language == "auto" else language
    segments_gen, info = model.transcribe(
        audio_path,
        language=lang_arg,
        beam_size=1,
        best_of=1,
        condition_on_previous_text=False,
        word_timestamps=True,
        vad_filter=True,
        vad_parameters={"min_silence_duration_ms": 500},
    )

    all_segments = []
    detected_lang = to_iso_lang(getattr(info, "language", "en"))
    t_duration = getattr(info, "duration", total_duration) or total_duration

    t_seg_start = time.time()
    last_seg_time = [time.time()]

    def transcribe_heartbeat():
        now = time.time()
        elapsed = now - t_seg_start
        since_last = now - last_seg_time[0]
        if since_last >= 4.0:
            calc_pct = min(92.0, 28.0 + (elapsed / max(t_duration, 1.0)) * 60.0)
            return f"Local whisper: processing audio ({elapsed:.0f}s elapsed)...", calc_pct
        return "", 0.0

    with ProgressHeartbeat("transcribing", transcribe_heartbeat, interval_sec=3.0):
        for seg in segments_gen:
            last_seg_time[0] = time.time()
            pct = 28.0 + (seg.end / max(t_duration, 1.0)) * 65.0
            progress(min(pct, 92.0), f"Transcribing {seg.end:.0f}/{t_duration:.0f}s...")

            words = []
            if seg.words:
                for w in seg.words:
                    words.append({
                        "start": round(w.start, 3),
                        "end": round(w.end, 3),
                        "text": w.word.strip(),
                        "confidence": round(float(w.probability), 3),
                    })

            all_segments.append({
                "start": round(seg.start, 3),
                "end": round(seg.end, 3),
                "text": seg.text.strip(),
                "words": words,
            })

            rss = rss_mb()
            print(f"[local] RSS: {rss:.0f} MB | seg end={seg.end:.1f}s", file=sys.stderr)
            if rss > 5000:
                raise RuntimeError(f"RAM exceeded 5 GB ({rss:.0f} MB) — aborting")

    return {
        "language": detected_lang,
        "duration": round(t_duration, 3),
        "segments": all_segments,
        "provider": "local",
    }


# ── Cache ─────────────────────────────────────────────────────────────────────

def cache_path(audio_path: str, cache_dir: str) -> str:
    """Generate cache path using audio path, size, and modification timestamp."""
    try:
        stat = os.stat(audio_path)
        key_str = f"{os.path.abspath(audio_path)}_{stat.st_size}_{stat.st_mtime_ns}"
    except Exception:
        key_str = os.path.abspath(audio_path)
    h = hashlib.sha256(key_str.encode()).hexdigest()[:16]
    os.makedirs(cache_dir, exist_ok=True)
    return os.path.join(cache_dir, f"{h}.transcript.json")


# ── Main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("audio_path", help="Path to 16kHz mono WAV file")
    parser.add_argument("--lang", default="auto", help="Language code (auto, en, ar, fr, ...)")
    parser.add_argument("--models-dir", default="", help="Directory to store local models")
    parser.add_argument("--download-root", dest="download_root", default="", help="Alias for models-dir")
    parser.add_argument("--cache-dir", default="", help="Directory for transcript cache")
    args = parser.parse_args()

    audio_path = args.audio_path
    if "GROQ_API_KEY" in os.environ:
        groq_key = os.environ["GROQ_API_KEY"].strip()
    else:
        groq_key = ""
        workspace_env = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".env")
        for env_path in [".env", "../.env", workspace_env]:
            if os.path.exists(env_path):
                try:
                    with open(env_path, "r", encoding="utf-8") as f:
                        for line in f:
                            line = line.strip()
                            if line.startswith("GROQ_API_KEY="):
                                groq_key = line.split("=", 1)[1].strip().strip('"').strip("'")
                                if groq_key:
                                    break
                except Exception:
                    pass
            if groq_key:
                break
    if groq_key.lower() in ("none", "null", "false", "disabled", "0"):
        groq_key = ""

    language = args.lang or "auto"

    appdata = os.environ.get("APPDATA", os.path.expanduser("~"))
    models_dir = args.download_root or args.models_dir or os.path.join(appdata, "ViralClipper", "models")
    local_appdata = os.environ.get("LOCALAPPDATA", appdata)
    cache_dir = args.cache_dir or os.path.join(local_appdata, "clipper", "transcript_cache")

    print(f"[transcribe] start | audio={audio_path} | rss={rss_mb():.0f}MB | groq={'configured' if groq_key else 'none'}", file=sys.stderr)

    if not os.path.exists(audio_path):
        error(f"Audio file not found: {audio_path}")
        sys.exit(1)

    # Check cache
    cp = cache_path(audio_path, cache_dir)
    if os.path.exists(cp):
        try:
            with open(cp, "r", encoding="utf-8") as f:
                cached = json.load(f)
            if cached.get("segments"):
                progress(100.0, f"Loaded from cache ({len(cached['segments'])} segments)")
                done(cached)
                return
        except Exception:
            pass

    transcript = None
    t0 = time.time()
    groq_err = ""

    # Try Groq first
    if groq_key:
        progress(20.0, "Transcribing via Groq Whisper API (fast cloud)...")
        try:
            transcript, groq_err = transcribe_groq(audio_path, groq_key, language)
            if transcript and transcript.get("segments"):
                elapsed = time.time() - t0
                progress(97.0, f"Groq done in {elapsed:.1f}s, {len(transcript['segments'])} segments")
            elif not groq_err:
                groq_err = "Groq returned 0 segments"
                transcript = None
        except Exception as e:
            groq_err = str(e)
            transcript = None
            print(f"Groq exception: {e}", file=sys.stderr)

    # Fallback: local faster-whisper (if Groq absent or failed with 0 segments or error)
    if not transcript or not transcript.get("segments"):
        if not groq_key:
            progress(20.0, "Groq not configured, using slow local fallback (faster-whisper small)...")
        else:
            err_desc = f" ({groq_err})" if groq_err else ""
            progress(20.0, f"Groq failed{err_desc}, using slow local fallback (faster-whisper small)...")
        try:
            transcript = transcribe_local(audio_path, language, models_dir)
            elapsed = time.time() - t0
            progress(97.0, f"Local done in {elapsed:.1f}s, {len(transcript['segments'])} segments")
        except Exception as e:
            error(f"Transcription failed: {e}")
            sys.exit(1)

    if not transcript or not transcript.get("segments"):
        error("Transcription produced empty result")
        sys.exit(1)

    # Save to cache
    try:
        with open(cp, "w", encoding="utf-8") as f:
            json.dump(transcript, f, ensure_ascii=False)
    except Exception as e:
        print(f"Cache write failed: {e}", file=sys.stderr)

    print(f"[transcribe] done | segments={len(transcript['segments'])} | rss={rss_mb():.0f}MB | elapsed={time.time()-t0:.1f}s", file=sys.stderr)
    transcript["forced_language"] = language  # language arg the caller supplied (may be 'auto')
    done(transcript)


if __name__ == "__main__":
    main()