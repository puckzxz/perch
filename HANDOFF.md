# Handoff

For whoever picks this up next. `README.md` covers *using* it; this covers
*working on* it — the architecture, the traps, and the things that cost real
time to discover and would cost the same again.

State as of `851f76b`: 14 commits, 68 tests, clippy clean, ~6900 lines. Nothing
pushed; there is no remote.

---

## What it is

A native Twitch client in one window: browse who you follow, watch up to four of
them at once, each with its own chat. Rust + [GPUI](https://github.com/zed-industries/zed)
(Zed's UI framework), with streamlink as the byte source and libmpv doing decode
and A/V sync.

It replaces the useful half of
[streamlink-twitch-gui](https://github.com/streamlink/streamlink-twitch-gui),
whose real pain point was that watching a stream spawned three windows.

## Why the stack is what it is

The project was originally going to use [GPUIX](https://github.com/remorses/gpuix)
(React bindings over GPUI). That was dropped early, and the reasoning still
matters because it constrains everything else:

- GPUIX has no canvas element and its `<img>` takes a **disk path only**, so
  video would have required a custom native element inside a fork of GPUIX,
  which itself vendors a fork of Zed.
- Since video needs Rust regardless, React was only buying the browse screens.
- `gpui-component` (the Rust widget library) runs `cargo test --all` on
  `windows-latest` every PR; GPUIX only `require()`-loads a prebuilt binary and
  never renders. That decided it.

The other rejected option was embedding mpv as a **child HWND** (`--wid`). It is
cheaper — zero CPU for frames — but a child window always composites *above* the
parent, so nothing can ever overlap the video. No overlay controls, no rounded
corners, no fading UI. Everything you see on top of the video exists because we
did not take that path.

---

## Layout

```
crates/
  mpv-frames    libmpv loaded at runtime, software render to BGRA
  streamlink    supervises streamlink as a headless Twitch byte source
  twitch-chat   read-only chat over anonymous IRC
  twitch-api    device-code sign-in and followed streams
  emotes        Twitch/FFZ/BTTV/7TV resolution + disk image cache
  settings      persisted user settings
  nativetwitch  the app
```

Every crate except the last is free of UI types, deliberately — they are
testable without a window, and the video pipeline in particular was built
standalone before any GPUI existed.

App modules:

| file | role |
|---|---|
| `main.rs` | shell: `RootView`, pages, stream slots, navigation |
| `browse.rs` | the follows page |
| `watch.rs` | the grid of panes; `Slot` lives here |
| `layout.rs` | derives grid shape from window aspect (pure, tested) |
| `video_view.rs` | player element + overlay controls |
| `video.rs` | render thread; owns the mpv `Player` |
| `chat.rs` | chat pane, emote rendering |
| `settings_view.rs` | settings sheet |
| `follows.rs` | sign-in + follows polling worker |
| `theme.rs` | **all** colour, spacing and type tokens |

**Threading model, used consistently:** anything blocking runs on a plain
`std::thread` and reports through a `futures::channel::mpsc`, which the UI drains
in a `cx.spawn_in` pump that calls `cx.notify()`. There is no async runtime. If
you add a network feature, follow that shape rather than introducing tokio.

---

## Traps

These are the expensive ones. Most are invisible until runtime.

### Video

**`gpui::surface()` is macOS-only.** The zero-copy video path does not exist on
Windows. Frames go through `img()` + `RenderImage`, which is CPU-side BGRA. Zed
does the same thing for screen share on non-macOS; that is where the pattern
came from.

**The atlas leaks without `drop_image`.** `RenderImage::new` mints a fresh
monotonic `ImageId` per frame and GPUI uploads every distinct id into its sprite
atlas. `VideoView` double-buffers and drops the frame *before* last — never the
one on screen.

**BGRA bytes go into an `RgbaImage` container unswapped.** GPUI documents
`RenderImage` as BGRA regardless of the buffer type's name. This looks like a bug
and is not one.

**mpv's `bgr0` has no alpha.** The fourth byte is documented as "uninitialized
garbage". Not filling it with `0xFF` makes every frame render fully transparent.

**`mpv_render_context_render` blocks until the frame's display time**, up to
`video-timing-offset` (50 ms default). That wait is what keeps video timed to
audio, so it is wanted — but calling it on the UI thread stalls the whole GPUI
frame loop. It runs on `video.rs`'s thread for that reason. A benchmark that
reads exactly 16.66 ms is measuring this, not CPU.

**Never let mpv render more pixels than the source has.** CPU upscaling measured
at 117–196% of a core — the most expensive thing this pipeline can do. Render
size is clamped to the source resolution and the GPU stretches the last bit,
which is free. This was introduced *and* reintroduced once; do not undo it.

**Animated GIFs need an `ElementId`.** From gpui's `img.rs`:

```rust
if global_id.is_some() && data.frame_count() > 1 {
    window.request_animation_frame();
}
```

No id means no per-element frame state and nothing ever asks for the next frame.
**The id must identify the image, not the slot** — keyed on position, a 40-frame
GIF in one row and a 1-frame PNG in another share state and GPUI indexes the PNG
with the GIF's frame number, which panics. `examples/gif_animation.rs` renders
the same GIF with and without an id as a regression check.

### Performance

Measured on a Ryzen 9 5950X against live streams, release build. Cost tracks the
**ratio** between source and pane far more than pixel count:

| source → pane | CPU (one core) |
|---|---|
| 1080p → 960×540 (exact half) | 35% |
| 1080p → 1920×1080 (1:1) | 79% |
| 1080p → 1280×720 (arbitrary) | **100%** |
| 720p → 1920×1080 (upscale) | 117–196% |

An arbitrary downscale costs *more* than native despite fewer pixels. This is why
`streamlink/quality.rs` prefers 1:1, then exact fractions, and never upscales.

**Hardware decode makes it worse.** `hwdec=auto-copy` lost every test — 13.8%→18.4%
of a core at 720p and 60→55 fps; 79.4%→92.1% at 1080p. The GPU→CPU readback costs
more than the decode saves. Do not "optimise" by enabling it.

**Always benchmark `--release`.** Debug builds are several times slower at the
per-frame format conversion.

### Networking

**rustls needs an explicit crypto provider.** It only auto-selects when exactly
one provider feature is enabled, and Cargo unifies features across the graph —
the app links rustls through gpui's tree too, so both `ring` and `aws_lc_rs` end
up on and rustls panics. `twitch-chat` pins `ring` and installs it explicitly.
This only reproduces in the real binary, never in an isolated example.

**rustls buffers plaintext.** Without an explicit `flush()`, the IRC handshake
sits unsent while we block on read and the server waits for a NICK that never
arrives.

**ureq must not treat status as error.** Twitch's device flow answers HTTP 400
with `{"message":"authorization_pending"}` while waiting for the user to type the
code. Discarding the body on error status made the *normal* case fatal and sign-in
gave up on the first poll. Agents are built with `.http_status_as_error(false)`.

**Twitch emote tag ranges are character indices, not bytes.** Slicing a Rust
`&str` with them is wrong the moment a message contains non-ASCII, which on Twitch
is constantly. `emotes/tokenize.rs` has a test for exactly this.

**streamlink's ad filtering only works in its own HLS pipeline.** Resolving a URL
with `--stream-url` and playing it elsewhere brings the ads back. Hence
`--player-external-http`.

**`--twitch-supported-codecs h264,h265,av1` is required** or Twitch's higher tiers
(1440p, 4K) are silently absent from the quality list — they ride on HEVC/AV1 and
streamlink filters to h264 by default.

### The two Twitch tokens

Unrelated credentials, easy to confuse, documented in `settings::Credentials`:

- **Client ID** — a public app id from dev.twitch.tv. Gets the follows list.
  Client Type must be **Public**; no secret, because sign-in uses device code
  flow and nothing secret can live in a desktop binary.
- **auth-token** — the twitch.tv **cookie**. Gets Prime/Turbo ad suppression and
  sub-only qualities via streamlink. A **full account credential**, stored in
  plain text. That is a documented tradeoff, not an oversight.

Neither can do the other's job. Refresh tokens are **single-use** — persist the
new one immediately or the next launch is locked out.

### GPUI / gpui-component

**`gpui-component 0.5.1` differs from its main-branch docs.** Read the vendored
source in `~/.cargo/registry/src/*/gpui-component-0.5.1/` rather than the online
guide. Known differences: masking is `InputState::masked(bool)` not
`Input::content_type`; `Button` variants need `ButtonVariants` in scope;
`selected_index(cx)` takes the app context.

**`gpui_component::init(cx)` must run before any widget**, and `Root::new` must
wrap the window's first view or overlays have nowhere to render.

**`group_hover` breaks under mouse capture.** Deriving visibility from it at
paint time made the volume slider's own control bar vanish the moment you pressed
it. Hover is explicit state (`on_hover`) now. Do not go back.

**Dependencies are plain crates.io versions** — `gpui 0.2.2`, `gpui-component 0.5.1`
— reproducible from `Cargo.lock`. An earlier plan called for pinning a git rev;
that turned out to be unnecessary.

---

## Design system

`theme.rs` is the single source of truth. **There are no ad-hoc colour, spacing or
type values anywhere in the UI, and it should stay that way** — the audit that
found 15 spacing values and 19 uses of one text size is what made the UI read as
unconsidered.

- **Colour** — quiet, because this is a window left open for hours. Player
  background is pure black; any lift shows as a grey halo around letterboxed
  video.
- **Spacing** — named by role (`PAGE_PAD`, `PANEL_PAD`, `CONTROL_PAD_*`,
  `GAP_TIGHT`, `GAP`, `GAP_SECTION`, `PANE_GAP`, `ROW_PAD_*`), not by size.
- **Type** — five roles (`TEXT_TITLE/BODY/LABEL/META/MICRO`). Label and meta share
  a size but differ in weight, so a thing you can click never looks like a thing
  you can only read. `weight_shout()` (bold) is reserved for the live badge.

Audit commands, worth re-running after UI work:

```bash
grep -ohE "\.(p|px|py|gap|gap_x|gap_y)_[0-9p]+\(\)" crates/nativetwitch/src/*.rs | sort | uniq -c
grep -ohE "\.text_(xs|sm|base|lg|xl)\(\)|FontWeight::[A-Z_]+" crates/nativetwitch/src/*.rs | sort | uniq -c
```

Both should return nothing outside `theme.rs`.

---

## Working on it

```bash
cargo build --release -p nativetwitch     # always release for anything perf-related
cargo test --workspace
cargo clippy --workspace --all-targets
./run.cmd outerheaven                     # or several channels
```

**Kill the app before rebuilding.** Windows locks the running exe and the link
step fails with "Access is denied".

**`cargo build | tail` reports `tail`'s exit code, not cargo's.** Use
`set -o pipefail` and `${PIPESTATUS[0]}`, or a bare build. This produced at least
one false "build OK".

**Large multi-file edits: write a Python patch script to the scratchpad and run
it**, rather than shell heredocs. Quoting fights you otherwise. Make the script
idempotent (treat an already-applied replacement as success) so a partial failure
can be re-run safely.

**Verify visually.** Much of this was found by screenshotting the running app with
PowerShell + `System.Drawing`, then reading the PNG. The pattern:

1. Poll for `MainWindowHandle`, `SetWindowPos` to topmost
2. `SetCursorPos` to reveal hover-only UI
3. `CopyFromScreen` into a bitmap, save, then `Read` the PNG

`GetWindowRect` includes invisible resize borders — trim ~8px. `SetForegroundWindow`
is often refused by Windows; use topmost instead. This loop caught the pill/chat
overlap, the chat-off-screen bug, and the channel-order verification.

**Verify pixel correctness by dumping PNGs.** `mpv-frames`'s `dump_frames` example
writes frames to disk with stats (per-channel means, alpha minimum, non-black
percentage). A red Superman "S" on a blue suit is how BGRA vs RGBA got confirmed.

**Clean up processes.** streamlink is a child of the app and dies with it on a
graceful close, but a hard kill orphans it. `taskkill //F //IM streamlink.exe`.

---

## Open items

Roughly in the order I would take them.

1. **Motion and feedback pass.** The user picked spacing then typography from a
   three-way split; this is the remaining one. Hover states are abrupt, panes
   appear instantly, toasts do not animate in.
2. **Quality does not re-pick on resize.** It is chosen when a channel opens,
   using the pane size at that moment. Maximising afterwards grows the render
   buffer but not the stream quality.
3. **No sign-out**, and no way to clear a bad token except editing the field.
4. **Animated WebP** (7TV, some BTTV) may render as stills. Twitch's animated
   emotes are GIF and animate correctly.
5. **Follows poll every 60 s** with no manual refresh.
6. **Console window** opens alongside the app. Deliberate while iterating —
   `#![windows_subsystem = "windows"]` removes it but also hides stderr.
7. **Orphaned streamlink on a hard crash.** A Windows job object would close it.
8. **Never tested on a vertical monitor.** The layout derives portrait grids and
   stacks chat below video, and the logic is unit-tested, but nobody has seen it.

## Things not to redo

- Do not enable hardware decode "for performance".
- Do not let mpv upscale.
- Do not derive control visibility from `group_hover`.
- Do not key animated-image element ids on position.
- Do not use `--stream-url` to skip streamlink's pipeline.
- Do not add tokio; use a thread plus an mpsc pump.
