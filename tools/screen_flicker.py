#!/usr/bin/env python3
"""Fullscreen flicker source for testing the strobe path.

Shows alternating white and black frames at a fixed rate, so the app's detected
period can be checked against a known value. The screen refreshes at 60 Hz, so
the rate must stay below 30 Hz: the strobe band (1-25 Hz) is covered, the PWM
band (100 Hz-5 kHz) is not - a screen cannot switch that fast.

Usage:
    python3 tools/screen_flicker.py 10 30      # 10 Hz, 30 s
    python3 tools/screen_flicker.py 2.5 60     # 2.5 Hz, 60 s

Keys: q quits early.
"""

import sys
import time
import tkinter as tk

def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    hz = float(sys.argv[1])
    seconds = float(sys.argv[2])
    if hz <= 0.0 or hz >= 30.0:
        sys.exit("rate must be in (0, 30) Hz: the screen refreshes at 60 Hz")

    half = 1.0 / (2.0 * hz)          # one flash + one dark = one period
    root = tk.Tk()
    root.set_fullscreen(True)
    label = tk.Label(root, font=("monospace", 40))
    label.pack(fill="both", expand=True)
    root.bind("q", lambda e: root.destroy())

    start = time.perf_counter()
    deadline = start + seconds
    n = 0
    while time.perf_counter() < deadline:
        white = n % 2 == 0
        bg = "white" if white else "black"
        fg = "black" if white else "white"
        root.configure(bg=bg)
        label.configure(bg=bg, fg=fg, text=f"{hz} Hz  {n // 2} flashes")
        root.update()
        n += 1
        # Schedule on an absolute clock so the rate does not drift over the run.
        next_t = start + n * half
        delay = next_t - time.perf_counter()
        if delay > 0:
            time.sleep(delay)

    root.destroy()

main()
