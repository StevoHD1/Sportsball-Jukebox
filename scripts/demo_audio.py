#!/usr/bin/env python3
"""Generates demo audio (synthesised, no samples) for screenshots and demos.

Usage: demo_audio.py <output-dir>
Writes 48 kHz stereo 16-bit WAV files.
"""

import math
import random
import struct
import sys
import wave
from pathlib import Path

RATE = 48_000
random.seed(7)


def write(path: Path, left, right=None):
    right = right if right is not None else left
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        frames = bytearray()
        for l, r in zip(left, right):
            frames += struct.pack("<hh", int(max(-1, min(1, l)) * 30000), int(max(-1, min(1, r)) * 30000))
        w.writeframes(bytes(frames))


def tone(freq, seconds, shape="sine", attack=0.01, release=0.08, level=0.5):
    n = int(seconds * RATE)
    out = []
    for i in range(n):
        t = i / RATE
        phase = (t * freq) % 1.0
        if shape == "saw":
            v = 2 * phase - 1
        elif shape == "square":
            v = 1.0 if phase < 0.5 else -1.0
        else:
            v = math.sin(2 * math.pi * phase)
        env = min(1.0, t / attack, (seconds - t) / release) if release > 0 else min(1.0, t / attack)
        out.append(v * env * level)
    return out


def mix(*tracks):
    n = max(len(t) for t in tracks)
    return [sum(t[i] if i < len(t) else 0.0 for t in tracks) for i in range(n)]


def concat(*parts):
    out = []
    for p in parts:
        out.extend(p)
    return out


def silence(seconds):
    return [0.0] * int(seconds * RATE)


def note(midi):
    return 440.0 * 2 ** ((midi - 69) / 12)


def kick(seconds=0.25):
    n = int(seconds * RATE)
    return [math.sin(2 * math.pi * (60 + 120 * math.exp(-i / 800)) * i / RATE) * math.exp(-i / 4000) * 0.9 for i in range(n)]


def snare(seconds=0.2):
    n = int(seconds * RATE)
    return [(random.random() * 2 - 1) * math.exp(-i / 2500) * 0.5 for i in range(n)]


def beat(bars, bpm=120):
    beat_len = 60 / bpm
    out = []
    for _ in range(bars * 4):
        out.extend(mix(kick(), silence(beat_len))[: int(beat_len / 2 * RATE)])
        out.extend(mix(snare(), silence(beat_len))[: int(beat_len / 2 * RATE)])
    return out


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)

    # "Charge!" organ fanfare.
    charge = concat(*[tone(note(n), d, "square", level=0.3) for n, d in
                      [(67, .18), (72, .18), (76, .18), (79, .36), (76, .18), (79, .9)]])
    write(out / "charge.wav", charge)

    # Goal horn: low sawtooth chord.
    horn = mix(tone(note(43), 3.0, "saw", 0.05, 0.6, 0.35), tone(note(50), 3.0, "saw", 0.05, 0.6, 0.25))
    write(out / "goal_horn.wav", horn)

    # Walk-up: intro, a 4-bar groove (looped by a marker), and an outro.
    intro = mix(beat(2), concat(*[tone(note(n), .5, "saw", level=.15) for n in [52, 55, 57, 59] * 2]))
    groove = mix(beat(4), concat(*[tone(note(n), .25, "square", level=.12) for n in [40, 40, 43, 45] * 8]))
    outro = tone(note(40), 2.0, "saw", 0.01, 1.8, 0.3)
    write(out / "walkup_23.wav", concat(intro, groove, outro))

    # Crowd build: rising filtered noise.
    n = RATE * 8
    noise, last = [], 0.0
    for i in range(n):
        k = 0.02 + 0.3 * i / n
        last += k * ((random.random() * 2 - 1) - last)
        noise.append(last * (0.2 + 0.8 * i / n))
    write(out / "crowd_build.wav", noise, list(reversed(noise)))

    for name, notes in {
        "defense": [60, 60, 62, 60],
        "lets_go": [64, 67, 64, 67, 72],
        "timeout": [57, 60, 64, 67, 64, 60] * 3,
        "lineups": [48, 55, 60, 64] * 4,
        "anthem_intro": [55, 60, 64, 67, 72],
        "dj_drop": [72, 71, 69, 67, 65, 64, 62, 60],
    }.items():
        write(out / f"{name}.wav", concat(*[tone(note(m), .3, "sine", level=.4) for m in notes]))


if __name__ == "__main__":
    main()
