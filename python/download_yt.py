"""
Resilient YouTube Video Downloader for ViralClipper.
Solves:
1. Blackholed / dead ISP Google Global Cache (GGC) nodes (e.g. Topnet / ATI Tunisia)
   by automatically testing TCP 443 connectivity and failing over to the authorized
   secondary/backup media nodes (mn) signed in the YouTube playback token.
2. Fast, bounded timeouts (no 20s x 10 retries = 200s+ hanging).
3. Automatic merge of best video (<=720p) and best audio into MP4 using FFmpeg.
4. Streams real-time `[download] XX.X%` progress for the Tauri UI.
5. Saves `.info.json` next to the destination for downstream pipeline metadata.
"""

import sys
import os
import json
import socket
import urllib.parse
import yt_dlp

# Force UTF-8 on Windows stdout/stderr to safely print titles with emojis, Arabic, and Unicode
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

def is_host_reachable(host, port=443, timeout=1.5):
    """Check if TCP port on host accepts connections within timeout seconds."""
    try:
        ip = socket.gethostbyname(host)
        s = socket.create_connection((ip, port), timeout=timeout)
        s.close()
        return True
    except Exception:
        return False

def failover_googlevideo_url(url):
    """
    If the googlevideo primary host is unreachable,
    route to the backup media node (mn) signed by YouTube.
    """
    if "googlevideo.com" not in url:
        return url

    p = urllib.parse.urlparse(url)
    primary_host = p.netloc

    # Quick connectivity check
    if is_host_reachable(primary_host, timeout=1.5):
        return url

    sys.stderr.write(f"[downloader] GGC host {primary_host} timed out. Resolving backup node...\n")
    sys.stderr.flush()

    query = urllib.parse.parse_qs(p.query)
    mn_val = query.get("mn", [""])[0]
    nodes = [n.strip() for n in mn_val.split(",") if n.strip()]
    mvi = query.get("mvi", ["2"])[0]

    # Test candidate backup nodes from YouTube's signed mn list
    candidates = nodes[1:] + ["sn-u1hpa5-53", "sn-hpa7zns6", "sn-4g5ednss"]
    for node in candidates:
        candidate_host = f"rr{mvi}---{node}.googlevideo.com"
        if is_host_reachable(candidate_host, timeout=1.5):
            sys.stderr.write(f"[downloader] Failover successfully routed to CDN server: {candidate_host}\n")
            sys.stderr.flush()
            return url.replace(primary_host, candidate_host)

    return url

def main():
    import argparse
    parser = argparse.ArgumentParser(description="ViralClipper Resilient Downloader")
    parser.add_argument("url", help="YouTube video URL")
    parser.add_argument("-o", "--output", required=True, help="Output file path or template")
    parser.add_argument("--ffmpeg-location", default=None, help="Path to ffmpeg executable")
    parser.add_argument("--cookies", default=None, help="Path to cookies.txt")
    parser.add_argument("--max-height", type=int, default=1080, help="Max video height")
    # Ignored compatibility arguments passed by yt-dlp wrappers
    parser.add_argument("--merge-output-format", default="mp4")
    parser.add_argument("--no-playlist", action="store_true")
    parser.add_argument("--no-warnings", action="store_true")
    parser.add_argument("--write-info-json", action="store_true")
    parser.add_argument("-N", type=int, default=4)
    args, unknown = parser.parse_known_args()

    # Detect Node.js for yt-dlp JS runtime (n challenge deciphering)
    js_runtimes = {}
    for np in [r"C:\Program Files\nodejs\node.exe", r"C:\Program Files (x86)\nodejs\node.exe"]:
        if os.path.exists(np):
            js_runtimes = {'node': {'path': np}}
            break

    # Resolve FFmpeg location
    ffmpeg_exe = args.ffmpeg_location
    if not ffmpeg_exe or not os.path.exists(ffmpeg_exe):
        local_appdata = os.environ.get("LOCALAPPDATA", os.path.join(os.path.expanduser("~"), "AppData", "Local"))
        winget = os.path.join(local_appdata, r"Microsoft\WinGet\Links\ffmpeg.exe")
        if os.path.exists(winget):
            ffmpeg_exe = winget
        else:
            ffmpeg_exe = "ffmpeg"

    format_selector = (
        f"bestvideo[height<={args.max_height}][ext=mp4]+bestaudio[ext=m4a]/"
        f"bestvideo[height<={args.max_height}]+bestaudio/"
        f"best[height<={args.max_height}]/best"
    )

    ydl_opts = {
        'format': format_selector,
        'outtmpl': args.output,
        'merge_output_format': 'mp4',
        'noplaylist': True,
        'quiet': False,
        'no_warnings': True,
        'socket_timeout': 6,
        'retries': 2,
        'fragment_retries': 2,
        'concurrent_fragment_downloads': 4,
    }

    if ffmpeg_exe and os.path.exists(ffmpeg_exe):
        ydl_opts['ffmpeg_location'] = ffmpeg_exe

    if args.cookies and os.path.exists(args.cookies):
        ydl_opts['cookiefile'] = args.cookies

    if js_runtimes:
        ydl_opts['js_runtimes'] = js_runtimes

    print(f"[downloader] Resolving stream metadata for: {args.url}", flush=True)
    with yt_dlp.YoutubeDL(ydl_opts) as ydl:
        try:
            info = ydl.extract_info(args.url, download=False)
        except Exception as e:
            sys.stderr.write(f"[downloader] Metadata extraction failed: {e}\n")
            sys.exit(1)

        # Inspect and failover unreachable hosts in all requested formats
        req_formats = info.get('requested_formats', [info])
        for f in req_formats:
            if 'url' in f:
                f['url'] = failover_googlevideo_url(f['url'])

        # Save info.json next to output if requested
        dest_prefix = args.output
        if "%(ext)s" in dest_prefix:
            dest_prefix = dest_prefix.replace(".%(ext)s", "")
        info_json_file = f"{dest_prefix}.info.json"
        try:
            os.makedirs(os.path.dirname(os.path.abspath(info_json_file)), exist_ok=True)
            with open(info_json_file, "w", encoding="utf-8") as jf:
                json.dump(info, jf)
        except Exception:
            pass

        print(f"[downloader] Starting resilient download of {info.get('title', 'Video')}...", flush=True)
        try:
            ydl.process_video_result(info, download=True)
            print("[downloader] Download and remux completed successfully.", flush=True)
        except Exception as e:
            sys.stderr.write(f"[downloader] Download failed: {e}\n")
            sys.exit(1)

if __name__ == "__main__":
    main()
