# Sportsball Jukebox: Architecture

Status: Phase 15 (pad actions). Last updated 2026-11-24.

## Goals, in priority order

1. **Reliability.** Audio must never drop out, glitch or stop unexpectedly
   during a live game. A UI hang, a slow disk or a bad file must not affect
   what is already playing.
2. **Performance.** Cues fire with no perceptible delay. Marker actions
   (loops, jumps) happen on the exact sample.
3. **Native feel.** A real macOS app (SwiftUI/AppKit) that behaves like one.
4. **Portability of the core.** The engine is plain Rust with no platform
   dependencies, so a Windows app could reuse it later.

## Big picture

```
┌──────────────────────────────────────────────────────────┐
│ macOS app — Swift (SwiftUI + AppKit)        apple/       │
│  cue grid, timeline editor, marker editor, device setup  │
└───────────────▲──────────────────────────────────────────┘
                │ C ABI (crates/sbj-ffi, include/sportsball_jukebox.h)
┌───────────────┴──────────────────────────────────────────┐
│ sbj-session: cues ↔ players, output device, firing       │
│ sbj-media:   decode files, resample to device rate       │
└───────────────▲──────────────────────────────────────────┘
                │
┌───────────────┴──────────────────────────────────────────┐
│ sbj-engine (Rust, platform-independent)                  │
│                                                          │
│  Controller ──commands──▶ Renderer                       │
│  (control thread) ◀─events── (audio thread)              │
│                   ◀─garbage─                             │
└───────────────▲──────────────────────────────────────────┘
                │ RenderCallback(&mut [f32])
┌───────────────┴──────────────────────────────────────────┐
│ sbj-backend                                              │
│  trait AudioBackend · OfflineBackend (tests)             │
│  SystemBackend: Core Audio via cpal (WASAPI if Windows)  │
└──────────────────────────────────────────────────────────┘
```

| Crate | Role |
|---|---|
| `sbj-engine` | Timeline model, marker actions, players, mixing, fades, transport. No I/O, no platform code. |
| `sbj-backend` | Device abstraction. `SystemBackend` uses cpal (Core Audio on macOS); `OfflineBackend` for tests. Reports device loss and glitches (xruns). |
| `sbj-media` | Decodes WAV/AIFF/CAF/FLAC/MP3/AAC/ALAC/Vorbis (Symphonia) and converts sample rates (rubato FFT resampler). Control side only. |
| `sbj-session` | The app model: a bank of cues, one output device, fire/stop/loop/gain. Generic over the backend, so it is tested end-to-end offline. |
| `sbj-ffi` | C ABI for the Swift app. Every call is panic-safe, tolerates null pointers, and reports errors via `sbj_last_error_message`. |

## One engine per audio interface

Each audio interface gets its own independent engine (one `Controller` plus
one `Renderer`), running at that interface's sample rate and buffer size, on
that interface's Core Audio I/O thread.

We decided **not** to play one cue on several interfaces in sync. Separate
interfaces have separate clocks that drift apart over time, and keeping them
aligned needs continuous resampling. Without that requirement:

- every engine is a simple, self-contained clock domain;
- a fault on one interface (unplugged, driver hang) cannot affect the
  others;
- every cue and timeline is routed to exactly one interface (and a channel
  range on it).

In the show, interfaces are referred to through **logical outputs**
("Main", "Locker room"...). Each output names a preferred device and an
optional backup; pads route to an output, never directly to a device.
That keeps shows portable between venues and makes failover a property of
the output.

If sync is needed later, macOS aggregate devices (which apply drift
correction) are the first thing to try, since they need no engine changes.

### Players

Each engine has a fixed number of **players** (64 by default; the session
uses one per cue). A player holds one timeline with its own transport,
gain and fade, so any number of cues can overlap. Players render into a
scratch buffer and are summed, then the master gain is applied. The
player count is fixed when the engine is created, so the audio thread never
allocates.

## Threading model and real-time rules

The audio callback must finish within the buffer period (for example 2.7 ms
at 128 frames / 48 kHz), every time. Anything that can block for an unknown
time is banned from that thread:

| Banned on the audio thread | Instead |
|---|---|
| Allocating or freeing memory | Everything is built on the control thread and moved in as a `Box`. Replaced objects go back through the **garbage queue** to be freed on the control thread. |
| Locks / mutexes | Lock-free SPSC ring buffers (`rtrb`) and atomics. |
| File or network I/O, logging | Audio is fully decoded into memory before it can be played. Events go out through a queue. |
| Unbounded loops | Each loop iteration either renders at least one frame or fires markers at a new position (see "Termination" below). |

These rules apply to all code reachable from `Renderer::process`.
`crates/sbj-engine/tests/realtime_safety.rs` installs an allocator that
counts any heap activity during `process` and fails the test if it sees
any.

### Communication

- **Commands** (control → audio): per player `Load`, `Unload`, `Play`,
  `Pause`, `Stop`, `Seek`, `Restart`, `SetPlayerGain`, `FadeOutAndStop`;
  global `StopAll { fade }` and `SetMasterGain`. Drained at the start of
  each audio block.
