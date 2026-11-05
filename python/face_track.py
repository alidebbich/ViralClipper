#!/usr/bin/env python3
"""
face_track.py - where is the speaker? (feeds the 9:16 smart crop)

Usage:  python face_track.py <video> --start 12.5 --dur 45 [--fps 2.5] [--model path/to/yunet.onnx]
Output: ONE json line on stdout:
  {"keys": [[t, cx], ...], "found_ratio": 0.93, "spread_ratio": 0.02, "samples": 112, "detector": "yunet"}
  t  = seconds from the START of the clip window, cx = horizontal face center 0..1.

Detectors (best first):
  1. YuNet (OpenCV FaceDetectorYN): accurate, fast, handles angles. Model (~230 KB) is looked up in
     --model, $CLIPPER_YUNET_MODEL, ./assets/models, ../assets/models, %APPDATA%/ViralClipper/models and is
     downloaded automatically on first use.
  2. Haar cascades bundled with OpenCV (no download, weaker).
Needs:  pip install opencv-python-headless
If OpenCV is missing or no face is found, found_ratio is 0 and the app falls back to blur-fit framing.
"""
import sys, os, json, argparse

# Force UTF-8 on Windows stdout/stderr
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

YUNET_URL = "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
YUNET_NAME = "face_detection_yunet_2023mar.onnx"

def find_model(explicit):
    cands = [explicit, os.environ.get("CLIPPER_YUNET_MODEL"),
             os.path.join("assets", "models", YUNET_NAME), os.path.join("..", "assets", "models", YUNET_NAME),
             os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "models", YUNET_NAME),
             os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")), "ViralClipper", "models", YUNET_NAME)]
    for c in cands:
        if c and os.path.isfile(c) and os.path.getsize(c) > 100_000:
            return c
    # try to download once (silently ignore failures)
    try:
        import urllib.request
        dest_dir = os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")), "ViralClipper", "models")
        os.makedirs(dest_dir, exist_ok=True)
        dest = os.path.join(dest_dir, YUNET_NAME)
        urllib.request.urlretrieve(YUNET_URL, dest)
        if os.path.getsize(dest) > 100_000:
            return dest
        os.remove(dest)
    except Exception:
        pass
    return None

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("video")
    ap.add_argument("--start", type=float, default=0.0)
    ap.add_argument("--dur", type=float, default=30.0)
    ap.add_argument("--fps", type=float, default=2.5)
    ap.add_argument("--model", default=None)
    ap.add_argument("--width", type=int, default=0, help="detection width (default 480 YuNet / 320 Haar)")
    a = ap.parse_args()
    empty = {"keys": [], "found_ratio": 0.0, "spread_ratio": 0.0, "samples": 0, "detector": "none"}
    try:
        import cv2
    except Exception as e:
        print(json.dumps({**empty, "error": f"opencv missing: {e}"})); return

    cap = cv2.VideoCapture(a.video)
    if not cap.isOpened():
        print(json.dumps({**empty, "error": "cannot open video"})); return

    yunet, detector_name = None, "none"
    mp = find_model(a.model)
    if mp and hasattr(cv2, "FaceDetectorYN"):
        try:
            yunet = cv2.FaceDetectorYN.create(mp, "", (320, 320), 0.6, 0.3, 5000)
            detector_name = "yunet"
        except Exception:
            yunet = None
    det_w = a.width or (480 if yunet else 320)
    front = prof = None
    if yunet is None and hasattr(cv2, "CascadeClassifier") and hasattr(cv2, "data") and hasattr(cv2.data, "haarcascades"):
        try:
            front = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_frontalface_default.xml")
            prof = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_profileface.xml")
            if not front.empty():
                detector_name = "haar"
        except Exception:
            front = prof = None

    src_fps = cap.get(cv2.CAP_PROP_FPS) or 30.0
    cap.set(cv2.CAP_PROP_POS_MSEC, a.start * 1000.0)
    step = max(1, int(round(src_fps / a.fps)))
    total = int(a.dur * src_fps)
    keys, spread, samples, i = [], 0, 0, 0
    while i < total:
        if not cap.grab():
            break
        if i % step == 0:
            ok, frame = cap.retrieve()
            if not ok:
                break
            samples += 1
            h, w = frame.shape[:2]
            small = cv2.resize(frame, (det_w, int(h * det_w / w)))
            sw = small.shape[1]
            faces = []                                  # list of (x, y, w, h)
            if yunet is not None:
                yunet.setInputSize((small.shape[1], small.shape[0]))
                _, res = yunet.detect(small)
                if res is not None:
                    faces = [tuple(r[:4]) for r in res]
            elif front is not None:
                gray = cv2.equalizeHist(cv2.cvtColor(small, cv2.COLOR_BGR2GRAY))
                ms = int(sw * 0.06)
                faces = list(front.detectMultiScale(gray, 1.1, 4, minSize=(ms, ms)))
                if not len(faces) and prof is not None:
                    faces = list(prof.detectMultiScale(gray, 1.1, 4, minSize=(ms, ms)))
            if len(faces):
                faces = sorted(faces, key=lambda f: f[2] * f[3], reverse=True)
                x, y, fw, fh = faces[0]
                cx = (x + fw / 2.0) / sw
                if len(faces) > 1:
                    x2, y2, fw2, fh2 = faces[1]
                    if fw2 * fh2 >= 0.45 * fw * fh:
                        cx2 = (x2 + fw2 / 2.0) / sw
                        if abs(cx - cx2) < 0.40:
                            cx = (cx + cx2) / 2.0      # two people close together: frame both
                        else:
                            spread += 1                # far apart: a 9:16 crop cannot hold both
                keys.append([round(float(i / src_fps), 3), round(float(min(max(float(cx), 0.0), 1.0)), 4)])
        i += 1
    cap.release()
    n = len(keys)
    print(json.dumps({"keys": keys,
                      "found_ratio": round(float(n / samples), 3) if samples else 0.0,
                      "spread_ratio": round(float(spread / n), 3) if n else 0.0,
                      "samples": int(samples), "detector": str(detector_name)}))

if __name__ == "__main__":
    main()
