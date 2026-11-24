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

cat > "$demo/Demo.sbjshow" <<JSON
{
  "format": "sportsball-jukebox-show",
  "version": 1,
  "name": "Home Opener",
  "slots": 24,
  "columns": 6,
  "banks": ["Pregame", "Game", "Postgame"],
  "preview_output": 1,
  "outputs": [
    {"name": "Arena", "backup": {"id": "unknown", "name": "Null Audio Device"}},
    {"name": "Concourse", "device": {"id": "unknown", "name": "Null Audio Device"}}
  ],
  "controls": {"stop_all": {"kind": "note", "number": 0, "channel": 16},
               "next_bank": {"kind": "cc", "number": 20}},
  "cues": [
    {"slot": 0, "title": "Charge!", "hotkey": "1", "color": "red",
     "midi": {"kind": "note", "number": 36, "channel": 10},
     "clips": [{"file": "$a/charge.wav"}]},
    {"slot": 1, "title": "Goal Horn", "hotkey": "2", "color": "orange",
     "midi": {"kind": "note", "number": 37, "channel": 10},
     "duck": -12,
     "clips": [{"file": "$a/goal_horn.wav", "fade_out": 0.5}]},
    {"slot": 2, "title": "Walk-up: #23", "hotkey": "3", "color": "blue", "group": "Music",
     "clips": [{"file": "$a/walkup_23.wav", "fade_in": 0.05}],
     "markers": [
       {"id": 1, "name": "Groove", "time": 4.0,
        "action": {"kind": "jump", "to": {"time": 4.0}}},
       {"id": 2, "name": "Vamp", "time": 12.0,
        "action": {"kind": "loop", "to": {"marker": 1}, "hold": true}},
       {"id": 3, "name": "Out", "time": 13.5,
        "action": {"kind": "fade_out", "seconds": 1.5}}]},
    {"slot": 3, "title": "Crowd Build", "hotkey": "4", "color": "teal", "loop": true,
     "clips": [{"file": "$a/crowd_build.wav"}]},
    {"slot": 4, "title": "DEFENSE!", "hotkey": "5", "color": "purple",
     "clips": [{"file": "$a/defense.wav"}]},
    {"slot": 5, "title": "Let's Go!", "hotkey": "6", "color": "pink",
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