- **Events** (audio → control): `MarkerReached`, `Started`, `Paused`,
  `Stopped`, `Seeked`, `ReachedEnd`, `TimelineLoaded`, each tagged with
  its player. `Copy` types, so
  sending one never allocates. If the queue is full the event is dropped and
  counted (`dropped_events`) rather than blocking.
- **Shared atomics**: the playhead position and playing state, for the UI to
  read at display refresh rate without messaging.
- **Garbage** (audio → control): replaced timelines. The controller drains
  this before every command. The queue is sized so it cannot overflow. If it
  somehow did, the object is leaked and counted (`leaked_garbage`) rather
  than freed on the audio thread.

## Timeline model

All positions and durations are in **sample frames** (`u64`) at the
engine's sample rate. That is what makes "loop at exactly 0:32.500" a
precise, testable statement. The UI converts to and from seconds or
musical time for display.

- **AudioBuffer**: decoded, interleaved `f32` audio, shared by `Arc`. It is
  resampled to the device's rate when loaded (`sbj-media`, on a background
  thread), never during playback. The device keeps its own sample rate;
  we never change it, since other apps share the device.
- **Clip**: a region of a buffer placed on the timeline, with gain and
  fade-in and fade-out (linear or equal-power).
- **Marker**: a position, a stable `MarkerId` for the UI, and an action.
- **Timeline**: validated and immutable once built. To edit it, build a new
  timeline and swap it in. That's cheap, since audio buffers are shared, and
  it means the audio thread never sees a half-edited timeline.

Validation happens on the control thread in `TimelineBuilder::build`:
sample rates match, clips stay within their buffers, gains are finite and
non-negative, marker ids are unique, and jump targets are within the
timeline.

### Marker actions

| Action | Behaviour |
|---|---|
| `Jump { to }` | Move the playhead to `to` and keep playing. |
| `Loop { to, count }` | Jump back to `to`. `Times(n)` jumps n times, so the section plays n+1 times, then continues past. `Infinite` loops until the operator acts. The counter re-arms when the playhead passes through, so nested loops behave like musical repeats. Seek and Stop also reset counters. |
| `Pause` | Halt at the marker. Play resumes from there, past the marker. |
| `Stop` | Halt and return to the start. |
| `FadeOut { length }` | Fade everything to silence over `length` frames, then stop. |
| `Start { player, fade_in }` | Start another player of the same engine from the top, at this exact sample (fading in over `fade_in`). If it is playing it restarts; if it is holding a vamp it is released instead. Reported as `Event::Triggered`. |
| `StopPlayer { player, fade }` | Stop another player of the same engine at this exact sample, over `fade` frames (0 = the 3 ms declick stop). |
| `Level { gain, length }` | Ramp this player's marker level to `gain` over `length` frames. It multiplies the player gain (cue gain, ducking, fade-in), so ducking and fades still work, and resets to 1.0 on stop or restart. |
| `Notify` | Nothing in the engine; the control side acts on the `MarkerReached` event. |

