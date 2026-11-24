# Testing on a Mac

The app is built by GitHub Actions, so the Mac only needs to run it. You
don't need Xcode or Rust unless you want to build locally (see the end of
this page).

**Requirements:** macOS 14.2 (Sonoma) or later, on Apple Silicon or Intel.

## 1. Download the app

1. On GitHub, open the repo → **Actions** → **CI**.
2. Pick a run with a green **Test and build app (macOS)** job. To build a
   branch on demand: **Run workflow**, choose the branch, run it, and wait
   about 10 minutes.
3. Under **Artifacts**, download **Sportsball-Jukebox-macOS**. It's a zip
   that contains another zip; double-click until you get
   `Sportsball Jukebox.app`.
4. Drag the app to **Applications** (optional).

## 2. Open it the first time

The app isn't notarized (that needs an Apple Developer account), so macOS
blocks the first launch:

1. Double-click the app. macOS says it can't verify it. Click **Done**.
2. Open **System Settings → Privacy & Security**, scroll down, and click
   **Open Anyway** next to "Sportsball Jukebox was blocked…". Confirm.

If that option doesn't appear, run this in Terminal instead:

```sh
xattr -dr com.apple.quarantine "/Applications/Sportsball Jukebox.app"
```

You only need to do this once for each downloaded build.

## 3. Test checklist

Bring a few audio files (WAV, AIFF, MP3, M4A/AAC, FLAC), including one
long track (3+ minutes) and one at 44.1 kHz. If you have an audio
interface, bring that too.

**Basics**
- [ ] App opens; the header shows "Main: <device name>" with a green dot.
- [ ] Click an empty pad, choose a file: the pad shows the title and
      duration.
- [ ] Drag a file from Finder onto a pad: same result.
- [ ] Click a loaded pad: sound plays immediately; the pad turns green and
      shows progress and the time remaining.
- [ ] Press the pad's key (1–0, Q–P, A–F): it fires.
- [ ] Clicking a playing pad restarts it from the top.

**Overlap and control**
- [ ] Fire 3–4 pads at once: they all play together, with no crackles.
- [ ] Right-click → Fade Out: smooth half-second fade.
- [ ] Right-click → Stop: stops instantly, with no click.
- [ ] Right-click → Loop, then fire: it loops seamlessly at the end.
- [ ] **Esc / Stop All**: everything fades out.
- [ ] The master volume slider changes the level smoothly (no zipper noise).

**Responsiveness**
- [ ] Hammer one hotkey rapidly: every press retriggers, with no lag.
- [ ] Load a long file while another pad is playing: playback doesn't
      glitch, and the spinner shows while it loads.
- [ ] 44.1 kHz files play at the correct pitch and speed.

**Devices** (with an interface)
- [ ] Show → Audio Outputs… lists the interface. Choose it for "Main" and
      Apply: the header badge shows it, and playback moves there.
- [ ] Unplug the interface during playback with no backup set: a red
      banner appears and the app doesn't crash. Plug it back in: it
      recovers by itself within a few seconds.

**Shows (Phase 2)**
- [ ] Load a few pads, quit, relaunch: the same show comes back.
- [ ] File → Save As… to a folder; edit something; after ~2 s the title
      bar's "— Edited" disappears (autosaved).
- [ ] Copy the show file plus an `Audio` folder with its sounds to
      another location (or a USB stick), open it there: everything plays.
- [ ] Rename one of the audio files and reopen the show: that pad shows a
      ⚠️ and its tooltip says which file is missing; other pads still work.
- [ ] Drag one pad onto another: they swap.
- [ ] Show → Show Settings: change the pad count and columns.

**Pad editor** (right-click a pad → Edit Pad…)
- [ ] The clip shows a waveform. Drag it along the timeline.
- [ ] Click the ruler to set the cursor, then press ▶: playback starts
      there and the green playhead moves.
- [ ] Add Clip… adds a second file after the first; both play in sequence.
- [ ] Add a marker, set it to **Loop** back to an earlier time, repeat 2×:
      the section plays three times, then continues. Listen for clicks at
      the loop point.
