# Notes for Claude

Read `docs/architecture.md` before changing the engine.

## Hard rules

- **Real-time safety.** Nothing reachable from `Renderer::process`
  (`crates/sbj-engine/src/render.rs`) may allocate, free, lock, block, do
  I/O or log. Build objects on the control side and send them in; send
  replaced objects back through the garbage queue.
  `tests/realtime_safety.rs` must stay green, and new command or action
  paths must be exercised there.
- **Sample accuracy.** Positions are `u64` sample frames. New timeline
  behaviour needs an offline-render test in `tests/markers.rs` run across
  `BLOCK_SIZES`.
- **C ABI.** `crates/sbj-ffi/include/sportsball_jukebox.h` is hand-written.
  Update it with every exported-function or struct change; `tests/smoke.c`
  checks it, and `sbj_abi_check` guards struct sizes at app launch. Every
  exported function must be panic-safe and null-tolerant, and set
  `sbj_last_error_message` on failure.
- **Swift can't be compiled here.** `apple/` only builds on macOS CI (Run
  workflow on the branch). Keep Swift conservative (Swift 5 mode,
  macOS 14.2 APIs) and lint the Rust side for macOS with
  `cargo clippy --workspace --all-targets --target aarch64-apple-darwin`.

## Before committing

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings
```
