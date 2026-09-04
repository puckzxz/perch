# Handoff

For whoever picks this up next. `README.md` covers *using* it; this covers
*working on* it — the architecture, the traps, and the things that cost real
time to discover and would cost the same again.

Roughly 11,000 lines across seven crates. `cargo test --workspace`,
`cargo clippy --workspace --all-targets` and `cargo fmt --all --check` are all
expected to pass; if one does not, that is the change you are looking at, not
the baseline.

---

## What it is

A native Twitch client in one window: browse everyone you follow — live or
not — plus what is popular and what is on; search for a channel; and watch up to
four at once, each with its own chat. Rust + [GPUI](https://github.com/zed-industries/zed)
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
  twitch-chat   read-only chat over anonymous IRC, plus the history backfill
  twitch-api    device-code sign-in, follows, top streams, categories, search
  emotes        Twitch/FFZ/BTTV/7TV resolution + disk image cache
  settings      persisted user settings
  perch         the app
```

Every crate except the last is free of UI types, deliberately — they are
testable without a window, and the video pipeline in particular was built
standalone before any GPUI existed.

App modules:

| file | role |
|---|---|
| `main.rs` | shell: `RootView`, pages, stream slots, navigation |
| `browse.rs` | the picker page: following, popular, categories, search |
| `watch.rs` | the grid of panes; `Slot` lives here |
| `layout.rs` | derives grid shape from window aspect (pure, tested) |
| `video_view.rs` | player element + overlay controls |
| `video.rs` | render thread; owns the mpv `Player` |
| `chat.rs` | chat pane: rows, emotes, scrollback |
| `chat_text.rs` | what a word in a message is — link, mention or plain (pure, tested) |
| `settings_view.rs` | settings sheet |
| `twitch.rs` | the worker: sign-in, follows polling, browse requests |
| `keys.rs` | the keymap: actions, bindings, contexts, and the listing |
| `theme.rs` | **all** colour, spacing, type and motion tokens |
| `sidebar.rs` | the follows rail down the left of both pages |
| `palette.rs` | the command palette, and what it can run |
| `controls.rs` | the one button, and the variants it comes in |
| `widget_theme.rs` | hands `theme.rs` to `gpui-component`'s own palette |
| `assets.rs` | the icons `gpui-component` asks the host for |
| `motion.rs` | the four animation shapes, and the state one of them needs |
| `diagnostics.rs` | where stderr goes when there is no console |
| `clock.rs` | the system's short time format, for chat stamps |
| `cpu_log.rs` | what the CPU was doing, sampled every five seconds |

**Threading model, used consistently:** anything blocking runs on a plain
`std::thread` and reports through a `futures::channel::mpsc`, which the UI drains
in a `cx.spawn_in` pump that calls `cx.notify()`. There is no async runtime. If
you add a network feature, follow that shape rather than introducing tokio.

**One thread owns the Twitch session, and it has to.** Refresh tokens are
single-use, so two things refreshing at once would spend the same token twice
and lock the user out. Every Helix read therefore goes through `twitch.rs`, not
merely for tidiness. It takes requests on a `std::sync::mpsc` channel and waits
on `recv_timeout` against the next follows-poll deadline — the wait and the
mailbox are the same thing, so browsing never queues behind the timer. Dropping
the service drops the sender, which wakes the worker immediately rather than
after the poll interval.

---

## Traps

These are the expensive ones. Most are invisible until runtime.

### Video

**`gpui::surface()` is macOS-only.** The zero-copy video path does not exist on
Windows. Frames go through `img()` + `RenderImage`, which is CPU-side BGRA. Zed
does the same thing for screen share on non-macOS; that is where the pattern
came from.

**One `ImageId` per stream, not per frame.** `RenderImage::new` takes a fresh
monotonic id from a global counter on every call, and GPUI keys its sprite atlas
on that id — so a source that mints one `RenderImage` per frame is asking it to
build and destroy a GPU texture sixty times a second. On a maximised 1440p pane
that is a 14.7 MB `CreateTexture2D` plus a shader resource view, and a `Release`
of the pair from the frame before, every frame. `video.rs` mints one id for the
whole stream and stamps it onto every frame instead, and `video_view.rs` calls
`Window::update_image`, which overwrites the tile's pixels in place.
`RenderImage.id` is a public field, so the perch half needs no patch; the
`update` verb is what does, because `get_or_insert_with` is insert-once and
hands back a cached tile without ever consulting the builder.

Two things about that call are easy to get wrong. It is guarded on
`Arc::ptr_eq`, **not** on the id: `render` runs on every *window draw*, not on
every decoded frame — `impl Element for Entity<V>` has no cache key, so chat
traffic and the control fade arrive there too — and every frame of a stream now
shares one id, so an id comparison would skip every real frame and freeze the
picture. And `update_image` returns `false` when the tile is missing or has
changed size (a pane resize), having removed the key on the way out, so the
`img` that follows inserts it the ordinary way.

**GPUI still does not refcount atlas tiles against the `Arc<RenderImage>`.**
`Window::drop_image` is the only thing that calls `sprite_atlas.remove`, so the
one tile a `VideoView` owns stays resident for the life of the window unless the
view hands it back — and the view dies on every ordinary action, including each
quality change. `Drop` cannot do it; it has no `Window`. `cx.on_release_in` can,
and the `Subscription` it returns has to be kept in a field or the hook is
dropped immediately. `ChatView` does the same for its emote cache.

**And it leaks a third time inside GPUI itself, where `drop_image` cannot
reach.** This is the expensive one: 43.7 GB of committed memory in five hours,
54% of the machine's entire commit charge. `DirectXAtlas::push_texture` rounds a
new atlas texture up to at least 1024x1024 in *each dimension separately*, so a
1280x936 frame gets a 1280x1024 texture — and etagere rounds the 936-tall shelf
to 960, leaving 64 rows spare. `DirectXAtlas::allocate` then scans *every*
existing texture for room, newest first, so the next chat emote lands in that
strip. The texture's `live_atlas_keys` never falls back to zero, `remove` never
frees it, and because `RenderImage::new` mints a fresh id per frame, one 5 MiB
texture is pinned for every emote inserted.

The tell is that it depends on the *source resolution*, which is why it looks
like magic until you measure it. Watching a 1098p stream and a 936p stream side
by side, the 936p one had leaked 8,731 textures and the 1098p one exactly zero:
1098 is over 1024, so its shelf fills the texture and nothing else can be packed
in. Anything from roughly 90p to 960p leaks; 961p and up does not.

The fix is in `vendor/gpui` — see the comment on `[patch.crates-io]` in the root
`Cargo.toml`. Textures created oversized are marked `dedicated` and skipped in
that scan. `scripts/verify-vendor.sh` proves the vendored tree is upstream plus
exactly that patch, and CI runs it before anything slower.

**Fixing that exposes a second leak underneath it, in the same file.** `remove`
never returned a tile's space to the shelf allocator, so a *shared* atlas was
write-once: `allocate` took space, nothing gave it back, and the texture could
only be discarded whole - which happens only once every key in it has gone.
While the bug above was live this was invisible, because frames were being
pinned into oversized textures instead of shared ones. Fix the first and the
traffic moves to shared atlases, where it shows up immediately: two panes went
from 491 live 4 MiB atlases to 891 in four minutes, 3.6 GB and climbing. Any
frame that fits inside 1024x1024 hits this - a 2x2 layout, or any pane smaller
than the default atlas.

That one is upstream's own fix (Zed PR #58874, "gpui: Free atlas tile space when
removing tiles"), so the vendored copy carries their version of it. Backporting
it to Metal needed one extra change: Metal's `remove` looked the key up with
`get` and erased it only when the texture hit zero live keys, so dropping the
same image twice decremented the count twice and could free a texture out from
under a live tile. `VideoView`'s release hook used to do exactly that, back when it
held `current` and `previous` and both could be the same `Arc`. Metal now takes
the key out up front like the other two backends; the hook that made it
reachable is gone anyway, since one id per stream means one frame to release.

**Task Manager lies about all of this.** It shows the working set, and these
textures are committed pages the driver mostly never touches, so 44 GB of commit
charge showed up as 5.6 GB "memory" in the process list. Read
`PrivateMemorySize64` — or walk the address space with `VirtualQueryEx` and
group by allocation size, which is what identified the 5,242,880-byte
(1280x1024 BGRA) blocks and turned a guess into a count.

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

**The render thread may not call libmpv synchronously.** render.h requires that
the thread calling `mpv_render_context_render` "does not call libmpv API
functions other than the mpv_render_* functions, except APIs which are declared
as safe", and client.h declares only the asynchronous ones safe:
`mpv_observe_property`, `mpv_set_property_async`, `mpv_command_async`, and
`mpv_wait_event` with a zero timeout. A synchronous get or set from that thread
can deadlock against a core that is waiting on a render; mpv breaks it with a
timeout, drops the frame, and logs "mpv_render_context_render() not being
called or stuck". `video.rs` therefore reads `width`, `height`, `hwdec-current`
and `decoder-frame-drop-count` by observing them and draining `poll_events`
each pass, and `Player::set_paused`, `set_volume` and `seek_to_live` queue
rather than apply. `Player::property` is still there for threads that never
render, and its doc says so.

**`img` sizes its own box from the frame, and the frame is sized from the box.**
gpui's `img` writes the image's aspect ratio onto its style unconditionally,
and taffy honours that ratio whenever a percentage height fails to resolve —
which, for a block child measured inside a flex column, it does. So the player
was as tall as its *width divided by the last frame's aspect*; the probe then
asked mpv for a frame that shape; that frame's aspect set the same height
again. A stacked pane after a rail toggle sat at four fifths of its box with
black under it, for the life of the process, and looked perfectly right after a
restart because the first frames are 1280×720. The pane and the player's root
are flex containers now, so the image is a stretched flex item whose size is
the pane's and nothing else. Do not put the frame back in a block; and if a
video ever renders smaller than its box, suspect this before anything in mpv.

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

**Hardware decode is on, and it was measured twice with opposite results.**
An early benchmark had `hwdec=auto-copy` losing — 13.8%→18.4% of a core at 720p,
79.4%→92.1% at 1080p — on the argument that the GPU→CPU readback costs more than
the decode saves. A later pair of 3.5-minute steady-state runs on a 936p60
stream, read from `cpu_log`, had it *winning* by a third: 146.6% → 97.1% of a
core, with the driver threads falling furthest. The comment on
`video::hwdec_requested` carries the numbers. It is quality-neutral — the same
bitstream through a fixed-function decoder is the same frames — which is why it
is worth taking, and `auto-copy` falls back to software on its own where a
codec or driver cannot do it. The lesson is the method: a short A/B is not a
measurement here; use the CSV over alternating multi-minute runs.
`PERCH_HWDEC=0` turns it off on a machine whose decoder misbehaves.

To repeat a measurement like it: `PERCH_CPU_LOG=1 run.cmd <channel>`, leave it
for a few minutes, then again with the setting under test flipped, and compare
steady-state rows of the CSV rather than a moment of each.

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

**Nothing that can fail may sit between the token swap and the save.**
`twitch_api::refresh` used to call `current_user()` before returning, which put
a second network request *after* the point of no return: a dropped packet during
it returned `Err`, the caller threw away the pair Twitch had just issued, and the
old refresh token was already dead. Two seconds of bad wifi signed the user out
permanently. It now takes the id and login as arguments — on a refresh they are
already on disk, so there is nothing to ask Twitch for. Only first-time sign-in
looks the user up, where a failure costs nothing because there is no predecessor
token to lose.

**`slow_down` is an instruction, not a synonym for "pending".** RFC 8628 §3.5
requires adding five seconds to the poll interval each time the device flow
returns it. Folding it into `Pending` left the client polling at exactly the rate
Twitch had asked it to reduce, until the code expired and a correctly typed one
still reported "the sign-in code expired". A sign-in window also runs for
minutes, so `Error::Network` during one is retried until the deadline rather
than tearing the worker down.

**Both followed endpoints paginate.** `/streams/followed` caps at 100 like
`/channels/followed` does. Fetching one page of it silently truncated the live
list, and because `on_streams` replaces `known_live` wholesale, a channel
hovering around rank 100 dropped out and returned on alternate polls — firing a
went-live toast every time it came back.

**Two threads write `settings.json`.** The sign-in worker persists OAuth tokens
as it gets them; the UI holds a snapshot of `Settings` taken at launch and
writes it back whenever a preference changes. Saving that snapshot wholesale put
`oauth` back to whatever it was at startup — erasing a fresh sign-in outright,
and, once a refresh had happened, restoring a refresh token Twitch had already
spent. That is why sign-in never survived a restart: every launch prompted for a
new device code.

The UI now saves through `Settings::save_preferences`, which re-reads the file
and keeps the sign-in it finds. It is written as "everything I own wins, and the
one field somebody else owns is named", so adding a UI field needs no change
there. `save_forgetting_sign_in` is the deliberate exception, for when the
client id changes and the tokens stop meaning anything. Both have tests, and the
first one fails if you swap it back to a plain `save`.

Re-reading was half the answer. Both writers are read-modify-write, and nothing
ordered them: a token save landing between the UI's read and its write was
overwritten by the tokens the UI had just read, which is the spent-refresh-token
lock-out again by another route, and both used the same `.json.part` temporary
name. Every write now holds a process-wide lock in the `settings` crate across
its read and its write, and the worker saves through `Settings::save_sign_in`
rather than through its own load-and-save. `two_writers_never_lose_each_others_fields`
hammers the two from two threads; without the lock it fails within a few hundred
rounds.

### GPUI / gpui-component

**`gpui-component 0.5.1` differs from its main-branch docs.** Read the vendored
source in `~/.cargo/registry/src/*/gpui-component-0.5.1/` rather than the online
guide. Known differences: masking is `InputState::masked(bool)` not
`Input::content_type`; `Button` variants need `ButtonVariants` in scope;
`selected_index(cx)` takes the app context.

**An overlay does not block input just by covering something.** GPUI hit-tests
into a *flat, z-ordered list* and every overlapping element is collected, not
just the topmost — `Frame::hit_test` pushes every hitbox containing the pointer
and only stops early on `HitboxBehavior::BlockMouse` (window.rs:775-796). Click
handling is synthesised from a down/up pair and never calls `stop_propagation`
on success (div.rs:2136-2245), so two stacked elements that both have `on_click`
both fire. That is why clicking "stop all" also opened whichever browse card was
behind it, and why a click inside the settings panel reached the grid.

The fix is one flag, not a handler: `.occlude()` (`HitboxBehavior::BlockMouse`)
or `.block_mouse_except_scroll()`, both on `InteractiveElement` (div.rs:998-1012),
so they work on a bare `Div` with no `.id()`. Because hit testing is flat and
knows nothing about the element tree, an occluder blocks its own *ancestors*
too — which is exactly what makes the modal pattern work.

Two things worth keeping in mind:

- **Occlude the smallest thing that is actually opaque.** Put it on a container
  and you block its whole bounding box, including empty space. Toast cards are
  occluded individually for that reason. The now-playing bar needs none of it:
  it is docked at the bottom of the browse page rather than floating over the
  grid, so there is nothing underneath it to block. It used to be a strip of
  220px thumbnails in the bottom-right corner, which covered two cards at
  1000px and would have laid 900px of tiles over the bottom row with four
  streams open.
- **`block_mouse_except_scroll` for overlays on the browse page**, so the wheel
  still reaches the grid underneath; plain `occlude` for the modal, where the
  page behind should not scroll either.

`browse.rs`'s `cx.stop_propagation()` on "+ add" is *not* the same pattern and
must stay as it is: that button is a descendant of the card, not an overlay, and
occluding it would kill the card's own hover and the group-hover that reveals it.

Hover probes are immune to all of this: `gpui::canvas` inserts no hitbox and
reads `window.mouse_position()` directly, so the video and pane probes keep
working through any occluder.

**`gpui_component::init(cx)` must run before any widget**, and `Root::new` must
wrap the window's first view or overlays have nowhere to render.

**Neither `group_hover` nor `on_hover` means "the pointer is over this."** Both
resolve through the same expression:

```rust
let is_hovered = has_mouse_down.borrow().is_none()
    && !cx.has_active_drag()
    && hitbox.is_hovered(window);
```

`cx.has_active_drag()` is *window-wide*, and gpui-component's `Slider` drags via
`on_drag` — so touching the volume slider makes every hover listener in the
window report false, including the one whose control bar holds that slider. And
because `on_hover` only fires on a `MouseMoveEvent`, leaving the window is
invisible to it: the last move it saw was inside, so the controls stayed up.

Hover is therefore **measured, not reported**: a `canvas` probe already runs each
frame for render sizing, and it now also asks
`window.is_window_hovered() && bounds.contains(&window.mouse_position())`. The
`on_hover` listeners that remain exist only to wake a repaint — a paused stream
sends no frames, so without them nothing would ask the probe to run again. Their
*value* is ignored on purpose. Do not wire it back up.

**Dependencies are plain crates.io versions** — `gpui 0.2.2`, `gpui-component 0.5.1`
— reproducible from `Cargo.lock`. An earlier plan called for pinning a git rev;
that turned out to be unnecessary.

### Windows packaging

**The release build has no console**, so `eprintln!` and panic messages go to a
handle that leads nowhere. `diagnostics::capture_stderr` points the process's
stderr at `%LOCALAPPDATA%\perch\perch.log` before anything can
write, and keeps the previous run as `.log.old`. It works by `SetStdHandle`
rather than by a logging facade because Windows resolves that handle on *every
write* — so it catches the library crates and panic output too, with no change
anywhere else. That was verified with a throwaway before the code was written;
if you ever doubt it, verify it again rather than assuming.

Debug builds keep their console on purpose (`#![cfg_attr(not(debug_assertions),
windows_subsystem = "windows")]`), which is also the only place `--help` is
readable.

**`perch.cpu.csv`, beside it, is for the CPU questions — when asked for.**
With `PERCH_CPU_LOG=1` in the environment, the app samples itself every five
seconds into `%LOCALAPPDATA%\perch\perch.cpu.csv`. Unlike the stderr log it is
appended to across runs rather than rotated per run — a restart in the middle
of a day is ordinary, and the morning should not land in a file that the next
restart throws away — and it rotates to a timestamped `perch.cpu.<stamp>.csv`
only on passing 32 MiB or on its columns changing. Roughly 0.6 MB for a working
day. The reason it exists is that "perch used 12%" is not something anyone can
act on, and the spike worth explaining is never happening while you are looking
at it; it is what the hardware-decode comparison was read from. It was on by
default for a while and is opt-in now: a shipped app should not carry a Win32
sampler and a file in AppData for people who will never read it. With it off,
every hook it has in `render` and the video thread is one relaxed atomic access.
Everything Win32 in it is behind `cfg(windows)`, constants and helpers
included: the macOS leg runs clippy with `-D warnings`, and a constant only the
Windows sampler reads is dead code there.

Three columns do most of the work. `by_thread` attributes the time by *thread
name*, which Windows keeps for anything Rust or libmpv named — so `main` is the
UI thread, `mpv-render` is `video.rs`, `worker`/`demux`/`vo`/`core` are libmpv,
`image-cache` is downloads. `renders_per_s` separates work from spinning: a
static page repainting sixty times a second is a bug and looks identical to a
busy one in a CPU number. And `child_pct` is streamlink and its python, because
Task Manager folds a parent's children into its row — so a stream left playing
as a browse thumbnail reads as perch using CPU when none of it is perch's.

What it said the first time it ran, which is the baseline to compare against:

| state | CPU (of one core) | renders/s |
|---|---|---|
| follows page, nothing playing | 0.3-2.5% | **0** |
| follows page, one stream still playing | ~50% | 60 |
| watch page, one stream | ~110% | 120 |

The first row is the important one: gpui does not repaint a page that has not
changed, so an idle perch is genuinely idle. Every bit of the rest is the
*stream*, and going back to the follows page does not stop one — the pane
becomes a muted thumbnail and mpv keeps decoding at full source resolution,
because the render size follows the element but the decode does not.

**A windowed app still gets console windows from its children.** streamlink is a
console-subsystem program, so every one we spawn came with its own console — and
since the app itself no longer has one, those were the only console windows a
user ever saw. `streamlink::command` sets `CREATE_NO_WINDOW`. Windows still
pairs a `conhost.exe` with each child; check for *visible windows*, not for the
absence of conhost, or you will conclude the fix did not work.

**The icon is an embedded resource**, stamped on by `build.rs` via
`winresource`, because gpui 0.2.2 has no window-icon API and Windows takes the
taskbar and titlebar icons from the executable anyway. It needs `rc.exe` from
the Windows SDK; a build without one warns and produces an icon-less binary
rather than failing. `assets/make-icon.ps1` regenerates the `.ico` — entries up
to 128px are DIBs and 256 is a PNG, because GDI+ cannot read a PNG-payload entry
back, so a PNG-only file is one you cannot open to check.

### Motion

**Not every cached image is immutable, and GPUI decodes one per path.** A
channel's preview lives at a *fixed* URL whose picture Twitch replaces every few
minutes, so caching it by URL forever pins whatever was there the first time you
looked — the browse page showed day-old thumbnails for exactly this reason.

Two things had to be true to fix it, and the second is the non-obvious one:

- Previews go through `ImageCache::get_or_request_fresh`, refetching past a
  `max_age` and living in an `images/live/` subdirectory that is emptied at
  startup, so nothing survives into the next run. That lookup deliberately does
  *not* consult the on-disk index the permanent path uses — promoting a file
  from a previous session is the bug, not the cure.
- Each refresh writes a **new filename**. GPUI caches a decoded image against
  its path, so replacing the bytes underneath leaves the stale picture on
  screen. The old file is deleted once the replacement is indexed, so a
  refreshing image costs one file rather than one per refresh.

Emotes and box art still use `get_or_request` and are still kept forever on
*disk*, which is right: they never change at their address.

**Deleting the file is only half of a refresh.** GPUI decodes an image once per
*path*, so a new filename means a new `RenderImage`, a new entry in
`App::loading_assets` and a new atlas tile — and nothing removes any of the
three on its own. The grid is not virtualised and painting is not culled, so an
idle browse page minted all three for every stream in the list every five
minutes and kept every generation it had ever drawn. `ImageCache` now records
each superseded path, `browse::release_retired_previews` releases what was
decoded from it, and `RootView::render` drains that list as its first statement.
The ordering is the whole trick: a path is recorded only once its replacement is
in the ready map, and the drain runs before any card is built, so nothing in the
frame can still be asking for what is being released. Get that backwards and the
card does not flicker — it goes permanently blank, because GPUI memoises the
failed re-read for the life of the process.

**Chat emotes are cached per pane, not per process.** A bare `img(path)` falls
through to `App::loading_assets`, which has no eviction of any kind: every emote
a pane ever drew — and every *frame* of an animated one, each its own atlas tile
— stayed resident forever, long after the channel was closed. `ChatView` owns a
`RetainAllImageCache` and clears it in `on_release_in`, so closing a pane or
switching channels gives it all back. That bounds emotes for panes that are
*open*, which is the growth that compounds over an evening; it does not bound a
single channel left open for hours, whose emote set saturates on its own.

**A `list`'s scrollbar extent comes from *measured* items only.** gpui measures
rows as it draws them, so in a live-appending list the ones you have not looked
at contribute nothing to `items.summary().height` — which is what
`max_offset_for_scrollbar` is derived from. The visible symptom is a thumb that
changes size as you scroll, because scrolling is what does the measuring.

`ListState::measure_all()` is the documented remedy and chat uses it, but it is
only half a fix: the pass runs once. Rows spliced in afterwards stay unmeasured
until something draws them — and a change in the list's *width* throws away
every measurement it has, because `List::prepaint` rebuilds the whole tree as
`Unmeasured` without re-running the pass. In a window that is one to four
resizable panes, that resize case is the bigger source of drift. The thumb's
*position* is correct throughout; only its size is short. A complete fix means
not using gpui's scrollbar geometry, which is a bigger job than it looks.

Re-arming the pass *is* possible — `ListState` is `Clone` over an
`Rc<RefCell<_>>` and `measure_all` mutates the shared inner, so
`list.clone().measure_all()` does it without touching the scroll pin. It is
deliberately not done: `layout_all_items` rebuilds the whole `SumTree` on every
pass, and paying that on a cadence to stop a thumb drifting is the worse trade.

**Do not reach for `reset()` to jump to the bottom.** It clears the scroll pin,
which is the thing you want, and it also splices every row as unmeasured, so
the next prepaint lays out all thousand of them — emote images included — in one
frame. It sets `reset = true` as well, so the scroll after a jump goes nowhere.
It did buy one thing on the way — re-arming `measure_all`, so the extent came
out correct — which is why the jump is now cheap and the thumb slightly less
accurate. What `follow_live` uses instead is
`set_offset_from_scrollbar(point(px(0.), px(f32::MAX)))`: any offset past the
end clamps to `scroll_max`, and a bottom-aligned list sitting exactly there sets
`logical_scroll_top = None` and does nothing else. `scroll_to`/`scroll_by` are
not alternatives — they *set* a pin, so they land at the bottom and are then
left behind by the next message. Scrolling with the wheel re-arms auto-follow on
its own, but only on reaching the very bottom, which in a fast channel is a long
way down.

**GPUI has no transitions.** `.hover()` swaps styles instantly and there is no
way to interpolate between them. Every animation in the app therefore goes
through `with_animation`, and `motion.rs` exists because that primitive only
does one thing: run forward from zero.

**Animation state is keyed on the element id**, and an id GPUI has already seen
comes back as a *finished* animation holding its last value. That is why a
two-way fade has to mint a new id on every flip (`Fade`), and why a one-shot
arrival needs no state at all — mounting the element is the whole trigger.

**Element ids are namespaced by every ancestor that has one**, plus an implicit
`ElementId::View(entity_id)` per entity. So `"controls"` is unique inside a
`VideoView` even with four of them on screen, but anything rendered by
`RootView` — every pane, every toast — has to carry its own discriminator.
`Fade::apply` composes the caller's id with the flip count via
`ElementId::NamedChild` rather than replacing it, so both survive.

**A repeating animation never stops asking for frames.** `motion::waiting` is
only ever attached to a state that ends. "Offline" and "failed" deliberately sit
still: a pulsing error is a permanent 60 fps repaint, and it reads as progress
when there is none.

**`on_hover` fires only when its value *changes*.** Since those listeners are
now just repaint triggers, that matters in one place: GPUI's idea of hovered and
ours must not drift. Pane element ids are keyed on the **channel**, never the
index, because closing a pane reindexes the rest and a position-keyed survivor
inherits the closed pane's `hover_state = true` — no change, so no repaint, so
its header stays hidden until the pointer leaves the pane entirely. See
`watch::pane_id`.

---

## Design system

`theme.rs` is the single source of truth. **There are no ad-hoc colour, spacing or
type values anywhere in the UI, and it should stay that way** — the audit that
found 15 spacing values and 19 uses of one text size is what made the UI read as
unconsidered.

- **Colour** — quiet, because this is a window left open for hours. Player
  background is pure black; any lift shows as a grey halo around letterboxed
  video.
- **Volume** — one number per channel plus a global default, and the default
  follows the last level you *chose*, so an unfamiliar channel opens near where
  you have been listening. Muting is the exception and never becomes the
  default: mute is something you do to one stream, usually to hear another, and
  a channel that opens silent reads as broken. That is also why the stored value
  is an `Option<u8>` — `Some(0)` (deliberately muted) and `None` (never opened)
  must not collapse into each other. Keys go through `settings::channel_key`,
  because the app does not agree with itself about case: Helix says `forsen`,
  the command line says whatever was typed.
- **Spacing** — named by role (`PAGE_PAD`, `PANEL_PAD`, `CONTROL_PAD_*`,
  `GAP_TIGHT`, `GAP`, `GAP_SECTION`, `PANE_GAP`, `ROW_PAD_*`), not by size.
- **Controls come from `controls.rs`.** There were ten hand-rolled buttons in
  six shapes — the tokens were shared the whole time and the component was not,
  which is the same drift one file down. A control that needs a shape not on
  that list is a new variant there, not an eleventh `div`. The same file owns
  the two things that are not buttons but were drawn by hand in three places
  each: `live_dot`, and `tag`, the passive `muted` / `paused` word in a pane
  header.
- **Layout reads a `layout::Body`, never the viewport.** The body is the window
  less the rail when it is open, and it is a type made in one place —
  `RootView::body` — so nothing laying out a page can be handed the viewport by
  mistake. The watch grid was, once: with the rail out, every stacked pane
  carried a 66px black band under its picture, because its 16:9 box was derived
  from a cell wider than the one it was drawn in. A test in `layout.rs` keeps
  the number.
- **Sizes the user dragged are settings, not view state.** `chat_width` and
  `video_share` live in `settings.json`, and the drag writes them on mouse *up*
  rather than on every move — a drag is hundreds of events and each save is a
  read-modify-write of the whole file. `Ctrl+0` puts both back to their
  defaults, `video_share: 0.0` meaning "derive it", which is what the layout did
  before anybody dragged anything.
- **The widget library reads the same tokens**, via `widget_theme::apply` after
  `gpui_component::init`. Without it `init` seeds its palette from
  `cx.window_appearance()` — the *operating system's* light/dark setting — so
  every input, dropdown, button and slider followed the OS while everything
  around them stayed dark.
- **Type** — four roles (`TEXT_TITLE/BODY/LABEL/META`). Label and meta share
  a size but differ in weight, so a thing you can click never looks like a thing
  you can only read. There were five: `TEXT_MICRO` and `weight_shout()` existed
  for the `LIVE` badge alone and went when it did.
- **Contrast is measured, not judged.** `MIN_CONTRAST` is AA for the sizes this
  app uses, `theme::contrast` computes it, and `every_text_tier_is_legible`
  holds every text token to it on every surface it lands on. `text_dim` used to
  fail that on two of three surfaces while carrying game names, offline channel
  names and the whole of the settings help. `readable()` bisects a username's
  lightness until it *measures* legible rather than stopping at a fixed one —
  the flat floor it replaced left pure blue at 2.7:1.
- **Motion** — three durations named by job (`MOTION_HOVER`, `MOTION_ENTER`,
  `MOTION_VIDEO`) plus the waiting pulse, and two easings (`ease_fade` for
  two-way changes, `ease_enter` for arrivals). Motion says *that something
  changed*; anything long enough to wait for is too long.

Audit commands, worth re-running after UI work:

```bash
grep -ohE "\.(p|px|py|gap|gap_x|gap_y)_[0-9p]+\(\)" crates/perch/src/*.rs | sort | uniq -c
grep -ohE "\.text_(xs|sm|base|lg|xl)\(\)|FontWeight::[A-Z_]+" crates/perch/src/*.rs | sort | uniq -c
grep -nE "Duration::from_(millis|secs)" crates/perch/src/*.rs
```

The first two should return nothing outside `theme.rs`. The third will show
genuine timings — the follows poll, the toast lifetime, an mpv frame wait — but
no *animation* duration should appear outside `theme.rs`.

### Where controls live

**Nothing static is ever drawn on the video.** Static information on a moving
picture is exactly what you end up staring past for three hours, so it lives in
a header above chat instead — chat is already a panel, so it costs nothing
there. The split:

- **Chat header**, always visible, one per pane: a live dot *when the pane is
  actually showing a picture*, the channel name — which opens twitch.tv, the
  way out of a chat that is read-only by design — viewer count, uptime, and the
  pane's close control. With more than one pane its bottom border marks the one
  the keyboard is talking to.
- **Over the video**, hover-revealed only: the playback bar (pause, mute,
  volume, quality), and in the top-left the "← follows" pill plus, when the rail
  is folded away, the control that brings it back. Point at the video and they
  come up; look away and the picture is all that is left.

The back pill follows the *panes'* hover, not the window's, so resting the
pointer in chat does not keep it on screen. An earlier arrangement put the
channel name and close over the video at the pane's top-right; that is gone, and
with it the collision that let one click both close a pane and navigate away.

Viewer count and uptime come from the follows poll — the same `LiveStream` the
browse cards use, looked up by login at render time rather than copied onto the
`Slot`, so there is one source and it cannot go stale. A channel you opened by
name but do not follow has no entry, and the header correctly shows neither.
Filling that gap needs a `GET /helix/streams?user_login=…` per channel.

**There is no chatter count to be had.** The old
`tmi.twitch.tv/group/user/<channel>/chatters` was shut down on 3 April 2023 and
returns 404. Its Helix replacement, `GET /chat/chatters`, requires
`moderator:read:chatters` *and* that the token's user is the broadcaster or one
of their moderators — 403 otherwise. IRC `NAMES` via `twitch.tv/membership` still
responds, but silently stops listing above ~1000 users, so it returns nothing on
exactly the channels worth asking about. `viewer_count` from Get Streams (no
scope, app or user token) is the only public number. Do not go looking again.

How a pane divides itself depends on its shape, and the two cases are
deliberately opposites. Beside the video, chat gets a fixed width and the video
takes the rest. Below it, the **video** gets a box the shape of its stream and
**chat** takes the rest — a window is tall because you want more chat, not more
letterboxing. `layout::stacked_video_height` owns that. It is sized from the
stream's *aspect*, which `VideoStream` publishes once the first frame decodes,
and not from the frame's size: render size follows the pane, so a pane sized
from the frame would be a feedback loop, but a broadcast's shape does not
change with the window. 16:9 stands in until the stream has said, so most
streams never move; a 4:3 or a vertical one reflows once. The box is capped at
`VIDEO_SHARE_MAX` of the cell, so a vertical stream pillarboxes rather than
pushing chat off the bottom, and a test pins that the box can never fill the
cell it stacks in.

The seam between video and chat takes no room. The divider's grab strip is
absolutely positioned astride the boundary — half into the video, half into
chat — off a zero-height (or zero-width) element in the flow, and the stacked
chat pane has no padding above its header. Video ends, name begins, and the
header's own bottom rule is the only line between them. The six pixels of grid
background that used to sit there were, once the box matched the picture, the
only gap left, and it read as one.

Left-anchoring the pane controls put the first pane's close button underneath
the page navigation, and since neither called `cx.stop_propagation()` a single
click closed a pane *and* navigated away. Top-right is the only anchor that
clears the corner for every grid shape `layout.rs` can derive — with four
columns, the third pane's *left* edge also lands under a centred nav.

### Browsing

Follows are **two lists that never merge**. `LiveStream` means *is live*, and
three things read it that way — the went-live toasts, the card's viewer count,
and the chat header's live dot — so an offline channel sitting in that vec would
be wrong in all three at once. Offline follows are `FollowedChannel`s, and they are
drawn as names rather than cards: a card is mostly a picture, and an offline
channel has none worth showing — a thumbnail stale by hours, or a profile
picture that costs another request per refresh and says nothing. Names also
pack, so a hundred follows is five rows instead of a wall of grey rectangles.
Clicking one still opens it: the video says offline, but the *chat* connects
either way, which is the reason to go there.

`/channels/followed` is the only Helix endpoint here that genuinely paginates,
and the only one whose `first` defaults to 20 rather than 100 — forget the
parameter and a long follows list quietly shows a fifth of itself.

**Refresh means "this list", not "follows".** One control, whichever list is up,
because the discovery tabs are otherwise fetched once and kept forever, which is
right for a page you glance at and wrong for one left open all evening.
`Request::Follows` is intercepted in `run` rather than handled in `serve`,
because `serve` cannot see the poll timer: answered there, the poll just done by
hand would be repeated automatically seconds later, for two of everything. And a
failed poll is `FollowsError`, never `Error` — the UI turns `Error` into
`SignIn::Error` and blanks the page, which is far too much to say about one
dropped request. It is only surfaced when somebody actually pressed the button;
the minute-by-minute poll fails to stderr, because an hour-long outage should
not be sixty toasts about a list that is still on screen.

Three lists on one page — following, popular, categories — because they are the
same question asked three ways, so they share one grid and one card. Only
categories look different, and only because box art is 3:4 rather than 16:9.
Opening a category *replaces* the page rather than nesting inside the tab, so
there is only ever one thing to scroll.

Everything the user does there arrives as one `browse::Action` rather than one
callback per control: the page is generic over its owner, so each extra closure
would be another type parameter threaded through every helper.

Two things worth keeping:

- A category's streams are dropped if the reply arrives after the user has left
  it. Without that check a slow response repopulates the page behind them.
- `RootView::fetch` refuses to set `loading` when there is nobody to answer —
  before sign-in, or after the worker has stopped. A request made while signed
  out would otherwise sit in the queue behind the device-code poll and pulse
  "Loading…" indefinitely. `fill_tab` picks it up once sign-in lands.

Lists are fetched once per tab and kept. There is no pagination: Helix caps a
page at 100, which is the top 100 streams or categories on Twitch, and that is
plenty to pick from. Adding "load more" means threading the `pagination.cursor`
Twitch already returns through `top_streams`/`top_categories`.

**Search** is three requests behind one result, and both halves have a reason:

- `/search/channels` answers with a **profile picture and no viewer count** — a
  different shape from every other list in the app. So only its logins are kept,
  and they go back through `/streams` to become ordinary stream records. One
  extra round trip buys cards identical to every other list. Helix takes up to
  100 `user_login` parameters, so it stays one request.
- Categories are searched in the same breath, because a name like "zomboid" is
  as likely to mean the game as a channel.

Results show **channels first**, and categories are capped at
`SEARCH_CATEGORY_LIMIT`. Twitch matches category names loosely — "moonmoon"
returns twenty-odd games with "moon" in them — and with categories first the
channel you actually searched for was below the fold. A live channel is directly
watchable; a category is another click.

Search runs on Enter, not per keystroke, since each one is three requests. The
**palette** is the opposite and deliberately so: it filters lists the app
already holds, costs nothing, and runs on every keystroke. Two boxes that look
alike doing different things is worth the words — one asks Twitch, one asks the
app.

**The palette's arrows and Escape are key events, not bindings.** Its own text
field is focused while it is open, that field's context is deeper than the
root's, and the keymap deliberately stands aside for a focused input — which is
the behaviour that keeps typing working everywhere else. A binding cannot win
that argument, so `on_key_down` reads the event on the way past instead.

**Avatars are a second request.** `/streams` carries a stream's preview, not the
channel's picture, so the rail gets its faces from `/users` — batched at Helix's
hundred per request, sent after the live list rather than with it, and merged
into what the UI already holds so the rail fills in rather than blinking.

**Card width is derived from the window**, by `browse::card_width`, and the row
is filled rather than merely fitted. A fixed 300px card left 306px of gutter
down one side of a 1600px window — one card short of another column — and a
different amount of it at every other size.

**What is only true right now goes over the picture**: the viewer count and
uptime sit on the thumbnail behind an `overlay()` wash, where broadcast UIs have
put them for decades. That corner used to hold a `LIVE` badge, which said the
same thing on every card in every list — all three lists are live-only, since
offline follows are names under their own heading — in the app's only saturated
red, while the number that actually varies sat in grey underneath.

### Chat

The pane reads a live log for hours, so nearly every decision in it is about
scanning rather than features.

**A hand-editable file has to survive being hand-edited.** `Settings::load`
strips a leading byte-order mark, because every obvious way to edit
`settings.json` on Windows writes one — Notepad's "UTF-8", PowerShell's
`Set-Content -Encoding utf8` — and `serde_json` will not take it. The failure
was silent twice: the app started on defaults, *and* could no longer save,
since every write reads the file back first to keep the tokens the worker put
there.

**The time is written the way the machine writes it.** `clock.rs` asks the OS
for its short time pattern once — `GetLocaleInfoEx` on Windows, a short-style
`CFDateFormatter` on macOS, a territory list from `LANG` elsewhere — and
renders every stamp through one small formatter that reads both Windows' and
ICU's grammars, with tests. A US machine reads `11:41 PM`, a German one
`23:41`, a Japanese one `午後11:41`, and a short time format the user edited by
hand in Region settings is honoured. Falls back to `HH:mm`, which is what chat
always showed.

**The time is said once a minute, not once a row.** It used to be a fixed 34px
gutter down the left holding a stamp per row, which in a busy channel is fifteen
consecutive `15:27`s standing in for a ruler — and stamping only the rows that
say something new leaves the gutter empty for most of them, which is 11% of a
300px pane reserved for nothing. `ChatView::time_break` draws the time and a
rule above the first row of each minute instead, and the rows get their width
back.

**An event row's body is wrapped in a `flex_row`**, and that is load-bearing. A
`message_line` is a `flex_wrap` row whose every word is `min_w_0`; dropped
straight into the event's `flex_col`, gpui sized it from its own content, which
for a line that can shrink to nothing is one character wide. The words then
wrapped one per line and painted over the rows beneath — so a resub with a note
attached, or an announcement carrying a link, came out as a vertical stack of
letters. Ordinary messages never showed it because they are already a row's
only child. Anything else that renders a `message_line` needs the same wrapper.

**Usernames go through `theme::readable`.** Twitch lets people pick any colour
and its own fallback palette ships pure blue, firebrick and seagreen — all
darker than the surface they land on. Lightness is lifted and hue kept, so
people stay recognisable by colour rather than being flattened to one, and the
lift stops at a *measured* contrast rather than a fixed lightness: the two are
not the same thing, and the flat floor this replaced left pure blue at 2.7:1.
The test runs against `twitch_chat::message::DEFAULT_COLORS` itself so the two
cannot drift.

**Anything per-row belongs to the row, never to its index.** The backlog drains
from the front once full, which shifts every surviving index. That inverted the
whole pane's stripe on every message past the cap, and would have cost links
their hover state too — rows carry a stable `seq` for exactly that.

**Words are classified in `chat_text`,** and the hard part is not matching URLs
but *not* matching them. Chat is full of `lol.`, `1.5` and `wtf.jpg`, so a bare
host only counts when what follows its last dot is a real TLD, and the list
deliberately omits `.so`, `.is`, `.at` and `.it` — real TLDs and common English
words both. Punctuation is split off the ends so a trailing comma is neither
underlined nor sent to the browser.

**Every word can shrink below its content width**, which sounds like it would
break words in half and does not: `flex_wrap` moves a word to the next line long
before it would have to shrink, so shrinking only ever reaches a word wider than
the *whole* pane. That is a long URL, in practice, and without it the link ran
off the edge of the chat — unreadable and unclickable past the boundary. The
breaking itself is gpui's job and it is better at it than a character cap would
be: `/` is not in `LineWrapper::is_word_char`, so a URL breaks at its path
separators, and a run with no break opportunity at all — an opaque media id — is
hard-broken at the edge rather than allowed to overflow.

**Mentions take the colour of whoever is being addressed,** from a login→colour
map that fills itself as people talk. A miss renders plainly rather than
guessing. This only works *because* of the readability clamp — without it you
would be scattering unreadable blues through body text, which is worse than
leaving mentions alone.

**Emotes overhang their line rather than growing it,** so a row with emotes is
no taller than one without. `ROW_PAD_Y` is what makes that work: the 4.5px of
overhang at each end of a row has padding to sit in.

**Between two *wrapped* lines of one message there is no padding at all** — they
sit exactly `LINE_BODY` apart — so an emote on the second line paints straight
over the descenders of the first, eating the tail of a `j` or a `g`, and gets
painted over in turn. That was live for as long as the overhang existed and is
only visible on a message long enough to wrap *and* carrying an emote, which is
why it survived a design pass and two rounds of screenshots. The fix is a
`gap_y` of one full overhang between wrapped lines, applied only to messages
that actually contain an emote, so a wrapped wall of plain text keeps its tight
leading. Single-line rows are untouched at 30px.

Worth knowing how this was diagnosed, because the obvious reading was wrong
twice. It looks like clipping, so the first guess was that something masks to
the row bounds — it does not: `Style::overflow_mask` returns `None` unless
overflow is set, and `list` masks to the whole list, not per item. The second
guess was that `ROW_PAD_Y` was too tight; raising it changed nothing, because
measuring the rendered pixels showed emotes at a full 28px either way. Measure
the row pitch and the emote's actual height before believing a screenshot.

**Scrollback holds where you put it** and resumes only when the wheel reaches
the very bottom, which in a fast channel is a long way down. The scrollbar and
the jump-to-live pill exist because nothing on screen said so otherwise — see
the `list` trap for why the thumb's *size* is not to be trusted.

**A pane opens with what was already being said.** Twitch publishes no
scrollback — IRC gives you what arrives after your JOIN and there is no Helix
endpoint — so the backfill comes from the community service Chatterino and
DankChat both use. It answers with **raw IRC**, which is the whole reason it is
cheap: the lines go through `message::parse_line` and `event_for` exactly as if
they had come off the socket, so a backfilled message is indistinguishable from
a live one and there is no second code path to keep in step. `event_for` exists
for that: it is the single answer to "what does this line mean", shared by the
session loop and the backfill.

Three consequences worth knowing. It is the only place the app asks a **third
party** for content, and doing so tells that service which channels are being
watched, which is why `Settings::chat_history` can turn it off. The fetch runs
**before** the socket rather than beside it, because history arriving after the
first live message would put older lines below newer ones — and only on the
first attempt, since refetching after a reconnect would re-deliver exactly what
is still on screen. And the service joins a channel the first time anybody asks
for it, so the very first request for a channel nobody watches comes back empty
and the one after it does not.

**"Connecting…" is a state of the pane, not a row in it.** It used to be a
notice row, which stopped working the moment history existed: a backfill arrives
stamped with the times those messages were really sent, all of them older than
now, so the row sat above an hour of history wearing a later timestamp than
everything beneath it. It is drawn in the empty space it explains instead, and
the pulse is safe there because the state always ends — at the join, or at the
first disconnect notice, and both put a row in the list.

**Events are washed, never outlined or barred.** A sub, a gift, a raid or an
announcement gets a tinted row and nothing that changes its geometry, because
the row still has to sit inside the ruler of timestamps you scan down. Two
intensities and no more: `msg-id` is an open set that grows whenever Twitch
ships a feature, and the `system-msg` tag is already finished English that says
which event it was. Anything unrecognised renders in full with the quiet wash —
the cost of a missing arm is a row that is not tinted, not a dropped event.
An announcement has no sentence at all and is nothing but body, which is why
`ChatNotice` has both halves optional and why a notice with neither is dropped.

### Keyboard

There was no key handling at all until late on, and adding it is mostly about
two GPUI behaviours that fail *silently*.

**A key reaches nothing unless something is focused.** The dispatch path comes
entirely from `window.focus`; with nothing focused it is the bare root node,
whose context stack is empty — and an empty stack fails every predicate. So
`RootView` holds a `FocusHandle`, takes focus on open, and takes it back through
`cx.on_focus_lost` whenever the focused element simply *disappears*, which is
what happens when the settings sheet closes and takes its buttons with it. That
listener fires only when the path empties, not when focus moves, so it cannot
loop. Without either half, every binding is dead and nothing says so.

**A key context and a key predicate are different grammars.** A context is
whitespace-separated identifiers (`Perch Watch`); a predicate is a
boolean expression over them (`Perch && Watch`). Interpolating the first
into the second parses to the first space and then fails, which
`KeyBinding::new` reports by panicking at startup. `keys.rs` builds predicates
from the same identifiers the contexts are made of and has a test pinning that
they agree, because a rename on one side would otherwise kill a whole page's
shortcuts quietly.

Three more things worth keeping:

- **Never bind with `context: None`.** It is scored at maximum depth and wins
  ties by later registration, so a context-free `escape` or bare letter beats
  gpui-component's own input bindings and eats typing — and on Windows the
  character is simply lost, because no `WM_CHAR` is generated. Everything here
  is scoped and carries `!Input && !Select && !PopupMenu`. `!X` scans the whole
  path rather than the current depth, which is what makes it work: the app's
  context is an *ancestor* of the focused input.
- **`track_focus`, never `id().focusable()`.** Giving the root div an id would
  re-namespace every descendant element id in the app, including the ones
  animated images depend on. `track_focus` needs no id and installs the
  mouse-down handler that re-arms shortcuts after a click — while a click on an
  input still focuses the input, because the inner handler calls
  `prevent_default` first.
- **The sheet replaces the page name in the context rather than adding to it**,
  so a page-scoped shortcut cannot fire through a modal and no binding has to
  remember to write `!Modal`.

There is deliberately **no transient feedback** for pause, mute or volume. Each
one announces itself through the thing it controls, so a flash of UI would only
be saying what you already know. What there *is* now is a standing one: the
pane header carries `muted` and `paused` tags, read off the `VideoView` at
render time. Those used to be visible only while the pointer was over the
video, so a channel saved muted opened silent with nothing on screen to say so.
The quality is deliberately not there — it is on the control bar, and a 340px
header with a name, a count, an uptime, a tag and `close` in it has no room for
a fifth thing; the count reads `358 · 8h 20m` beside the live dot, the card's
shape, for the same reason. The shortcut list lives in the settings sheet and
is read from `keys::SHORTCUTS`, beside the bindings, so a documented key is a
bound one.

**Fullscreen** is `f` on the watch page, `F11` on either, and a double-click on
the video. The double-click lives on the pane's root element; the control bar
over it is `.occlude()`d so a double-click on `pause` does not also reach it.
A single click deliberately does nothing there — it is how a pane is made the
active one, and pausing on a click would turn choosing a pane into stopping it.

**The window remembers where it was.** `Settings::window` is written from
`on_window_should_close` with the platform's restore bounds, so a maximised or
fullscreen window is saved as the size it would un-maximise to. On open it is
used only if some display still intersects it — a monitor unplugged since is
the common way to lose a window — and otherwise the default is centred and
fitted to the primary display, which the old fixed 1600×920 was not: it opened
with its bottom edge off a laptop screen.

### What deliberately does not move

Both of these were considered and rejected, so they read as decisions rather
than as things nobody got to:

- **Browse cards.** Hover is instant there. Fading each of a hundred cards would
  need per-card state in `RootView`, and sweeping a pointer across a grid feels
  worse with fades than without — the highlight lags behind the cursor.
- **Chat rows.** Animating arrivals would pin the frame loop at 60 fps for what
  is a text list, and in a fast channel a per-message fade is a strobe.

---

## Working on it

```bash
cargo build --release -p perch     # always release for anything perf-related
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

**Verify animation by measuring, not by looking.** One still cannot tell a fade
from a cut. Extend the same loop to burst-capture with a stopwatch and reduce a
small crop to a mean brightness per frame, then read the numbers: a cut is one
step between two values, a fade has intermediates. The control bar measured
`0 → 4.73 @ 87 ms → 6.56 @ 159 ms` going in and the mirror image coming out,
which is what a 120 ms symmetric fade looks like. Keep crops small — sampling a
full window through `GetPixel` is slow enough to distort the timing you are
trying to measure. For the waiting pulse the tell is the *ratio*: trough over
peak came out at 0.45, which is `PULSE_FLOOR` exactly.

To reach a state that only exists briefly, drive the app into it rather than
racing a restart — switching quality puts a pane back into `Starting` with the
window already open and stable.

**Measure the pixels, do not read the screenshot.** Two separate wrong diagnoses
of the emote overlap came from looking at a zoomed crop and believing it. What
settled it was scanning a column of the chat pane for divider lines to get the
row pitch, then scanning each row band for the tallest run of non-background
pixels to get the emote's real height. An emote that measures 28 in a 30px row
is not being clipped, whatever it looks like at 6x.

**Verify pixel correctness by dumping PNGs.** `mpv-frames`'s `dump_frames` example
writes frames to disk with stats (per-channel means, alpha minimum, non-black
percentage). A red Superman "S" on a blue suit is how BGRA vs RGBA got confirmed.

**Clean up processes.** streamlink is a child of the app and dies with it on a
graceful close, but a hard kill orphans it. `taskkill //F //IM streamlink.exe`
on Windows, `pkill -f streamlink` on macOS.

---

## Shipping a build

`.github/workflows/release.yml` builds `--release --locked` on two runners and
zips each result beside `LICENSE` and that platform's `RUNNING` page:

- push a `v*` tag and it cuts a GitHub Release, whose assets download without
  an account — which is the whole point, since the reason to build this at all
  is somebody who does not want to compile it;
- run it by hand (`workflow_dispatch`) and the same zips are run artifacts,
  which need a login and expire. That is for handing a build to somebody who
  is already here.

Publishing is its own job, gated on both builds. It has to be: two jobs each
calling `gh release create` on the same tag is a race whose loser fails on a
release that already exists.

**Windows** ships the bare `perch.exe`, and that is genuinely all it needs: the
widget icons are `include_bytes!` and the app icon is a linked resource, so
there is nothing beside the exe to lose.

**macOS** ships a `perch.app`, because on that system the icon, the name and
dock activation come from a bundle rather than from the executable — the one
place where the two platforms need different shapes rather than the same shape
built twice. `packaging/macos/bundle.sh` assembles it: the `.icns` is generated
from the same `perch.ico` Windows uses, so there is no second icon file to
drift, and the plist's version is read from the crate manifest. The binary is
universal, `lipo`'d from `aarch64-` and `x86_64-apple-darwin` slices both built
at `MACOSX_DEPLOYMENT_TARGET=11.0` so the `LSMinimumSystemVersion` it claims is
true of both halves.

The signature is ad-hoc. That is not a substitute for a Developer ID and is not
trying to be: an arm64 binary needs *some* signature to execute at all, and
copying files into a bundle invalidates the linker's. Gatekeeper still
quarantines the download, and `RUNNING-macos.txt` opens by explaining how to
clear it. Notarising properly means a paid Apple Developer account, which is a
decision rather than a task.

CI runs `bundle.sh` on the macOS leg against a debug binary. A bundle that
assembles and then will not launch is invisible to every other check, and a tag
push is the worst moment to discover one.

What neither zip carries is libmpv — somebody else's licence to redistribute,
from an MIT app — or streamlink, which is a Python application. The `RUNNING`
pages say where to get both, and on macOS that is one `brew install` for the
pair. Neither is a silent failure: each surfaces in the pane, naming itself and
the environment variable that overrides the search.

## What to build next

Nothing here is agreed. The four items that were, plus chat backfill, the
follows filter and window placement, are built. Ranked by what would be
noticed, roughly:

1. **Stream metadata for channels you do not follow.** The chat header's viewer
   count and uptime come from the follows poll, so they are blank for anything
   opened from popular, from search, or by name. `GET /helix/streams?user_login=…`
   per open channel would fill it.
2. **A stable order for the rail and the grid.** Both re-sort by viewers on every
   poll, so a row can move under the pointer while a menu is open. Keeping the
   order a channel arrived in for the session, or animating the move, are the
   two answers; neither is free.
3. **Badges in the chat gutter** — sub, mod, VIP. The tags already arrive and
   are parsed into the map; nothing reads them.
4. **Reply context lines.** `reply-parent-*` tags arrive too.
5. **Highlight rules** that wash the row background rather than colouring a
   word. The wash already exists for events.
6. **Rebindable keys.** `keys::bindings` is a plain `Vec<KeyBinding>` built from
   constants; the work is a UI and a settings shape, not a mechanism.
7. **Sign-out.** There is no way to clear a bad token except editing the field.
8. **The auth-token cookie off argv.** It is documented as a tradeoff, but a
   per-spawn `--config` file with a user-only ACL, deleted once streamlink has
   started, would take it out of the process list at the cost of one more file
   on disk. Not done here because it changes a documented decision.

**Known to be out of reach**, so nobody re-derives it:

- **Chatter counts.** The old `tmi.twitch.tv/.../chatters` endpoint was shut
  down on 3 April 2023 and returns 404. Helix `GET /chat/chatters` needs
  `moderator:read:chatters` *and* that the token's user moderates the channel.
  IRC `NAMES` still responds but stops listing above ~1000 users, so it returns
  nothing on exactly the channels worth asking about. `viewer_count` from Get
  Streams is the only public number.
- **Moderation, whispers, the emote picker, sending messages.** All need an
  authenticated connection. Sending is genuinely feasible — add `chat:edit` to
  the scopes and authenticate the IRC session — but the user has ruled it out:
  this is a viewer, not a chat client.
- **Text selection across a message.** Every word is its own element and GPUI
  has no cross-element text selection, so there is no contiguous run to select.
  Links being clickable is currently the only way to get a URL out of the pane.
- **Link previews.** Chatterino ships this off by default on privacy grounds,
  which is a strong enough hint not to build it.
- **Chat scrollback from Twitch itself.** There is none. The website renders its
  own history server-side and exposes no endpoint; every client that shows it
  uses the third-party service `twitch_chat::history` talks to.

## Paging, and what is not paged

`twitch_api::Page` carries a cursor back out to the caller; `browse::Listing`
holds one beside the items it belongs to. The two followed endpoints do *not*
work that way — they walk their pages inside the API layer until Helix stops —
and the difference is that they finish. You follow a fixed number of people;
"popular" is every live channel on Twitch, so how far to go is the user's call
and the cursor has to survive the round trip to reach them.

Search is deliberately unpaged. `SEARCH_PAGE_SIZE` is 40 and
`SEARCH_CATEGORY_LIMIT` is 12 because a short relevance-ordered list is the
feature — see the comment on the latter.

`Listing::absorb` takes an `append` flag rather than working it out, because a
reply carries no memory of the request that asked for it. Refresh always starts
a list again: appending a fresh page one onto a stale page two is neither the
old list nor the new one.

## Known limits

None of these is being worked on; all of them are real.

1. **The chat scrollbar's thumb size drifts.** Its position is right. See the
   `list` trap — a real fix means not using gpui's scrollbar geometry.
2. **Chat keeps 1000 messages** and drains from the front even while you are
   scrolled back reading them. Raised from 500 when the pane started opening
   with a backlog; a row is a `ChatMessage` and a few `SharedString`s, and only
   the visible ones are ever laid out, so it can go further if it needs to.
3. **Quality does not re-pick on resize.** It is chosen when a channel opens,
   using the pane size at that moment.
4. **No sign-out**, and no way to clear a bad token except editing the field.
5. **Animated WebP** (7TV, some BTTV) may render as stills. Twitch's own
   animated emotes are GIF and animate correctly.
6. **`ImageCache::new` is still on the pre-window UI thread.** It is now
   bounded rather than unbounded — `scan_and_prune` trims the permanent
   directory to 256MB oldest-first and deletes `.part` debris in the same pass
   it indexes, so startup no longer gets slower with every run — but it is one
   `read_dir` plus a stat per file, done synchronously in `RootView::new`.
   Deleting `%LOCALAPPDATA%/perch/images` is always safe.
7. **Orphaned streamlink on a hard crash.** A Windows job object would close it.
8. **Never tested on a vertical monitor.** The layout derives portrait grids and
   stacks chat below video, the logic is unit-tested, but nobody has seen it.
9. **The offline follows list has no cap.** Someone following several hundred
    channels gets several hundred names. There is a filter now, on the tab and
    in the palette, so they can be found; the wall is still a wall.
10. **A volume drag writes `settings.json` per pixel.** Pre-existing: every
    `SliderEvent::Change` is a full read-modify-write of the file, and there are
    a hundred of them in one drag. `set_volume_for` returns whether anything
    changed so a repeated value is free.

    The `Slider` still reports no drag-end, but that is no longer the obstacle
    it was written as: the divider drag has the same shape and solves it with
    `RootView::on_mouse_up` on the root, because the *window* sees the release
    even when the widget does not. Whoever fixes this can hang a save on the
    same listener rather than inventing a debounce.
11. **Chat history depends on somebody else's server.** If it is down the pane
    opens blank, which is what it did before the feature existed. Failures go to
    the log rather than the pane, on purpose.
12. **Shortcuts are not rebindable**, and `Esc` does not close the video quality
    menu — that menu is hand-rolled rather than a gpui-component popup, so it
    carries no key context of its own.
13. **The palette scrolls with no visible scrollbar.** It is a window of eight
    rows around the selection, so arrowing scrolls it; the wheel works and
    nothing says so. The settings sheet, the browse lists and the rail now draw
    gpui-component's `Scrollbar` over a tracked `ScrollHandle`.
14. **The rail lists live channels only.** Offline follows are on the browse
    page and in the palette, which is the same gap as limit 9 seen from the
    other side.

## Things not to redo

- Do not turn hardware decode off "for performance", and do not turn any
  video setting on or off on the strength of one short A/B. See the
  performance section for how the same option measured a loss and then a win.
- Do not let mpv upscale.
- Do not reach for mpv's `profile=fast` or `dither=no` to cut the render cost.
  Dithering is a `vo=gpu` shader stage and is not in the software render path at
  all: `dither=no` renders byte-for-byte identical frames (compared by hash) and
  measured the same, four alternating pairs splitting 2-2 — what looked like a
  20% saving was one outlier dragging a mean. `profile=fast` *does* measure
  faster, by also forcing bilinear scaling, which every pane in a grid downscales
  through: picture quality spent on a number that is mostly not real. Forcing
  `scale=bilinear` on its own measured *slower* than the default.
- Do not derive control visibility from `group_hover` or from `on_hover`'s value.
- Do not key animated-image element ids on position.
- Do not use `--stream-url` to skip streamlink's pipeline.
- Do not add tokio; use a thread plus an mpsc pump.
- Do not put a repeating animation on a state that can persist.
- Do not assume an overlay blocks input because it covers something; use
  `occlude` / `block_mouse_except_scroll`, and put it on the smallest thing that
  is actually opaque.
- Do not cache an image whose URL is stable but whose content is not, and do not
  refresh one in place — GPUI decodes per path.
- Do not derive anything per-row from a row's index; the backlog drains from the
  front.
- Do not bind a key with `context: None`; it outranks every scoped binding and
  swallows typing.
- Do not interpolate a key *context* into a key *predicate*; they are different
  grammars and the failure is a startup panic.
- Do not expect a shortcut to fire with nothing focused, and do not skip
  `on_focus_lost` — focus is never reassigned when the focused element vanishes.
- Do not merge offline follows into `Vec<LiveStream>`; three separate things
  read that list as "who is live".
- Do not answer `Request::Follows` in `serve`, which cannot reset the poll timer.
- Do not report a failed follows poll as `TwitchEvent::Error`; the UI reads that
  as "signed out".
- Do not size a video's container from a percentage height in a block parent.
  `img` carries the frame's aspect ratio and taffy will use it the moment the
  percentage cannot resolve, which then decides the next frame's size. Flex
  container, stretched item; see the video trap.
- Do not call `.hover()` on a control from `controls.rs` a second time. gpui
  allows one hover style per element and asserts on the second in a debug
  build, so a debug build with two panes open panicked on the close button.
  A different pointer behaviour is a new `Variant`, which is what
  `Destructive` is.
- Do not let anything overhang its line without checking what is directly above
  and below it — inside a wrapped message that is another line, not padding.
- Do not `join()` a worker thread from a `Drop` that runs on the UI thread.
  All four workers — the two supervisors, the mpv render thread and the chat
  client — are dropped from click handlers, and neither a `stop` flag nor a
  killed child reaches a thread that is inside a network read, a TCP connect,
  or `mpv_terminate_destroy` — so a join froze the window for as long as that
  took. Set the flag, kill or shut down what you can, and let the thread retire
  on its own. Every one of them tests `stop` between phases for exactly this.
- Do not call `Player::property`, or any synchronous libmpv function, from the
  render thread. Observe the property and read it from `poll_events`; see the
  video trap on the render-thread rule.
- Do not leave a socket read without a timeout. The chat socket has an idle
  timeout and sends its own `PING` on the first silent stretch, because a
  connection that died without a reset — a sleep, a NAT table — otherwise
  parks the reader forever and the pane simply stops.
- Do not use `.output()` or `.status()` on a child something else may need to
  kill; both own the `Child` internally, so there is no handle to reach it by.
  See `streamlink::run_tracked`.
- Do not hand `Library::new` a bare filename on Windows. With no path separator
  it uses the standard DLL search order, which includes the working directory.
- Do not use `.truncate()` and expect an ellipsis. It sets one, and gpui only
  applies it when the measure pass has a definite width — which a child of a
  flex *column* does not get, so the text is clipped mid-glyph by the ancestor's
  `overflow_hidden` and eats its own padding on the way out. `text_ellipsis()`
  plus `line_clamp(1)` takes the wrapping path, where the width is known. Both
  were built and looked at; only the second one truncates.
- Do not put a `flex_wrap` row of `min_w_0` children directly inside a flex
  column. gpui sizes it from its own content, and a line that can shrink to
  nothing measures one character wide — so it wraps one letter per line and
  paints over whatever is beneath. Wrap it in a `flex_row` first; see
  `render_event`.
- Do not assume `gpui-component` draws its own icons. It asks the *host* for
  `icons/<name>.svg` and ships none, so with no `AssetSource` every chevron,
  eye and clear button renders as nothing — and silently, since a missing asset
  is not an error anywhere in that path. The clickable ones are still there and
  still clickable, which is worse than absent.
- Do not let `gpui_component::init` have the last word on the palette. It seeds
  itself from `cx.window_appearance()`, which is the *operating system's*
  light/dark setting, and nothing else in this app asks the OS anything. Call
  `widget_theme::apply` after it.
- Do not set a margin on the element you hand to `motion::arrive`. It animates
  `mt` and overwrites whatever is there, so the offset belongs on a wrapper.
  The palette spent a build cycle against the top of the window this way.
- Do not write a comment asserting a guarantee the code does not enforce. Two
  were found this way — streamlink's credential "is never logged and never
  echoed" while it sat on the child's argv, and `keys::SHORTCUTS` being "beside
  the bindings" as though proximity were a check. Both were true when written.
  If it is worth claiming, it is worth a test.
