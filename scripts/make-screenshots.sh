#!/bin/bash
# Builds a demo show with synthesised audio, runs the app in screenshot
# mode, and leaves PNGs in <output-dir>. macOS only.
#
# Usage: scripts/make-screenshots.sh "<path to Sportsball Jukebox.app>" <output-dir>
set -euo pipefail

app="$1"
out="$(mkdir -p "$2" && cd "$2" && pwd)"
here="$(cd "$(dirname "$0")" && pwd)"
demo="$(mktemp -d)/Demo"
mkdir -p "$demo/Audio"

python3 "$here/demo_audio.py" "$demo/Audio"
a="$demo/Audio"

# A pad picture (a simple gradient with a ball), standard library only.
mkdir -p "$demo/Images"
python3 - "$demo/Images/defense.png" <<'PY'
import math, struct, sys, zlib
w = h = 256
rows = []
for y in range(h):
    row = bytearray([0])
    for x in range(w):
        d = math.hypot(x - 170, y - 90)
        if d < 48:
            r = g = b = 235 if (x + y) % 40 > 6 else 40
        else:
            t = y / h
            r, g, b = int(40 + 120 * t), int(20 + 30 * t), int(90 + 120 * (1 - t))
        row += bytes((r, g, b))
    rows.append(bytes(row))
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY

cat > "$demo/Demo.sbjshow" <<JSON
{
  "format": "sportsball-jukebox-show",
  "version": 1,
  "name": "Home Opener",
  "slots": 24,
  "columns": 6,
  "banks": ["Pregame", "Game", "Postgame"],
  "preview_output": 1,
  "level_match": -16,
  "pad_defaults": {"color": "teal", "fade_out": 1.0},
  "colors": [{"name": "Home red", "value": "#C8102E"}, {"name": "Sponsor gold", "value": "#E0A21B"}],
  "outputs": [
    {"name": "Arena", "backup": {"id": "unknown", "name": "Null Audio Device"}},
    {"name": "Concourse", "device": {"id": "unknown", "name": "Null Audio Device"}}
  ],
  "controls": {"stop_all": {"kind": "note", "number": 0, "channel": 16},
               "next_bank": {"kind": "cc", "number": 20},
               "rundown_go": {"kind": "cc", "number": 64}},
  "rundown": [
    {"id": 1, "text": "Doors open"},
    {"id": 2, "pad": 6, "text": "Warm-up music"},
    {"id": 3, "text": "Pregame"},
    {"id": 4, "pad": 8},
    {"id": 5, "pad": 7},
    {"id": 6, "pad": 3, "text": "Crowd build into puck drop"},
    {"id": 7, "text": "Period 1"},
    {"id": 8, "pad": 24},
    {"id": 9, "pad": 0, "text": "First goal: Charge!"},
    {"id": 10, "text": "Intermission"},
    {"id": 11, "pad": 9}
  ],
  "cues": [
    {"slot": 0, "title": "Charge!", "hotkey": "1", "color": "red",
     "midi": {"kind": "note", "number": 36, "channel": 10},
     "clips": [{"file": "$a/charge.wav"}]},
    {"slot": 1, "title": "Goal Horn", "hotkey": "2", "color": "orange",
     "midi": {"kind": "note", "number": 37, "channel": 10},
     "duck": -12, "sponsor": "Acme Bank", "play_mode": "over",
     "on_fire": [{"kind": "fire_pad", "pad": 3, "seconds": 1.5}],
     "clips": [{"file": "$a/goal_horn.wav", "fade_out": 0.5}]},
    {"slot": 2, "title": "Walk-up: #23", "hotkey": "3", "color": "blue", "group": "Music",
     "clips": [{"file": "$a/walkup_23.wav", "fade_in": 0.05}],
     "markers": [
       {"id": 1, "name": "Groove", "time": 4.0,
        "action": {"kind": "jump", "to": {"time": 4.0}}},
       {"id": 2, "name": "Vamp", "time": 12.0,
        "action": {"kind": "loop", "to": {"marker": 1}, "hold": true}},
       {"id": 4, "name": "Horn", "time": 8.0,
        "action": {"kind": "fire", "pad": 1}},
       {"id": 3, "name": "Out", "time": 13.5,
        "action": {"kind": "fade_out", "seconds": 1.5}}]},
    {"slot": 3, "title": "Crowd Build", "hotkey": "4", "color": "teal", "loop": true,
     "clips": [{"file": "$a/crowd_build.wav"}]},
    {"slot": 4, "title": "DEFENSE!", "hotkey": "5", "color": "purple",
     "image": "$demo/Images/defense.png",
     "clips": [{"file": "$a/defense.wav"}]},
    {"slot": 5, "title": "Let's Go!", "hotkey": "6", "color": "#E0A21B",
     "clips": [{"file": "$a/lets_go.wav"}]},
    {"slot": 6, "title": "Timeout Music", "hotkey": "q", "color": "green", "group": "Music",
     "fade_in": 0.5,
     "playlist": {"items": ["$a/timeout.wav", "$a/lineups.wav", "$a/dj_drop.wav"],
                  "shuffle": true, "crossfade": 1.5}},
    {"slot": 7, "title": "Starting Lineups", "hotkey": "w", "color": "blue",
     "clips": [{"file": "$a/lineups.wav"}]},
    {"slot": 8, "title": "Anthem Intro", "hotkey": "e", "color": "gray",
     "clips": [{"file": "$a/anthem_intro.wav"}]},
    {"slot": 9, "title": "DJ Drop", "hotkey": "r", "color": "purple", "output": 1,
     "clips": [{"file": "$a/dj_drop.wav"}]},
    {"slot": 12, "title": "Penalty Kill", "hotkey": "a", "color": "red",
     "clips": [{"file": "$a/defense.wav"}]},
    {"slot": 13, "title": "Power Play", "hotkey": "s", "color": "orange",
     "clips": [{"file": "$a/lets_go.wav"}]},
    {"slot": 24, "title": "Period Start", "hotkey": "1", "color": "green",
     "clips": [{"file": "$a/charge.wav"}]}
  ]
}
JSON

"$app/Contents/MacOS/Sportsball Jukebox" --open-show "$demo/Demo.sbjshow" --screenshots "$out" &
pid=$!
for _ in $(seq 1 120); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
done
kill "$pid" 2>/dev/null || true
ls -la "$out"
test -n "$(ls "$out"/*.png 2>/dev/null)"
