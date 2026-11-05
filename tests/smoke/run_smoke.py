#!/usr/bin/env python3
"""
run_smoke.py — Smoke test for the Viral Clipper pipeline.

For each fixture MP4:
  1. Extract 16kHz mono WAV using ffmpeg (as the Rust pipeline does).
  2. Run python/transcribe.py on it (Groq primary, faster-whisper fallback).
  3. Measure per-stage time and peak RSS.
  4. Print summary table.

Usage:
  python tests/smoke/run_smoke.py [--fixture ar_tunisian|fr_french|en_english|mixed|all]
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time

import psutil

FIXTURES_DIR = os.path.join(os.path.dirname(__file__), "..", "fixtures")
PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
TRANSCRIBE_PY = os.path.join(PROJECT_ROOT, "python", "transcribe.py")
PYTHON = sys.executable
FFMPEG = "ffmpeg"

ALL_FIXTURES = ["ar_tunisian", "fr_french", "en_english", "mixed"]
LANG_MAP = {
    "ar_tunisian": "ar",
    "fr_french": "fr",
    "en_english": "en",
    "mixed": "auto",
    "long_test": "en",
}


def extract_audio(video_path: str, wav_path: str) -> bool:
    """Extract 16kHz mono WAV from video, as the Rust pipeline does."""
    cmd = [
        FFMPEG, "-y",
        "-i", video_path,
        "-ac", "1",
        "-ar", "16000",
        "-vn",
        wav_path
    ]
    res = subprocess.run(cmd, capture_output=True, timeout=60)
    return res.returncode == 0 and os.path.exists(wav_path) and os.path.getsize(wav_path) > 100


def get_rss_mb(proc: subprocess.Popen) -> float:
    try:
        p = psutil.Process(proc.pid)
        total = p.memory_info().rss
        for c in p.children(recursive=True):
            try:
                total += c.memory_info().rss
            except Exception:
                pass
        return total / 1024 / 1024
    except Exception:
        return 0.0


def run_transcription(audio_path: str, language: str) -> dict:
    t0 = time.time()
    peak_rss = 0.0
    progress_lines = []
    done_result = None
    error_result = None

    cmd = [PYTHON, TRANSCRIBE_PY, audio_path, "--lang", language]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        cwd=PROJECT_ROOT,
    )

    def rss_monitor():
        nonlocal peak_rss
        while proc.poll() is None:
            rss = get_rss_mb(proc)
            if rss > peak_rss:
                peak_rss = rss
            time.sleep(0.3)

    t = threading.Thread(target=rss_monitor, daemon=True)
    t.start()

    for raw_line in proc.stdout:
        line = raw_line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        typ = obj.get("type")
        if typ == "progress":
            progress_lines.append({
                "pct": obj.get("pct"),
                "msg": obj.get("msg"),
                "elapsed_s": round(time.time() - t0, 2),
            })
        elif typ == "done":
            done_result = obj.get("transcript", {})
        elif typ == "error":
            error_result = obj.get("msg")

    proc.wait()
    stderr_out = proc.stderr.read() if proc.stderr else ""
    t.join(timeout=2.0)
    elapsed = time.time() - t0

    return {
        "elapsed_s": round(elapsed, 2),
        "peak_rss_mb": round(peak_rss, 1),
        "exit_code": proc.returncode,
        "progress_count": len(progress_lines),
        "last_progress": progress_lines[-1] if progress_lines else None,
        "segments": len(done_result.get("segments", [])) if done_result else 0,
        "language": done_result.get("language") if done_result else None,
        "duration_s": done_result.get("duration") if done_result else None,
        "provider": done_result.get("provider") if done_result else None,
        "forced_language": done_result.get("forced_language") if done_result else None,
        "raw_transcript": done_result,
        "error": error_result,
        "stderr_tail": stderr_out[-300:] if stderr_out else "",
        "success": done_result is not None and error_result is None and (done_result.get("segments") or []) != [],
    }


def main():
    parser = argparse.ArgumentParser(description="Smoke test for Viral Clipper pipeline")
    parser.add_argument("--fixture", choices=ALL_FIXTURES + ["long_test", "all"], default="all")
    parser.add_argument("--long", action="store_true", help="Run with long_test fixture")
    args = parser.parse_args()
    if args.long:
        fixtures = ["long_test"]
    else:
        fixtures = ALL_FIXTURES if args.fixture == "all" else [args.fixture]

    out_dir = os.path.join(PROJECT_ROOT, "tests", "smoke", "out")
    os.makedirs(out_dir, exist_ok=True)


    # Verify fixtures exist
    missing = [f for f in fixtures if not os.path.exists(os.path.join(FIXTURES_DIR, f"{f}.mp4"))]
    if missing:
        print(f"Missing fixture MP4s: {missing}")
        print("Run: python tests/smoke/make_fixtures.py")
        sys.exit(1)

    print(f"\nSmoke test — {len(fixtures)} fixture(s)")
    print("=" * 72)
    results = []

    with tempfile.TemporaryDirectory() as tmp:
        for fixture_name in fixtures:
            lang = LANG_MAP.get(fixture_name, "auto")
            video_path = os.path.join(FIXTURES_DIR, f"{fixture_name}.mp4")
            wav_path = os.path.join(tmp, f"{fixture_name}.wav")

            print(f"\n[{fixture_name}] ({lang})")

            # Stage 1: audio extraction
            t_audio = time.time()
            audio_ok = extract_audio(video_path, wav_path)
            audio_time = time.time() - t_audio
            if not audio_ok:
                print(f"  FAIL: audio extraction failed")
                results.append({"fixture": fixture_name, "success": False, "error": "audio extraction failed",
                                "elapsed_s": 0, "peak_rss_mb": 0, "segments": 0, "language": None, "provider": None})
                continue
            wav_mb = os.path.getsize(wav_path) / 1024 / 1024
            print(f"  Audio: {wav_mb:.1f} MB WAV in {audio_time:.1f}s")

            # Stage 2: transcription
            print(f"  Transcribing...")
            r = run_transcription(wav_path, lang)
            r["fixture"] = fixture_name

            status = "PASS" if r["success"] else "FAIL"
            print(f"  {status} | {r['elapsed_s']:.1f}s | {r['peak_rss_mb']:.0f} MB RSS | "
                  f"{r['segments']} segs | lang={r['language']} | provider={r['provider']}")
            if r["error"]:
                print(f"  ERROR: {r['error']}")
            if not r["success"] and r["stderr_tail"]:
                print(f"  STDERR: {r['stderr_tail']}")
            
            # Stage 3: rendering
            if r["success"]:
                print(f"  Rendering...")
                t_render = time.time()
                
                # Write transcript to temp file
                transcript_json_path = os.path.join(tmp, f"{fixture_name}_transcript.json")
                with open(transcript_json_path, "w", encoding="utf-8") as f:
                    json.dump(r["raw_transcript"], f)
                
                cmd = ["cargo", "run", "--manifest-path", "src-tauri/Cargo.toml", "--bin", "smoke", "--", video_path, transcript_json_path, tmp]
                proc = subprocess.run(cmd, capture_output=True, text=True, cwd=PROJECT_ROOT)
                r["render_time_s"] = round(time.time() - t_render, 2)
                r["render_success"] = proc.returncode == 0
                
                if proc.returncode == 0:
                    import shutil
                    thumb_path = None
                    for l in proc.stdout.splitlines():
                        if l.startswith("THUMB="):
                            thumb_path = l.split("=", 1)[1]
                    if thumb_path and os.path.exists(thumb_path):
                        dest = os.path.join(out_dir, f"{fixture_name}.jpg")
                        shutil.copy2(thumb_path, dest)
                        print(f"  Render PASS | {r['render_time_s']:.1f}s | thumb: {dest}")
                    else:
                        print(f"  Render FAIL | missing thumb")
                        r["render_success"] = False
                else:
                    print(f"  Render FAIL | {proc.stderr[-300:]}")
            else:
                r["render_success"] = False
                r["render_time_s"] = 0.0

            results.append(r)

    print(f"\n{'='*95}")
    print(f"{'Fixture':<22} {'Status':<6} {'Time(s)':<9} {'RSS(MB)':<9} {'Segs':<6} {'Forced':<8} {'Lang':<6} {'Provider':<8} {'Render(s)'}")
    print(f"{'-'*95}")
    for r in results:
        st = "PASS" if r.get("success") and r.get("render_success") else "FAIL"
        print(f"{r['fixture']:<22} {st:<6} {r.get('elapsed_s',0):<9.1f} {r.get('peak_rss_mb',0):<9.0f} "
              f"{r.get('segments',0):<6} {str(r.get('forced_language','')):<8} {str(r.get('language','')):<6} {str(r.get('provider') or 'n/a'):<8} {r.get('render_time_s',0.0):.1f}")

    failures = [r["fixture"] for r in results if not r.get("success")]
    if failures:
        print(f"\nFAILED: {failures}")
        sys.exit(1)
    print(f"\nAll {len(results)} fixtures PASSED.")


if __name__ == "__main__":
    main()
