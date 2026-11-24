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
- [ ] App opens; the header shows the output (e.g. "48.0 kHz · 2 ch") with
      a green dot.
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
- [ ] The output picker lists the interface. Selecting it moves playback
      there.
- [ ] Unplug the interface during playback: a red "device disconnected"
      banner appears and the app doesn't crash. Pick another output and
      fire again.
- [ ] Plug it back in, click the rescan button (↻), and reselect it.

**Stress**
- [ ] Leave a looping cue running for 10+ minutes. The status line should
      not start showing "glitches".
- [ ] Open other heavy apps while playing.

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
