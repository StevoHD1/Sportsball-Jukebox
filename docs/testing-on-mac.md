# Testing on a Mac

The app is built by GitHub Actions, so the Mac only needs to run it. You
don't need Xcode or Rust unless you want to build locally (see the end of
this page).

**Requirements:** macOS 14.2 (Sonoma) or later, on Apple Silicon or Intel.

## 1. Download the app

If there is a release (GitHub → **Releases**), download its
`Sportsball-Jukebox-<version>.dmg`. Otherwise:

1. On GitHub, open the repo → **Actions** → **CI**.
2. Pick a run with a green **Test and build app (macOS)** job. To build a
   branch on demand: **Run workflow**, choose the branch, run it, and wait
   about 10 minutes.
3. Under **Artifacts**, download **Sportsball-Jukebox-macOS** (a zip) and
   double-click it to get `Sportsball-Jukebox-<version>.dmg`.

Open the disk image and drag **Sportsball Jukebox** onto **Applications**.
(`docs/install.md` is the short version of this page for anyone you
give the app to.)

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

If macOS asks whether Sportsball Jukebox may find devices on your local
network, click **Allow** (the phone remote and OSC need it).

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
- [ ] Plug in any MIDI controller. Show → Control shows it. Right-click
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

**Phase 6: operator awareness and preview**
- [ ] The Now Playing panel (right side; ⇧⌘P toggles it) lists playing
      pads, soonest to end first, with a large countdown.
- [ ] A pad with markers shows "→ Vamp in 0:03.2"; a vamping pad shows
      VAMP and "Press the pad to go"; a playlist shows "Up next: <song>".
- [ ] In the last 5 seconds a pad's progress bar and its countdown turn
      red.
- [ ] Audio Outputs → Preview (headphones) output: choose your headphone
      output. ⌥-click a pad: it plays only in the headphones, and the
      panel shows a blue Preview row with a stop button. ⌥-click again
      stops it. Groups and ducking are not affected.

**Phase 7: game log**
- [ ] Show → Game Log (⇧⌘L): set the game name, fire a few pads, refresh.
      Each play is listed with time, pad, song, what triggered it and how
      long it played.
- [ ] Give a pad a sponsor in its editor and fire it: the Sponsors tab
      counts it.
- [ ] Export CSV and open it in Numbers or Excel.
- [ ] Open Log Folder: today's file contains a line per play, even after
      quitting.

**Phase 8: phone/tablet remote** (needs a phone on the same Wi-Fi)
- [ ] Show → Control → turn on the remote. If macOS asks about incoming
      connections, Allow. The window shows an address, a PIN and a QR code.
- [ ] Scan the QR code with the phone camera: the page opens without
      asking for the PIN and shows the pads in their colours.
- [ ] Tap pads: they fire on the Mac within a fraction of a second; the
      Mac's bottom bar says "Remote …".
- [ ] Switch banks on the phone: the Mac follows.
- [ ] Now Playing rows count down; Stop stops a pad; STOP ALL fades all.
- [ ] Press New PIN on the Mac: the phone asks for the PIN; the old one
      no longer works.
- [ ] Game Log: remote presses are listed with the source "remote".
- [ ] Turn the remote off: the phone shows "Offline — retrying".

**Phase 9: markers that fire and stop pads**
- [ ] In a pad editor, add a marker and set its action to **Fire Pad**,
      choosing a short sound (e.g. a horn) on the same output. Fire the
      pad: the horn lands exactly on the marker every time, in time with
      the music. The editor says "Same output: sample-exact".
- [ ] Put the marker on a beat and loop the section (Loop marker): the
      horn hits on the same beat each pass.
- [ ] **Stop Pad**: fire a crowd loop, then a song with a Stop Pad marker
      for the crowd loop: it fades at the marker.
- [ ] **Volume**: add a marker at -12 dB over 0.5 s, and a later one at
      0 dB: the music dips, then comes back. Fire again: it starts at
      full volume.
- [ ] Choose a target on another output (or a playlist pad): it still
      fires, a moment after the marker.
- [ ] Drag the target pad to another spot in the grid: the marker still
      fires it. Game Log lists those plays with the source "marker".

**Phase 10: level matching and silence**
- [ ] Drop a file that has silence at the start on a pad: it fires with
      no gap. The editor's clip row shows its loudness and peak.
- [ ] Show Settings → Level-match pads at -16 LUFS. Fire a quiet song and
      a loud one: they sound about equally loud. The clip row shows
      "→ plays +x dB".
- [ ] Turn Level-match off for one pad in its editor: it plays at its own
      level again.
- [ ] A playlist of songs from different sources: each song plays at a
      similar level.
