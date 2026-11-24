# Sportsball Jukebox

A high-performance, high-reliability sound board for live sports, built
as a native macOS app. Inspired by Sports Sounds Pro, it adds
sample-accurate timeline markers with actions (loop, jump, pause, fade) and
support for several audio interfaces at once.

**Status:** Phase 1. A first playable Mac app: a 24-pad cue grid with
hotkeys, overlapping playback, looping, fades, Stop All and output device
selection, on top of the Rust engine. Requires macOS 14.2+. See
[docs/architecture.md](docs/architecture.md), and
[docs/testing-on-mac.md](docs/testing-on-mac.md) to try it.

## Layout

```
apple/           macOS app (SwiftUI); Xcode project generated from project.yml
crates/
  sbj-engine/    real-time engine: timeline, markers, players, mixing
  sbj-backend/   audio devices: Core Audio (via cpal) + offline backend for tests
  sbj-media/     file decoding and sample-rate conversion
  sbj-session/   app model: cues, output device, firing
  sbj-ffi/       C ABI for the Swift app (header in include/)
scripts/         build helpers (Rust engine → universal static library)
docs/            architecture, design notes, Mac testing guide
```

## Development

Requires only Rust (version pinned in `rust-toolchain.toml`; `rustup`
installs it automatically).

```sh
cargo test --workspace                                # all tests
cargo clippy --workspace --all-targets -- -D warnings # lint
cargo fmt --all                                       # format
cargo build --release -p sbj-ffi --target aarch64-apple-darwin  # Mac static lib
```

All Rust crates build and test on Linux, macOS or Windows; on Linux the
real-device backend is a stub and tests use the offline backend. The Mac
app needs a Mac to run. CI builds it on GitHub's macOS runners, and
[docs/testing-on-mac.md](docs/testing-on-mac.md) covers building it
locally.

## CI

- **Linux:** fmt, clippy, tests and the C API smoke test on every push.
- **macOS:** tests, then builds `Sportsball Jukebox.app` (universal,
  ad-hoc signed) as a downloadable artifact. Runs on pull requests, pushes
  to `main`, and manual runs (Actions → CI → Run workflow), to save macOS
  minutes.
