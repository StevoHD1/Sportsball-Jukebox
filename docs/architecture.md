# Sportsball Jukebox: Architecture

Status: Phase 0 (engine foundations). Last updated 2026-11-24.

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
│ macOS app — Swift (SwiftUI + AppKit)                     │
│  cue grid, timeline editor, marker editor, device setup  │
└───────────────▲──────────────────────────────────────────┘
                │ C ABI (crates/sbj-ffi, include/sportsball_jukebox.h)
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
│  CoreAudioBackend (Phase 1) · WASAPI/ASIO (maybe later)  │
└──────────────────────────────────────────────────────────┘
```

| Crate | Role |
|---|---|
| `sbj-engine` | Timeline model, marker actions, mixing, fades, transport. No I/O, no platform code. |
| `sbj-backend` | Device abstraction. Opens an output on a device and calls a render callback on the real-time thread. |
| `sbj-ffi` | C ABI for the Swift app. Every call is panic-safe and tolerates null pointers. |

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

If sync is needed later, macOS aggregate devices (which apply drift
correction) are the first thing to try, since they need no engine changes.

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

- **Commands** (control → audio): `SetTimeline`, `Play`, `Pause`, `Stop`,
  `Seek`, `SetMasterGain`, `FadeOutAndStop`. Drained at the start of each
  audio block.
- **Events** (audio → control): `MarkerReached`, `Started`, `Paused`,
  `Stopped`, `Seeked`, `ReachedEnd`, `TimelineLoaded`. `Copy` types, so
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
  resampled to the engine rate when loaded (Phase 1), never during playback.
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

### Planned marker actions

- Trigger another cue or timeline, possibly on another interface.
- Jump to a named marker instead of a raw position.
- Conditional / "armed" markers: loop until the operator presses a key,
  then continue at the next musical boundary. This is common for walk-up
  music and intros.
- Volume ramp to a level, rather than only fade-to-stop.
- Short crossfades on jumps and loops (a few milliseconds) to remove clicks
  at the edit point.

## Reliability features (planned)

- **Device failover**: when an interface disappears, re-route its cues to a
  configured backup device and show it clearly in the UI.
- **Crash isolation**: the engine never depends on the UI thread, and a
  frozen UI keeps playing. Show files autosave and recover after a crash.
- **Pre-flight checks**: on show load, verify every file decodes, every
  routed device exists, and memory use is acceptable.
- **No disk I/O at fire time**: all audio for a show is decoded into memory
  at load time.

## Testing strategy

| Layer | How | Where |
|---|---|---|
| Engine logic | Offline rendering with "ramp" buffers whose sample values encode their source frame index, so output proves exactly which frames played | Linux CI, every push |
| Real-time safety | Allocation-counting global allocator during `process` | Linux CI |
| C ABI | Rust tests, plus `tests/smoke.c` compiled against the public header | Linux and macOS CI |
| Core Audio backend | Device tests on a real Mac | Borrowed Mac |
| UI | Manual testing; XCUITest later | Borrowed Mac |

## Build and distribution

- Rust toolchain pinned in `rust-toolchain.toml`.
- CI (`.github/workflows/ci.yml`): Linux runs fmt, clippy, tests and the C
  smoke test on every push. macOS runs tests and builds a universal
  (arm64 + x86_64) `libsbj_ffi.a` plus header, uploaded as a build artifact.
  It runs only on PRs, `main` and manual runs, because macOS minutes cost
  10× on private repos.
- No Apple Developer account yet, so builds are unsigned. On the test Mac,
  right-click → Open the first time, or run
  `xattr -dr com.apple.quarantine SportsballJukebox.app`.

## Roadmap

**Phase 0 — Foundations (done)**
Engine core, marker actions, real-time safety test, offline backend, C ABI,
CI.

**Phase 1 — First sound on a Mac**
- File decoding (Symphonia: WAV/AIFF/MP3/AAC/FLAC) and resampling at load
  time.
- `CoreAudioBackend`: device enumeration by UID and one output stream per
  device.
- Xcode project: SwiftUI shell linking `libsbj_ffi.a`, with a cue grid that
  fires preloaded sounds.
- CI builds an unsigned `.app` as a downloadable artifact.
- First test session on the borrowed Mac: latency, glitches, device
  switching.

**Phase 2 — Show building**
Timeline editor with waveforms, marker/action editor, show file format
(versioned, autosaved), hotkeys, multiple interfaces with routing and
failover.

**Phase 3 — Live operation and polish**
MIDI/OSC/Stream Deck triggers, hot-cue banks, a "panic" fade-all control,
pre-flight checks, accessibility, performance profiling, signing and
notarization.
