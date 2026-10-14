# Perch — notes for agents

A native Twitch client: Rust + GPUI (Zed's UI framework, vendored with
patches under `vendor/gpui`), streamlink as the byte source, libmpv for decode.
Windows first; macOS builds and ships; Linux compiles and is not claimed.

This file is an index. The facts live in two places, and each is the one source
for what it covers:

- `README.md` — using it: what every control does, the requirements, the keys.
- `HANDOFF.md` — working on it (about 5,000 lines, so read it by section, with
  `grep -n '^#' HANDOFF.md` for the map). Before changing anything, read:
  - **Product decisions** — who it is for and what was declined. A feature that
    cuts across one is a conversation with the user, not a cleanup.
  - **Things not to redo** — the traps that cost real time. An idea that needs
    one of them is wrong.
  - the section for the area you are touching (Video, Recordings, Chat,
    GPUI / gpui-component, Keyboard, Browsing, Windows packaging, ...).
  - **What to build next** and **Known limits** — the backlog, and what is
    real but accepted.

## Checks

The toolchain is pinned (`rust-toolchain.toml`). CI runs, and a change is not
done until it passes, locally:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
bash scripts/verify-vendor.sh
```

Before a release, `bash scripts/live-check.sh` asks every service perch rides
without a contract (unpublished GQL, the history service, the emote providers,
streamlink's rendition names) once; it is kept out of CI on purpose.

`verify-vendor.sh` proves `vendor/gpui` is crates.io's gpui plus exactly the
hunks tagged `PERCH PATCH`; a new patch goes in the root `Cargo.toml`'s list
and in the script. The two `float_literal_f32_fallback` warnings from
`vendor/gpui/src/taffy.rs` are upstream's and expected.

`cfg(unix)` / macOS code cannot be checked by a Windows build. Type-check it by
`#[path]`-including the module into a scratch crate (stub crates for what it
imports) and running
`cargo +1.98.0 clippy --offline --target aarch64-apple-darwin --all-targets -- -D warnings`.
Prefer two `cfg`'d definitions of a function to a `cfg` on statements inside
one: the latter left a `needless_return` that only the macOS leg caught.

## Layout

- `crates/perch` — the app. `root/` is the window's state and behaviour split
  by concern (`streams`, `follows`, `pane_actions`, `renditions`, ...);
  `watch/`, `chat/`, `video_view/` are the watch page, a pane's chat and its
  player. `video.rs` owns the mpv render thread.
- `crates/streamlink` — runs streamlink; the version check, the relay, ads.
- `crates/twitch-api` — Helix, plus the two unpublished GQL queries
  (`recommend`) the app rides knowingly.
- `crates/twitch-chat` — IRC, backfill from the history service, VOD replay.
- `crates/emotes` — emote sets and the on-disk image cache.
- `crates/mpv-frames` — libmpv loaded at runtime, software render to BGRA.
- `crates/settings` — `settings.json` and the history file.

At runtime: settings in `%APPDATA%\perch\settings.json`; the log, the crash
file and the image cache in `%LOCALAPPDATA%\perch\` (`perch.log`, `.log.old`,
`perch.crash.log`, `images\`).

## Running a test copy

The user runs perch daily while it is worked on.

- Perch is single-instance: a second `perch.exe` hands its arguments to the
  running one and exits. Run a test build with its own `APPDATA` and
  `LOCALAPPDATA` (copies, never the user's own files), and back up the user's
  files before any session that might touch them.
- Stop a test copy **by the PID you started**, never by process name: that
  closes the user's perch too.
- Drive it with posted window messages (`PostMessage`) and capture it with
  `PrintWindow`; never move the real mouse or send global keys.
- Never open twitch.tv in a browser pane: a hidden page once played an ad at
  full volume. Read Twitch through its APIs.

## Writing here

Match the code around you: prose comments that say why, sentence case on
every control's words, the lowercase passive tags in a pane's header (`muted`,
`paused`, `buffering`, `replay`, `ad break`). When a change makes HANDOFF or
README wrong, fix them in the same change.
