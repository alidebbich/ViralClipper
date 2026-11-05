#!/usr/bin/env python3
"""
make_fixtures.py — generate smoke test video fixtures using ffmpeg lavfi sources + edge-tts.

Produces 4 files in tests/fixtures/:
  ar_tunisian.mp4   ~30s, Arabic TTS (Tunisian) over solid-colour 9:16 bars
  fr_french.mp4     ~25s, French TTS over testsrc pattern
  en_english.mp4    ~28s, English TTS over solid-colour
  mixed.mp4         ~35s, Mixed Arabic+English over solid-colour

No copyrighted media. No fontconfig dependency.
TTS via edge-tts (Microsoft Neural, free).
"""

import asyncio
import subprocess
import sys
import os
import time

FIXTURES_DIR = os.path.join(os.path.dirname(__file__), "..", "fixtures")
FFMPEG = "ffmpeg"

FIXTURES = [
    {
        "name": "ar_tunisian",
        "text": "هذا مقطع تجريبي بالعامية التونسية، وهو يهدف إلى اختبار نظام الترجمة والتعليقات العربية بشكل صحيح.",
        "voice": "ar-TN-HediNeural",
        "fallback_voice": "ar-EG-SalmaNeural",
        "video_filter": "color=c=blue:s=1080x1920",
        "duration": 30,
    },
    {
        "name": "fr_french",
        "text": "Ceci est un clip de test en francais. Il sert a verifier que la transcription et les sous-titres fonctionnent correctement pour la langue francaise.",
        "voice": "fr-FR-DeniseNeural",
        "fallback_voice": "fr-FR-HenriNeural",
        "video_filter": "testsrc=s=1080x1920",
        "duration": 25,
    },
    {
        "name": "en_english",
        "text": "This is a test clip in English. It verifies that the transcription pipeline, word-level timestamps, and caption rendering all work correctly for English language content.",
        "voice": "en-US-JennyNeural",
        "fallback_voice": "en-US-GuyNeural",
        "video_filter": "color=c=0x006400:s=1080x1920",
        "duration": 28,
    },
    {
        "name": "mixed",
        "text": "Hello, welcome to this mixed language test. We are testing Arabic and English together. مرحبا بكم في هذا الاختبار.",
        "voice": "en-US-JennyNeural",
        "fallback_voice": "en-US-GuyNeural",
        "video_filter": "color=c=purple:s=1080x1920",
        "duration": 35,
    },
    {
        "name": "long_test",
        "text": "Welcome to the long test fixture. " * 30,
        "voice": "en-US-JennyNeural",
        "fallback_voice": "en-US-GuyNeural",
        "video_filter": "color=c=black:s=1080x1920",
        "duration": 150,
    },
]


async def make_tts(text: str, voice: str, fallback: str, out_path: str) -> bool:
    """Generate TTS audio using edge-tts."""
    import edge_tts
    for v in [voice, fallback]:
        try:
            communicate = edge_tts.Communicate(text, v)
            await communicate.save(out_path)
            if os.path.exists(out_path) and os.path.getsize(out_path) > 1000:
                print(f"  TTS OK: {v}")
                return True
        except Exception as e:
            print(f"  TTS voice {v} failed: {e}", file=sys.stderr)
    return False


def make_video(fixture: dict, audio_path: str, out_path: str) -> bool:
    """Combine TTS audio with ffmpeg lavfi video source. No fontconfig needed."""
    duration = fixture["duration"]
    vfilt = fixture["video_filter"]

    # No drawtext — avoids fontconfig dependency on Windows.
    # Colour source + audio only.
    cmd = [
        FFMPEG, "-y",
        "-f", "lavfi", "-i", f"{vfilt}:r=30",
        "-i", audio_path,
        "-t", str(duration),
        "-c:v", "libx264", "-preset", "ultrafast", "-crf", "28", "-pix_fmt", "yuv420p",
        "-c:a", "aac", "-b:a", "128k",
        "-shortest",
        out_path
    ]
    res = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    if res.returncode == 0 and os.path.exists(out_path) and os.path.getsize(out_path) > 5000:
        size_mb = os.path.getsize(out_path) / 1024 / 1024
        dur_s = fixture["duration"]
        print(f"  Video OK: {os.path.basename(out_path)} ({size_mb:.1f} MB, ~{dur_s}s)")
        return True
    print(f"  Video FAILED:", file=sys.stderr)
    print(res.stderr[-400:] if res.stderr else "(no stderr)", file=sys.stderr)
    return False


async def main():
    os.makedirs(FIXTURES_DIR, exist_ok=True)
    total_start = time.time()
    results = []

    for fixture in FIXTURES:
        name = fixture["name"]
        print(f"\n=== Fixture: {name} ===")
        t0 = time.time()

        audio_path = os.path.join(FIXTURES_DIR, f"{name}.mp3")
        video_path = os.path.join(FIXTURES_DIR, f"{name}.mp4")

        if os.path.exists(video_path) and os.path.getsize(video_path) > 10000:
            print(f"  Already exists, skipping")
            results.append((name, True, 0.0))
            continue

        # TTS audio (may already exist)
        if not (os.path.exists(audio_path) and os.path.getsize(audio_path) > 1000):
            tts_ok = await make_tts(
                fixture["text"], fixture["voice"], fixture["fallback_voice"], audio_path
            )
            if not tts_ok:
                print(f"  FAILED: could not generate TTS")
                results.append((name, False, time.time() - t0))
                continue
        else:
            print(f"  TTS already exists ({os.path.getsize(audio_path)//1024} KB)")

        vid_ok = make_video(fixture, audio_path, video_path)
        elapsed = time.time() - t0
        results.append((name, vid_ok, elapsed))

    print(f"\n{'='*60}")
    print(f"Fixtures summary (total: {time.time()-total_start:.1f}s)")
    print(f"{'='*60}")
    for name, ok, t in results:
        status = "OK  " if ok else "FAIL"
        path = os.path.join(FIXTURES_DIR, f"{name}.mp4")
        size = f"{os.path.getsize(path)/1024/1024:.1f} MB" if os.path.exists(path) else "missing"
        print(f"  {status}  {name:<22} {t:5.1f}s  {size}")

    failures = [n for n, ok, _ in results if not ok]
    if failures:
        print(f"\nFAILED fixtures: {failures}")
        sys.exit(1)
    print("\nAll fixtures generated successfully.")


if __name__ == "__main__":
    asyncio.run(main())
