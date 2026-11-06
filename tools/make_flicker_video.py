#!/usr/bin/env python3
"""Flicker source as a video file, for testing the strobe path.

Writes an MP4 of alternating white and black frames at a fixed rate, so the app's
detected period can be checked against a known value. Play it fullscreen:

    python3 tools/make_flicker_video.py 10 60 flicker.mp4
    ffplay -f full-screen -loop 0 flicker.mp4

The video frame rate is fixed, so the rate is quantized: the script prints the
rate it actually produces. Rates that do not divide the frame rate exactly are
rounded to the nearest achievable one.

Usage:
    python3 tools/make_flicker_video.py <hz> <seconds> <out.mp4> [fps]
"""

import subprocess
import sys

W, H = 320, 240

def main():
    if len(sys.argv) < 4:
        print(__doc__)
        sys.exit(1)
    hz = float(sys.argv[1])
    seconds = float(sys.argv[2])
    out = sys.argv[3]
    fps = int(sys.argv[4]) if len(sys.argv) > 4 else 60

    half_frames = max(1, round(fps / (2.0 * hz)))
    real_hz = fps / (2.0 * half_frames)
    total = int(seconds * fps)
    err = (real_hz - hz) / hz * 100.0
    print(f"target {hz} Hz, producing {real_hz} Hz ({err:+.2f} %), {total} frames at {fps} fps")

    frame = W * H * 3
    white = b"\xff" * frame
    black = b"\x00" * frame
    data = b"".join(white if (i // half_frames) % 2 == 0 else black for i in range(total))
    p = subprocess.run(
        ["ffmpeg", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(fps),
         "-i", "pipe:", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
         "-y", out],
        input=data, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
    )
    if p.returncode:
        sys.exit(p.stderr.decode()[-400:])
    print(f"wrote {out}")

main()