- [ ] Trim Silence on a file with silence at the end: the pad ends (and a
      loop wraps) without a gap.

**Phase 11: game rundown**
- [ ] Right-click a few pads → Add to Rundown. The Rundown panel opens
      on the left (⇧⌘R toggles it). Add a note ("Timeout 1") with the
      button at the top.
- [ ] Drag steps to reorder; Delete removes the selected step.
- [ ] Press Space: the standby step fires and the next one is on standby;
      notes are skipped. ⌘→ / ⌘← move standby without firing; clicking a
      step puts it on standby.
- [ ] Control → learn "Rundown GO" to a controller button: it fires the
      steps too. So does the GO bar on the phone remote.
- [ ] Unplug the interface and press GO: an error appears and standby
      stays on the same step.

**Phase 12: show building**
- [ ] Show → Show Library (⌥⌘L): add a folder of audio with +. Search
      for a file; click the headphones to preview it (needs a preview
      output); drag it onto an empty pad.
- [ ] Open a pad's editor, click Library in its toolbar, and drag two
      files onto the timeline: they become clips where you dropped them.
- [ ] In a pad's editor, pick a custom colour with the colour well, and
      choose a picture (or drop an image file on a pad). The title stays
      readable; the pad still turns green while playing.
- [ ] Save the show, copy its folder (with Audio and Images) elsewhere,
      open it there: pictures still show.
- [ ] Right-click a pad → Copy Style, then another → Paste Style: it
      takes the colour, picture, fades, group and so on, but keeps its own
      sound, title and key.
- [ ] Show Settings → New pads: set a colour and fade-out; drop a sound
      on an empty pad: it gets them. Right-click a pad → Use Style for New
      Pads does the same from a pad.

**Phase 13: layout and live transport**
- [ ] Show Settings → Pads per bank: raise it (e.g. 48). In the bank bar,
      turn **Fit** on: all pads fit the window and resize with it. Turn it
      off and use the size slider (or ⌘+ / ⌘−): pads keep that size and
      the grid scrolls.
- [ ] Now Playing is a strip across the top; Show → "Now Playing on the
      Right" moves it back. Drag the edge of the Library, Rundown or
      Now Playing column to resize it; the width is remembered.
- [ ] While a pad plays, click partway along its bar in Now Playing: it
      jumps there without a click. Drag along the bar to scrub.
- [ ] Fire two pads, press **Fade Last** (⇧Esc): the second fades; press
      it again: the first fades. Stop Last (⌥Esc) cuts instead.
- [ ] Turn **Multiplay** off for a bank, fire one pad then another: the
      first fades out. Other banks are unaffected.
- [ ] Set a Preview output (Audio Outputs), right-click a pad → Preview:
      the preview player appears bottom right. Click or drag its bar to
      hear the middle or end; ⏮ restarts; ✕ stops.

**Phase 14: undo and Save With Audio**
- [ ] Paste Style onto a pad, then Edit → Undo Paste Style (⌘Z): it's
      back as it was. ⇧⌘Z redoes it. Try Clear Pad, dragging pads to swap,
      and dropping a new sound on a pad, then undo each.
- [ ] While a pad plays, change and undo a different pad: the playing pad
      carries on.
- [ ] Typing in a pad's title and pressing ⌘Z undoes the typing, not a
      show change.
- [ ] File → Save With Audio… (⌥⌘S): pick a name. A folder appears with
      the show, Audio and Images; pads keep firing while it copies. Copy
      that folder to a USB stick or another Mac and open the show there:
      every pad, picture and library file is found.

**Phase 15: pad actions**
- [ ] Turn Multiplay off for a bank. Open a horn pad's editor →
      Actions… → Play mode "Play over everything". Fire the music, then
      the horn: the music keeps playing. Fire another pad: the music hands
      over, the horn carries on. The horn shows ⚡ and its tooltip says why.
- [ ] Set a pad to "Solo" in a Multiplay bank: firing it fades out the
      bank's other pads.
- [ ] Add actions to a pad: Stop other pads; Fire a pad after 2 s; Duck
      everything else -15 dB for 4 s; Switch to bank. Fire it from a key,
      MIDI and the rundown: each time all of them happen, in order.
- [ ] Make two pads fire each other: firing one doesn't hang the app.
- [ ] Pad editor → Colour menu → Save This Colour as a Preset…: it's
      listed under Show colours on every pad, and in Show Settings → New
      pads. Show → Colour Presets…: change its colour with "Recolour pads"
      on: every pad using it changes. Save, open the show on another Mac:
      the presets are there.
- [ ] A custom-coloured pad looks the same colour as its swatch in the
      Colour menu (not darker).

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