**Cross-player actions at the exact sample (Phase 9).** Players render
one after another, so a marker on player 5 at frame 37 of a block cannot
start player 2 at frame 37 if player 2 has already rendered the block.
Instead, timelines with `Start`/`StopPlayer` markers mark their player as
one that "acts on others". `Renderer::process` splits each block into
slices that end at the next marker of any such player. At each slice
boundary those players fire their markers first (`Player::prepare`),
which only collects the cross-player actions into a preallocated list;
the actions are applied (in firing order); then every player renders the
slice. So every player is at the same sample when an action applies,
chains (A starts B, B's marker starts C) stay exact, and blocks with no
such timelines render as one slice, exactly as before. Actions beyond the
list's capacity (256 at one instant) are dropped and counted.

### Marker semantics

1. A marker fires when the playhead **reaches** its position during
   playback, before the frame at that position is rendered.
2. Several markers at one position fire in the order they were added. A
   marker that moves the playhead or halts playback ends the sequence.
3. Markers exactly where the playhead **lands** (after a jump, a seek, or
   when pressing play) do **not** fire.
4. Rendering is split at every marker position, so results are identical
   whatever block size the device uses. Tests run every scenario at block
   sizes 1, 7, 64, 333 and 512.

**Termination:** every pass of the render loop either renders at least one
frame or fires the markers at the current position. Rule 3 means it cannot
fire the same position twice without advancing. A `process` call therefore
always finishes, even for a jump that targets itself.

### Click-free edits (declick)

Cutting from one point in a sound to another produces a click. Whenever
the playhead jumps **while audible** (a marker jump or loop, a retrigger,
a seek), the player renders a short linear crossfade from where it was to
where it lands. The outgoing audio continues for `declick_frames` (3 ms
by default) while the new audio fades in. An operator **Stop** fades out
over the same length instead of cutting. Marker Pause and Stop actions
still cut exactly at the marker; use a Fade Out marker before them for a
soft ending. The crossfade uses a second preallocated scratch buffer, so
it keeps the no-allocation guarantee, and tests check it sample by
sample.

### Pad markers in the show (Phase 9)

The show's marker kinds `fire`, `stop_pad` and `volume` map onto the
actions above (`sbj-session/src/show.rs`, `cue_timeline`):

- `fire` / `stop_pad` with `pad` (a slot). If the target is a plain pad on
  the **same output**, the marker becomes `Start` / `StopPlayer` and the
  engine acts at the exact sample; the target's fade-in (fire) or
  fade-out (stop, unless `seconds` is given) is baked in. The session
  then gives a fired pad what pressing it also does, from the
  `Triggered` event on its next poll: a log entry (source `marker`),
  the exclusive-group hand-over and ducking.
- If the target is on **another output** (another device, so another
  engine and clock) or is a **playlist**, the marker becomes `Notify` and
  the session fires or stops the pad when it sees `MarkerReached`, within
  one poll (about 30 ms). Without sync between devices, sample accuracy
  across them would be meaningless anyway.
- `volume` with `db` and `seconds` becomes `Level`.
- How a marker reaches its target is baked into the source pad's
  timeline, so the session reloads source pads when that changes (the
  target moves output, becomes a playlist, changes its fades, is emptied
  or filled). Moving pads and renumbering banks rewrite `pad` so markers
  keep acting on the same sound. The preview player never acts on other
  pads.

## Show files

A show (`*.sbjshow`) is versioned JSON; the schema is defined by
`crates/sbj-session/src/show.rs` and mirrored in Swift by
`apple/SportsballJukebox/Show.swift`.

```json
{
  "format": "sportsball-jukebox-show", "version": 1,
  "name": "Home opener", "slots": 24, "columns": 6,
  "outputs": [
    { "name": "Main",
      "device": { "id": "<Core Audio UID>", "name": "Scarlett 2i2" },
      "backup": { "id": "...", "name": "MacBook Pro Speakers" } }
  ],
  "cues": [
    { "slot": 0, "title": "Charge", "hotkey": "1", "output": 0,
      "gain": 0.8, "loop": false, "color": "red",
      "clips": [ { "file": "/Music/charge.wav", "start": 0, "offset": 1.5,
                   "length": 12, "fade_in": 0.05, "fade_out": 0.5 } ],
      "markers": [
        { "id": 1, "name": "Vamp", "time": 4.0,
          "action": { "kind": "jump", "to": { "time": 4.0 } } },
        { "id": 2, "name": "Back", "time": 8.0,
          "action": { "kind": "loop", "to": { "marker": 1 }, "count": 2 } },
        { "id": 3, "name": "Horn", "time": 9.5,
          "action": { "kind": "fire", "pad": 1 } },
        { "id": 4, "name": "Under PA", "time": 10.0,
          "action": { "kind": "volume", "db": -12, "seconds": 0.5 } } ] }
  ]
}
```

- Times are seconds; they become sample frames when a cue is loaded onto
  an output at that device's rate.
- Fields at their default value are omitted; readers must default them.
  New fields must be optional. Breaking changes bump `version` and add a
  migration in `Show::from_json`. Newer versions are refused with a
  clear message.
- Saves are atomic (write temp file, rename).
- Audio is referenced by path. If a show moves (another Mac, a USB stick),
  files are looked for next to the show and in an `Audio` folder beside
  it.
- A problem with one cue (missing file, bad marker target) is reported for
  that pad; it never stops the rest of the show loading.

The C API passes shows, cues and outputs as JSON in this same schema,
which keeps the API small and the Swift side free of hand-written
marshalling.

## Reliability features

- **Device failover (done)**: each output tries its preferred device,
  then its backup, then the system default. When a device disappears
  mid-show, `Session::poll` reopens the output on the next one and
  resumes every cue that was playing from where it was, then tells the
  operator. Outputs with no device keep retrying every 2 s.
- **Loading never blocks firing (done)**: decoding happens outside the
  session lock, in parallel.
- **Autosave (done)**: shows save themselves a couple of seconds after
  each change; untitled shows are kept in Application Support.
- **Crash isolation**: the engine never depends on the UI thread, and a
  frozen UI keeps playing.
- **Pre-flight checks (partial)**: on load, every file's decode result and
  each cue's marker targets are checked and reported per pad. Still to do:
  memory budget, and a pre-game "everything OK?" summary.
- **No disk I/O at fire time**: all audio for a show is decoded into memory
  at load time.

## Banks and external control

**Banks** are pages of pads. A cue's `slot` is `bank * slots + pad`, every
output's engine has a player for every slot (up to 512), and only the
visible bank's pads are drawn. That's why keyboard shortcuts apply to
the bank shown. Resizing banks renumbers cues so each keeps its bank and
pad.

**MIDI and OSC** (`crates/sbj-control`) run on their own threads (CoreMIDI's,
and a UDP thread). They call `Session::handle_control` directly under the
session lock, which avoids the UI's ~30 ms poll interval. Lock order is
always listeners, then session: tearing down a MIDI connection waits for
in-flight callbacks, which hold the session lock. MIDI learn assigns the
next MIDI message to a pad or a show-wide control (Stop All, next/previous
bank). See `docs/control.md` for the OSC address space and Stream Deck
setup.

**Phone/tablet remote** (`sbj-control/src/remote.rs`, Phase 8): a small
HTTP/1.1 server written on `std` (no web framework): an accept thread
and a short-lived thread per connection, `Connection: close`, 16 KiB
request limit, 5 s timeouts, at most 32 connections. `/` serves an
embedded page (`remote.html`, plain JS that polls `GET /api/state`);
`/api/*` needs the PIN header, compared in constant time, with wrong
guesses delayed and serialised. Requests go to `Session::handle_remote`
(`sbj-session/src/remote.rs`) under the session lock, which maps button
presses to `ControlMessage::Remote` with an OSC address, so the remote
shares OSC's code path, logging and control footer. The FFI handler
holds only a weak reference to the session, like the MIDI/OSC sink.

**Pre-game check** (`Session::preflight`): output devices, backups, glitch
counts, per-pad problems, pads that can't play, duplicate keys (per bank)
and MIDI triggers, and memory used by decoded audio. Each item is an
error, warning or info.

## Firing a pad (Phase 4)

`Session::fire_cue` is what pressing a pad does, whether from a click,
its key, MIDI or OSC:

1. **Vamp release.** If the pad is playing and holding at a vamp (a loop
   marker with `hold: true`, i.e. `LoopCount::UntilReleased`), it
   releases the vamp: the current pass finishes and playback continues
   past the loop marker. Releases are one-shot, so the next vamp holds
   again, and a restart re-arms everything. The engine publishes a
   `holding` flag per player (playing, unreleased, and a vamp at or after
   the playhead), so the UI can show "VAMP · press to go".
2. **Retrigger.** Otherwise, a playing pad's `retrigger` setting
   decides: `restart` (default), `stop` or `fade`.
3. **Exclusive group.** Starting a pad fades out other playing pads with
   the same `group` (case-insensitive) over the show's `group_fade`
   (1 s by default).
4. **Ducking.** While a pad with `duck` (dB, negative) is playing, every
   other pad on the same output is lowered by the deepest active duck.
   The dip uses a 150 ms ramp and recovery 800 ms
   (`SetPlayerGainOver`). A just-fired pad counts as playing until the
   audio thread reports it (at most 250 ms), so ducking starts at once.
   Recovery is noticed on the UI's next poll, within about 33 ms.

**Fades.** A pad's `fade_in` ramps it up from silence each time it
fires. The pad's gain is set to zero instantly before the restart, then
ramped. Its `fade_out` (default 0.5 s) is the length used by Fade Out,
"pressed again: fade" and OSC `/sbj/fade`. The show's `stop_all_fade`
(default 0.5 s) is used by Stop All from every source.

**Playlist pads.** A cue with `playlist` plays one of its `items` per
press, in order or shuffled. With `auto_advance` it carries on through
them, wrapping around. Every pad slot has two engine players: its own and
a twin at `slot + MAX_SLOTS` (outputs therefore run 1024 players). A
playlist alternates between them, so the next song fades in on the idle
player while the current one fades out. Auto-advance starts the next song
50 ms more than the crossfade before the end, so the UI's poll interval
can never leave a gap, and fades over exactly the time remaining, so the
outgoing song still plays to its end. Pressing a playing playlist pad
crossfades to the next song, or stops or fades it if its retrigger
setting says so. Shuffle never repeats a song across a reshuffle. A
playlist pad counts as playing while either of its players is, and the
cue state reports the current item for the UI.

**Preview.** The show's `preview_output` names an output (e.g. the
operator's headphones) for auditioning. `Session::preview_cue` plays a pad
there on a reserved player, `2 × MAX_SLOTS`, so each output runs
`2 × MAX_SLOTS + 1` players. It never touches the pad's own players, its
exclusive group or ducking, so the crowd hears nothing different.
Previewing the same pad again stops it. Playlist pads preview the song
they would play next, and the cue state reports that song (`next_item`).
To make "up next" always known, a shuffled playlist reshuffles as soon as
it reaches its end, not on the next press.

**Routing.** A cue's `route` (`first`, `count`, 1-based) sends it to
specific output channels (e.g. 3–4 of an 8-channel interface). Mono
sources feed every routed channel; multichannel sources map channel by
channel. A route beyond the device's channels is reported as a problem
and the cue plays on all channels.

## Level matching and silence (Phase 10)

`sbj-media/src/analysis.rs` measures every file once, when it is
decoded (off the audio thread, about 90 ms for a 4-minute stereo song):

- **Integrated loudness** per ITU-R BS.1770-4 / EBU R128: K-weighting
  (shelf + high pass, coefficients derived for any sample rate), 400 ms
  blocks with 75 % overlap, absolute gate -70 LUFS, relative gate -10 LU.
  Sounds shorter than one block are measured as one block. A mono file
  counts as both speakers of a pair (+3 dB), because the engine plays
  mono on every channel. A 0 dBFS 1 kHz tone in one channel reads
  -3.01 LUFS at 44.1, 48 and 96 kHz (tested).
- **Sample peak**, and **silence** at the start and end (below -60 dBFS),
  keeping 5 ms before the first sound so attacks are not cut.

With the show's `level_match` target set (e.g. -16 LUFS), every clip and
playlist song gets a clip gain of `target - loudness`, capped at +12 dB
of boost and at whatever keeps its peak at or below -1 dBFS. It is a
static gain baked into the timeline, so it costs nothing at play time,
and it multiplies with the pad volume, ducking and fades as before. A pad
can opt out (`"level_match": false`). Changing the target reloads every
pad.

Files dropped on a pad start where the sound starts: the first clip's
offset is set to the measured leading silence, so a pad with a second of
silence at the top still fires instantly. The editor shows each file's
loudness, peak and the gain applied, and a Trim Silence button for both
ends.

## Game rundown (Phase 11)

The show's `rundown` is an ordered list of steps (`sbj-session/src/rundown.rs`):
a pad to fire (`pad`, with an optional `text` label) or a note (`text`
only, e.g. "Timeout 1"). The session keeps one pad step on **standby**
(not saved; the first pad step when a show opens). **GO** fires the
standby pad exactly as pressing it does (groups, ducking, retrigger, vamp
release) and moves standby to the next pad step; notes are skipped. If
the pad cannot fire (its device is gone), standby stays so GO can be
retried. Back/next move standby without firing.

GO comes from the app (button or Space), MIDI (learnable "Rundown GO"),
OSC (`/sbj/go`, `/sbj/rundown/next|prev`) and the phone remote
(`POST /api/go`, `/api/rundown/next|prev`, plus a `rundown` object in the
state). Plays are logged with the source `rundown` (or the control
surface's own source). Steps follow pads that move or are renumbered;
the pre-game check warns about steps on empty pads.

## Show building (Phase 12)

- **Library** (`sbj-session/src/library.rs`): the show's `library` lists
  audio files kept for building pads; the library shown is those plus
  every file a pad uses, with which pads use each. Library-only files are
  **not decoded** when the show opens, so a large library costs no
  memory; they are decoded when previewed (`preview_file`, on the preview
  output, level-matched like a new pad) or put on a pad. Files a pad uses
  can't be removed from it. The app adds files or whole folders, searches,
  and drags files onto pads or into a pad's timeline (the pad editor has
  its own copy of the library, as the main window can't be reached while
  the editor sheet is open).
- **Watch folders** (`sbj-session/src/watch.rs`): the show's
  `watch_folders` (`[{path, subfolders}]`) are scanned from `poll` every
  2 s, on the control thread. A new audio file joins the library once
  it has finished copying: the same size and modification time on two
  scans, or last modified 3+ s ago. Files are skipped if the library
  already has them, by resolved path or by name and size (so files
  copied into `Audio/` by Save With Audio don't come back as
  duplicates). Hidden files, links and empty files are ignored, and
  subfolders go 8 deep at most. Each folder picks up only its `types`
  (extensions). Empty means every audio type except MP4, which is
  usually video. Non-audio files are never picked up. Files are never moved or copied: the
  folder is the source of truth, so a file removed from the library
  comes back on the next scan while it's still in the folder. Imports bump the status JSON's `watch_imports`, which makes
  the app reload the library and record one "Import from Watch Folder"
  undo step. Each import also posts a notice and an Event Log entry
  (kind `library`). A folder that can't be read is logged once, listed
  in `missing_watch_folders`, and flagged by the pre-game check.
- **Cloud files** (Box Drive, iCloud and other File Provider folders):
  online-only files are macOS "dataless" placeholders (`SF_DATALESS` in
  `st_flags`; `is_online_only`). Reading one makes the cloud app
  download it. For watch folders with `download` on (the default),
  `scan_watch_folders` starts a background thread that reads each
  placeholder (at most 4 at once). A file isn't fetched automatically
  again for 10 minutes, in case Box gives the space back.
  `download_file` does the same by hand. Results come back over a
  channel to `poll` and are logged; the status JSON's `downloads` counter
  makes the app reload the library, whose entries say `online_only` and
  `downloading`. Marking files "Make Available Offline" is Box's own
  setting, which other apps can't change.
- **Library player:** library files play on the preview player of the
  `library_output` (the preview output when unset), with pause and
  resume (`pause_preview`, `resume_preview`; `paused` in the preview
  status). Selected files go onto the bank shown with Add to Bank.
- **Several files at once (app):** dropping several files or a folder
  on an empty pad or a gap in the grid makes one pad per file on the
  bank's empty pads (from the pad dropped on). If they don't fit, the app
  asks whether to add banks for the rest or skip them. It's all one undo
  step. ⌥-drop makes a playlist pad instead.
- **Bank settings:** `bank_defaults` gives a bank its own settings for
  new pads, overriding `pad_defaults`
  (`effective_pad_defaults(bank)`). Removing a bank drops its settings.
- **Bulk edit** (`bulk.rs`): `bulk_edit(bank, PadChanges)` changes only
  the settings given, on every pad or on one bank's pads. The settings
  are colour, volume, fades, loop, group, output, duck, level match,
  retrigger and play mode. In the JSON, a field left out is left alone
  and `null` clears it (a `deserialize_with` helper tells the two
  apart). An output that doesn't exist is refused before anything
  changes.
- **Pad looks:** `color` is a preset name or `#RRGGBB` (the remote page
  understands both); `image` is a picture shown on the pad, copied into
  an `Images` folder beside a saved show and found there if the show
  moves (`show::resolve_file` looks in `Audio` and `Images`).
- **Copy / Paste Style** (app only): colour, picture, fades, loop,
  group, output, ducking, retrigger and level matching; not the sound,
  title or key.
- **New-pad defaults:** the show's `pad_defaults` (colour, fades, loop,
  group, output, ducking, skip leading silence, level matching), applied
  by the app when audio is dropped on an empty pad. Settable in Show
  Settings or from a pad (Use Style for New Pads).

