import json
import os
import subprocess
import sys
import tempfile
import time
import shutil

VIDEOS = {
    "Gaming": "https://www.youtube.com/watch?v=4TUXGa4HW6I",
    "Podcast": "https://www.youtube.com/watch?v=oslEIsL8Mbs",
    "Arabic": "https://www.youtube.com/watch?v=noAb_kD7xxw",
    "French": "https://www.youtube.com/watch?v=uzm_i4S4fTE",
    "Tutorial": "https://www.youtube.com/watch?v=ihHwLfIg_Dk",
    "Comedy": "https://www.youtube.com/watch?v=CoFjr3pgjCg"
}

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
PYTHON = sys.executable
TRANSCRIBE_PY = os.path.join(PROJECT_ROOT, "python", "transcribe.py")
MATRIX_OUT = os.path.join(PROJECT_ROOT, "docs", "MATRIX-RESULTS.md")

def run_matrix():
    with open(MATRIX_OUT, "w", encoding="utf-8") as f:
        f.write("# Real-Video Matrix Results\n\n")

    for cat, url in VIDEOS.items():
        print(f"=== Processing {cat} ===")
        t0 = time.time()
        
        with tempfile.TemporaryDirectory() as tmp:
            video_path = os.path.join(tmp, "video.mp4")
            audio_path = os.path.join(tmp, "audio.wav")
            
            # 1. Download
            print("  Downloading...")
            # Download 1 minute of video to keep matrix test fast
            dl_cmd = ["yt-dlp", "-f", "worstvideo[ext=mp4]+bestaudio[ext=m4a]/mp4", "--download-sections", "*00:01:00-00:02:00", "--merge-output-format", "mp4", "-o", video_path, url]
            dl_proc = subprocess.run(dl_cmd, capture_output=True, text=True)

            if not os.path.exists(video_path):
                print(f"  Download failed! {dl_proc.stderr}")
                continue

            # 2. Extract Audio
            print("  Extracting audio...")
            subprocess.run(["ffmpeg", "-y", "-i", video_path, "-ac", "1", "-ar", "16000", "-vn", audio_path], capture_output=True)

            # 3. Transcribe
            print("  Transcribing...")
            t_transcribe = time.time()
            tr_proc = subprocess.run([PYTHON, TRANSCRIBE_PY, audio_path, "--lang", "auto"], capture_output=True, text=True, cwd=PROJECT_ROOT)
            
            transcript_path = audio_path + ".transcript.json"
            
            if not os.path.exists(transcript_path):
                print(f"  Transcription failed! {tr_proc.stderr}")
                continue

            # 4. Clip Analysis & Render
            print("  AI Analysis...")
            cmd = ["cargo", "run", "--bin", "matrix", "--", video_path, transcript_path]
            proc = subprocess.run(cmd, capture_output=True, text=True, cwd=os.path.join(PROJECT_ROOT, "src-tauri"))
            
            t_total = time.time() - t0
            print(f"  Finished {cat} in {t_total:.1f}s")
            
            with open(MATRIX_OUT, "a", encoding="utf-8") as f:
                f.write(f"## {cat}\n")
                f.write(f"- **URL**: {url}\n")
                f.write(f"- **Time**: {t_total:.1f}s\n")
                if proc.returncode == 0:
                    f.write(f"- **Status**: PASS\n")
                    f.write("```text\n")
                    f.write(proc.stdout)
                    f.write("\n```\n")
                else:
                    f.write(f"- **Status**: FAIL\n")
                    f.write("```text\n")
                    f.write(proc.stderr[-500:])
                    f.write("\n```\n")
                f.write("\n")

if __name__ == "__main__":
    run_matrix()