- [ ] Set a marker's target to another marker and drag that marker: the
      yellow arrow follows.
- [ ] Fade Out marker: smooth fade, then stop.
- [ ] Change the pad's key; a warning appears if another pad uses it.
- [ ] Cancel discards changes; Done keeps them.

**Multiple interfaces** (needs two outputs, e.g. an interface + the
built-in speakers or headphones)
- [ ] Show → Audio Outputs…: add a second output on another device. Route
      a pad to it in its editor. Fire pads on both outputs at once.
- [ ] Give "Main" a backup device. While a pad on Main plays, unplug
      Main's interface: within a moment it carries on from the same spot
      on the backup, an orange notice appears, and the header shows
      "(backup)".
- [ ] Plug it back in: reselect it in Audio Outputs to move back.
- [ ] Unplug a device with no backup and no other device: red banner,
      pads say "Not ready", the app keeps running; plug it back in and it
      recovers by itself.

**Phase 3: live operation**
- [ ] Loop markers and retriggering a playing pad sound smooth, with no
      click at the jump.
- [ ] Show → Show Settings: add a second bank. Switch with the tabs and
      ⌘1/⌘2; each bank's keys fire only its own pads.
- [ ] Plug in any MIDI controller. Show → MIDI & OSC shows it. Right-click
      a pad → Learn MIDI Trigger → hit a pad on the controller: the pad
      gets a 🎹 badge and that control now fires it. The bottom bar shows
      what each message did.
- [ ] Learn Stop All and Next Bank to controller buttons; test them.
- [ ] Turn on OSC (port 53100). From another device on the network (a
      phone with an OSC app, or `oscsend <ip> 53100 /sbj/fire/1`), fire
      pads and switch banks.
- [ ] Stream Deck (if you have one): set a button to send OSC
      `/sbj/title` with a pad name (see `docs/control.md`).
- [ ] Show → Pre-game Check: unplug a device or rename an audio file and
      check again; the problem is listed in red.
- [ ] Turn on VoiceOver (⌘F5) and move over the pads: each reads its
      title, state and key.

**Phase 4: game-day workflow**
- [ ] In a pad editor, add a Loop marker and set Repeat to "Until the pad
      is pressed (vamp)". Fire the pad: it loops and shows "VAMP · press
      to go". Press its key: it finishes the pass and plays on to the end.
- [ ] Give two pads the group "Music". Fire one, then the other: the
      first fades out over a second.
- [ ] Set a pad to "Duck others" at -12 dB and fire it while music plays:
      the music dips, then comes back when it ends.
- [ ] Set "Pressed again" to Stop on a pad: a second press stops it.
- [ ] With a multichannel interface, set a pad's Channels to 3–4: it
      plays only there.

**Phase 5: playlists and fades**
- [ ] Drag three songs onto an empty pad at once: it becomes a playlist
      ("♫ 3 songs"). Fire it: the pad shows "♫ 1/3 · <song>".
- [ ] Press it again while playing: it crossfades to the next song.
- [ ] Let it play: at the end of each song it crossfades into the next
      without a gap, and wraps around after the last one.
- [ ] In its editor, turn on Shuffle, reorder songs by dragging, and set
      the crossfade. Right-click the pad → Next Song.
- [ ] Give a pad a 1 s fade-in: it swells up when fired. Give it a 3 s
      fade-out: right-click → Fade Out takes 3 s.
- [ ] Show Settings → Stop All fade 2 s: Esc now takes 2 s.

**Stress**
- [ ] Leave a looping cue running for 10+ minutes. The status line should
      not start showing "glitches".
- [ ] Open other heavy apps while playing.
- [ ] Open a show with many long files: the spinner shows, and pads in
      the current show keep firing while it loads.

Note anything odd, especially clicks, delays, or a glitch count in the
header, and the Mac model and macOS version.

## Building locally (optional)

```sh
# One-time setup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
brew install xcodegen
xcode-select --install   # or install Xcode from the App Store

# Each time
cd apple && xcodegen generate && open SportsballJukebox.xcodeproj
```

Then press ⌘R in Xcode. The project's pre-build step compiles the Rust
engine automatically.