## Layout and live transport (Phase 13)

- **Scrubbing:** `Session::seek_cue` moves a playing pad's playhead (the
  current song for playlists) with the usual declick crossfade; Now
  Playing's bars are click/drag targets. `seek_preview` does the same for
  the preview (playing it again if it had ended), and the preview state
  reports its length, for the floating preview player.
- **Fade Last / Stop Last:** the session remembers the order pads started
  (`fire_order`, the last 64); "last" is the most recent one still
  playing. Buttons (⇧Esc / ⌥Esc), MIDI learn (Fade Last), OSC
  `/sbj/fadelast` and `/sbj/stoplast`, and the remote.
- **Multiplay per bank:** on by default (pads overlap). The show's
  `single_play` lists banks with it off: firing a pad there (by any means
  but a marker) fades out the bank's other playing pads over the group
  hand-over time.
- **Layout (per Mac, not in the show):** pads fit the window's height or
  keep a chosen size and scroll; Now Playing across the top (default) or
  on the right; the library, rundown and Now Playing columns are
  resizable.

## Undo and Save With Audio (Phase 14)

- **Undo** (`sbj-session/src/undo.rs`): the app marks each user action
  (`record_undo("Paste Style")`); if the show document changed since the
  last mark, the previous document is pushed (50 steps). Undo/redo swap
  whole documents, then reload only the pads whose settings differ (plus
  pads whose markers reach them; everything if outputs, pads per bank or
  level matching changed), so untouched pads keep playing. The app hangs
  this on the window's `UndoManager`, so Edit → Undo "Paste Style" and ⌘Z
  behave like any Mac app, and text fields keep their own undo. Opening
  or creating a show clears the history.
