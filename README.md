# Sportsball Jukebox

A high-performance, high-reliability sound board for live sports, built
as a native macOS app. Inspired by Sports Sounds Pro, it adds
sample-accurate timeline markers with actions (loop, jump, pause, fade) and
support for several audio interfaces at once.

**Status:** Phase 0. The Rust audio engine, marker system and CI are in
place. The macOS app comes next. See [docs/architecture.md](docs/architecture.md).

## Layout

```
crates/
  sbj-engine/    real-time engine: timeline, markers, mixing (platform-independent)
  sbj-backend/   audio device abstraction + offline backend for tests
  sbj-ffi/       C ABI for the Swift app (header in include/)
docs/            architecture and design notes
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

Engine work can be done and tested on Linux, macOS or Windows. The macOS
app itself (Phase 1+) needs a Mac to run; CI builds it on GitHub's macOS
runners.

## CI

- **Linux:** fmt, clippy, tests and the C API smoke test on every push.
- **macOS:** tests plus a universal `libsbj_ffi.a` build artifact. Runs on
  pull requests, pushes to `main`, and manual runs (Actions → CI → Run
  workflow), to save macOS minutes.
