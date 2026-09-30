# Viral Clipper — Project Instructions

This file is auto-loaded by Claude Code at the start of every session in this repo.
Keep it factual and current — update it whenever a convention or gotcha changes.

## What this project is

A Crayo.ai/OpusClip-style desktop app: paste a YouTube link → download → transcribe →
find the best moments → cut vertical clips with captions → apply a template/thumbnail →
export. Local-first; cloud APIs (Gemini, Pexels) are optional enhancements, not dependencies.

## Stack

- Frontend: React 18 + TypeScript, Vite 5, plain CSS (no CSS framework), React hooks only
- Shell: Tauri v2 — plugins: dialog, shell, fs, global-shortcut, tray-icon
- Backend: Rust, Tokio, `rusqlite` (SQLite), `reqwest` (rustls-tls), `serde`, `uuid`,
  `chrono`, `anyhow`, `Arc<Mutex<>>` / `tokio::sync::Mutex`
- Python sidecar (invoked as subprocess): `faster-whisper` (int8, CPU), `yt-dlp`, `torch`,
  Python 3.14.6 embedded
- Bundled binaries: `ffmpeg` 8.1 (NVENC), `yt-dlp`, `python.exe`
- Optional: Ollama (local LLM rerank), Gemini Pro API (viral scoring, thumbnail prompts),
  Pexels API (B-roll)

## Project structure

```
clipper/
├── src/                    # React frontend (App.tsx is the 7-tab shell)
├── src-tauri/src/
│   ├── lib.rs              # Tauri entry, commands, tray
│   ├── pipeline/           # 6-stage orchestrator — download→transcribe→score→cut→caption→export
│   ├── ffmpeg/             # filter graphs, ASS captions, NVENC
│   ├── whisper/            # Python sidecar wrapper
│   ├── ollama/, gemini/    # LLM clients
│   ├── scorer/             # heuristic viral scoring
│   ├── templates/          # template CRUD + Tauri commands
│   ├── database/           # SQLite migrations + CRUD
│   ├── assets/             # Pexels sync
│   └── thermal/            # GPU monitoring/throttle
├── binaries/               # ffmpeg.exe, yt-dlp.bat, python.exe
└── python/                 # transcribe.py, heuristics.py
```

## Conventions

- No new frontend state library. Hooks + context only.
- No new CSS framework. Extend the existing dark theme via CSS custom properties.
- Every Tauri command must return a typed error the frontend can render — never a raw panic
  or a generic "something went wrong."
- Every pipeline stage must have a timeout and emit a clear failure event on timeout —
  hanging silently is treated as a bug, not an edge case.
- Every ffmpeg call using NVENC (`h264_nvenc`) must have a `libx264` (CPU) fallback path.
- Every optional integration (Ollama, Gemini, Pexels) must degrade gracefully when
  unavailable — never fail the whole pipeline over an optional feature.

## Known gotchas (update this as you find more)

- Python 3.14 is very new — `faster-whisper`/`torch` compatibility with it is not guaranteed.
  If the Python sidecar fails silently, check this first.
- yt-dlp's progress-line output format changes between versions — any Rust code that parses
  stdout for "download complete" is fragile. Prefer checking the actual exit code + expected
  output file on disk over string-matching progress lines.
- `Arc<Mutex<>>` / `tokio::sync::Mutex` guarding pipeline state: never hold a lock across an
  `.await` point.

## Commands

- Dev: `cargo tauri dev`
- Build: `cargo tauri build`
- Frontend only: `npm run dev` (in `src/`)

## What "done" looks like for a task

Don't report a fix as complete until you've actually run the pipeline end-to-end (or the
specific tab/feature you touched) and confirmed the behavior, not just that it compiles.