- **Save With Audio** (`collect.rs`): plans copies of every file the show
  uses (pads, playlists, library, pictures) into `Audio/` and `Images/`
  beside the chosen show file, giving clashing names a number; copies
  without holding the session (pads keep firing), via a temporary name so
  a failed copy never leaves half a file; then rewrites the show's paths
  relative to the folder and re-files the decoded audio under the new
  names (nothing reloads or stops). Missing files are skipped and
  reported. The folder then opens anywhere.

## Pad actions (Phase 15)

Each pad has a **play mode** (`play_mode`): follow the bank's Multiplay
(default), **over** (never fades other pads, and single-play hand-overs
leave it alone, e.g. horns over music) or **solo** (fades out the bank's
other pads even in a Multiplay bank). And a list of **actions**
(`on_fire`, `sbj-session/src/actions.rs`) run in order each time it
starts, however it was fired (including rundown and marker starts):
stop other pads (all, or this bank; with a fade), fire a pad (now, or
after a delay, timed on the control side within a poll), stop a pad,
duck everything else by N dB for X seconds (folded into the ducking
calculation, which also handles per-pad ducking; the deepest wins), and
switch bank. Pads started by actions are logged as `action`. Nesting is
capped at four levels, so pads that fire each other can't loop. Action
targets follow pads that move or are renumbered, the pre-game check
flags actions on empty pads, and pads with actions show a ⚡ tag whose
tooltip lists them.

