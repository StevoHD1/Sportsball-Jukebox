# Sportsball Jukebox: Architecture

Status: Phase 6 (operator awareness and preview). Last updated 2026-11-24.

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

### Planned marker actions

- Trigger another cue or timeline, possibly on another interface.
- Jump to a named marker instead of a raw position.
- Conditional / "armed" markers: loop until the operator presses a key,
  then continue at the next musical boundary. This is common for walk-up
  music and intros.
- Volume ramp to a level, rather than only fade-to-stop.
- Short crossfades on jumps and loops (a few milliseconds) to remove clicks
  at the edit point.

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
          "action": { "kind": "loop", "to": { "marker": 1 }, "count": 2 } } ] }
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
  downloadable artifact. It runs only on PRs, `main` and manual runs,
  because macOS minutes cost 10× on private repos.
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

**Next candidates**
- Recording a game log (what fired when) for sponsor or licensing
  reports.
- Signed releases once there is a Developer account.
