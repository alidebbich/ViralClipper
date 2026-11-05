#!/usr/bin/env python3
"""
verify_visual_hashes.py — Characterization test for visual regression.
Computes a tolerant perceptual hash (block mean luminance hash) of smoke test output frames.
"""
import os
import sys
import hashlib
from PIL import Image

OUT_DIR = os.path.join(os.path.dirname(__file__), "out")
HASHES_FILE = os.path.join(os.path.dirname(__file__), "visual_hashes.json")

def compute_block_hash(img_path: str, grid_size: int = 8) -> str:
    """Compute an 8x8 block average hash (tolerant to minor compression artifacts)."""
    with Image.open(img_path) as img:
        img = img.convert("L").resize((grid_size, grid_size), Image.Resampling.BILINEAR)
        pixels = list(img.getdata())
        avg = sum(pixels) / len(pixels)
        bits = "".join("1" if p > avg else "0" for p in pixels)
        return hex(int(bits, 2))[2:].zfill(grid_size * grid_size // 4)

def hamming_distance(h1: str, h2: str) -> int:
    b1 = bin(int(h1, 16))[2:].zfill(64)
    b2 = bin(int(h2, 16))[2:].zfill(64)
    return sum(c1 != c2 for c1, c2 in zip(b1, b2))

def main():
    if not os.path.isdir(OUT_DIR):
        print(f"Directory {OUT_DIR} does not exist.", file=sys.stderr)
        sys.exit(1)

    import json
    files = sorted([f for f in os.listdir(OUT_DIR) if f.endswith(".jpg")])
    if not files:
        print(f"No JPG files found in {OUT_DIR}", file=sys.stderr)
        sys.exit(1)

    current_hashes = {}
    for f in files:
        path = os.path.join(OUT_DIR, f)
        current_hashes[f] = compute_block_hash(path)

    if "--record" in sys.argv or not os.path.exists(HASHES_FILE):
        with open(HASHES_FILE, "w", encoding="utf-8") as f:
            json.dump(current_hashes, f, indent=2)
        print(f"Recorded baseline visual hashes for {len(current_hashes)} fixtures in {HASHES_FILE}")
        return

    with open(HASHES_FILE, "r", encoding="utf-8") as f:
        baseline_hashes = json.load(f)

    max_dist = 6  # Tolerant threshold for JPEG artifact differences
    all_ok = True
    print("Visual hash verification:")
    for f, h in current_hashes.items():
        if f not in baseline_hashes:
            print(f"  [WARN] {f}: New fixture output, not in baseline")
            continue
        dist = hamming_distance(h, baseline_hashes[f])
        status = "PASS" if dist <= max_dist else "REGRESSION"
        if dist > max_dist:
            all_ok = False
        print(f"  [{status}] {f}: distance={dist}/{max_dist}")

    if not all_ok:
        sys.exit(1)
    print("All visual hashes within tolerance!")

if __name__ == "__main__":
    main()