**Colours:** ready pads draw their exact colour (so it can be matched by
eye or eyedropper); pads that can't play are faded. The show keeps named
**colour presets** (`colors`, `{name, value: "#RRGGBB"}`), listed first
in a pad's Colour menu, then the standard colours, then other custom
colours already used in the show. Changing a preset's colour can
recolour every pad (and the new-pad default) using it
(`set_show_colors(…, recolor)`).

**Played pads** (`played.rs`): the session keeps which pads have been
used this game (not saved with the show, so firing doesn't make the
show "edited" or fill the undo history; cleared when a show opens).
With a played colour set in the show (`played_color`), pads are marked
as they fire (`mark_played`, on by default), however they're fired, and
show in that colour with a ✓ when not playing. Marks can be set by hand,
follow pads that move, and reset per page or for all pages. The remote
shows them too.

## Play log (Phase 7)

`sbj-session/src/log.rs` records each time a pad or playlist song starts:
the time, game name, show, bank and pad, title, the audio file, the
sponsor (a per-pad field), the output, and what triggered it (`operator`,
`midi`, `osc`, `remote`, `marker`, `rundown`, `action`, or `auto` for
playlist auto-advance). Entries close when
the sound stops, finishes or is replaced, and the time played is
measured on the wall clock. Closed entries are appended as JSON lines to
`<log dir>/YYYY-MM-DD.jsonl` (UTC days) straight away, so a crash loses
nothing. The app sets the log directory to
`~/Library/Application Support/Sportsball Jukebox/Logs`. All file
writing happens on the control thread during `poll`, never on the audio
thread.

The Game Log window shows recent plays, totals per song (for music
licensing returns) and per sponsor (for sponsorship fulfilment), and
exports CSV with local dates and times.

**Event log** (`diagnostics.rs`): what went wrong, and when. Each entry
has a time, a level (`info`, `warning`, `error`), a kind (`glitch`,
`device`, `fire`, `show`, `notice`) and a message. Logged:

- glitches (audio xruns counted by the backend's `StreamStatus`): `poll`
  compares each output's count with the last one it saw and logs the
  new ones, naming the output, the device and the pads playing on it;
- outputs opening on a device (with rate and channels), failing to find
  one (only when the error changes, not on every retry), losing their
  device and switching to the backup;
- pads that fail to fire, audio files that couldn't be loaded when a
  show opens, and failed saves;
- every operator notice (`notify` both queues the notice and logs it).

The last 500 events stay in memory (`sbj_session_events_json`); all of
them are appended to `<log dir>/events-YYYY-MM-DD.jsonl`. Resetting the
glitch count (`sbj_session_reset_glitches`) stores an offset per output,
so the status JSON's `xruns` counts from zero while the log keeps the
history. The Event Log window (Show → Event Log…, ⇧⌘E, or click an
output's status in the header) lists them, filters problems or
glitches, resets the count and copies a diagnostics report (Mac model,
macOS and app versions, outputs, devices, recent events) for bug
reports.

## Performance

`cargo run --release -p sbj-engine --example bench` renders 30 s in
128-frame blocks (a 2.7 ms deadline) with 1–128 players running at once,
each looping with a marker. It reports the average and worst time per
block as a share of the deadline. On a GitHub macOS runner (Apple
Silicon):

| Players playing at once | Average | Worst block |
|---|---|---|
| 1 | 0.02% | 0.2% |
| 16 | 0.3% | 1.5% |
| 64 | 1.3% | 2.8% |
| 128 | 2.6% | 5.8% |

That leaves well over 90% of each 2.7 ms deadline spare even with 128
sounds at once. CI reruns this on every macOS build.

## Testing strategy

| Layer | How | Where |
|---|---|---|
| Session, shows, failover | Offline backend with several simulated devices that tests can unplug and re-plug | Linux CI |
| Engine logic | Offline rendering with "ramp" buffers whose sample values encode their source frame index, so output proves exactly which frames played | Linux CI, every push |
| Real-time safety | Allocation-counting global allocator during `process` | Linux CI |
| C ABI | Rust tests, plus `tests/smoke.c` compiled against the public header | Linux and macOS CI |
| Core Audio backend | Device tests on a real Mac | Borrowed Mac |
| UI | Manual testing; XCUITest later | Borrowed Mac |

## Build and distribution

- Rust toolchain pinned in `rust-toolchain.toml`.
- **Minimum macOS: 14.2 (Sonoma)**, required by cpal's Core Audio backend.
- The Mac app lives in `apple/`. Its Xcode project is generated from
  `apple/project.yml` by XcodeGen and is not committed. A pre-build phase
  runs `scripts/build-rust-apple.sh`, which builds the engine as
  `target/apple/libsbj_ffi.a`. Swift imports the C header through the
  `SbjEngineC` module map, and the app checks `sbj_abi_check` at launch.
- CI (`.github/workflows/ci.yml`): Linux runs fmt, clippy, tests and the C
  smoke test on every push. macOS runs tests, then builds a universal
  (arm64 + x86_64) ad-hoc-signed `Sportsball Jukebox.app`, zipped as a
  downloadable artifact, on every push, PR, tag and manual run (free,
  as the repo is public). Manual runs on a branch also commit fresh
  screenshots.
- No Apple Developer account yet, so builds are ad-hoc signed, not
  notarized. See `docs/testing-on-mac.md` for opening them.

## Roadmap

**Phase 0: Foundations (done)**
Engine core, marker actions, real-time safety test, offline backend, C ABI,
CI.

**Phase 1: First sound on a Mac (built; awaiting Mac testing)**
- Multiple players per engine, so cues overlap. Restart, per-player gain,
  Stop All with fade.
- File decoding (Symphonia) and high-quality resampling at load time.
- Core Audio output through cpal: device list by persistent id, one stream
  per device, device-loss and glitch reporting.
- Session layer and C API for the app.
- SwiftUI app: 24-cue grid with keyboard hotkeys, drag-and-drop or click
  to load, progress display, loop/stop/fade/clear per cue, output device
  picker, master volume, Stop All (Esc), disconnect banner.
- CI builds the `.app`.
- Next: first test session on the borrowed Mac (`docs/testing-on-mac.md`).

**Phase 2: Show building (built; awaiting Mac testing)**
- Show files: versioned JSON, atomic saves, autosave, reopen last show,
  moved-show audio lookup, per-pad problem reporting.
- Several interfaces at once: logical outputs, one engine each, pads
  routed by output, preferred + backup devices, automatic failover that
  resumes playback, retrying offline outputs.
- Pad editor: timeline with waveform clips (move, trim, fades, volume)
  and markers (jump/loop/pause/stop/fade-out; targets by time or marker;
  repeat counts) with arrows to targets, play from cursor, live playhead.
- Configurable hotkeys and colours, drag pads to rearrange, grid size.
- Engine fix: a gain set while a player is stopped applies instantly.

**Phase 3: Live operation and polish (built; awaiting Mac testing)**
- Declick crossfades on jumps, loops, retriggers and seeks; soft operator
  stop.
- Banks of pads (⌘1–9, tabs, MIDI, OSC).
- MIDI triggers with learn; show-wide MIDI Stop All and bank switching.
- OSC control (also the Stream Deck route).
- Pre-game check.
- VoiceOver labels on pads and controls.
- Engine benchmark; screenshot mode for docs.
- Signing and notarization documented (`docs/releasing.md`) for when there
  is an Apple Developer account.

**Phase 4: Game-day workflow (built; awaiting Mac testing)**
- Vamps: loops that hold until the pad is pressed again, then exit at
  the loop point.
- Retrigger behaviour per pad (restart / stop / fade).
- Exclusive groups with crossfaded hand-over.
- Ducking other pads on the same output while a pad plays.
- Per-pad output channel routing.

**Phase 5: Playlists and fades (built; awaiting Mac testing)**
- Playlist pads: in order or shuffled; one song per press or carrying on
  with crossfades; Next Song from the menu, MIDI (fire) or OSC
  `/sbj/next/<pad>`. Dropping several files on a pad makes one.
- Per-pad fade-in on fire and fade-out length; show-wide Stop All fade.

**Phase 6: Operator awareness and preview (built; awaiting Mac testing)**
- Now Playing panel: everything playing, soonest to end first, with a
  large countdown, the next marker or playlist song, and stop and fade
  buttons.
- Pads turn red in their last 5 seconds.
- Headphone preview: ⌥-click a pad (or Preview in its menu or editor) to
  hear it on the preview output only.

**Phase 7: Game log and reports (built; awaiting Mac testing)**
- Play log of every pad and playlist song, with source, sponsor, output
  and time played; written to daily files as it happens.
- Game name stamped on entries; per-pad sponsor.
- Game Log window: plays, song totals, sponsor totals, CSV export.

**Phase 8: Phone/tablet remote (built; awaiting Mac testing)**
- Web page served by the app on the local network: banks, pads with
  colours and progress, Now Playing with countdowns, Stop, GO, Stop All.
- 6-digit PIN, QR code that signs in, presses logged as `remote`.

**Phase 9: Pad markers (built; awaiting Mac testing)**
- Marker actions Fire pad, Stop pad and Volume. Sample-exact on the same
  output (the renderer lines all players up at such markers); within a
  poll on another output.
- References follow pads that move; plays are logged as `marker`.

**Phase 10: Level matching and silence (built; awaiting Mac testing)**
- EBU R128 loudness, peak and silence measured for every file.
- Show-wide level matching to a target (with boost and peak limits),
  per-pad opt-out, playlists matched song by song.
- Dropped files skip leading silence; Trim Silence in the editor.

**Phase 11: Game rundown (built; awaiting Mac testing)**
- Ordered list of pad steps and notes; standby, GO (Space), back/next.
- GO from MIDI, OSC and the phone remote; steps follow moved pads.

**Phase 12: Show building (built; awaiting Mac testing)**
- Audio library sidebar (import files/folders, search, preview, drag
  onto pads or into a pad's timeline).
- Custom pad colours and pictures; Copy/Paste Style; defaults for new
  pads.

**Phase 13: Layout and live transport (built; awaiting Mac testing)**
- Pads fit the window or keep a size and scroll; Now Playing across the
  top; resizable side panels.
- Scrub bars in Now Playing; floating preview player with scrubbing.
- Fade Last / Stop Last; Multiplay switch per bank.

**Phase 14: Undo and Save With Audio (built; awaiting Mac testing)**
- Edit → Undo / Redo for show edits, reloading only what changed.
- File → Save With Audio… makes a self-contained show folder.

**Phase 15: Pad actions (built; awaiting Mac testing)**
- Play mode per pad (follow bank / play over / solo).
- On-fire actions: stop others, fire or stop a pad (with delay/fade),
  duck everything else for a while, switch bank; ⚡ tag on the pad.

**Next candidates**
- Signed releases once there is a Developer account.
