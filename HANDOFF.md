# Handoff

For whoever picks this up next. `README.md` covers *using* it; this covers
*working on* it — the architecture, the traps, and the things that cost real
time to discover and would cost the same again.

State: 18 commits, 81 tests, clippy clean, ~8600 lines.
Nothing pushed; there is no remote.

---

## What it is

A native Twitch client in one window: browse who you follow or what is popular,
watch up to four of them at once, each with its own chat. Rust + [GPUI](https://github.com/zed-industries/zed)
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
  twitch-api    device-code sign-in, follows, top streams, categories, search
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
| `browse.rs` | the picker page: following, popular, categories |
| `watch.rs` | the grid of panes; `Slot` lives here |
| `layout.rs` | derives grid shape from window aspect (pure, tested) |
| `video_view.rs` | player element + overlay controls |
| `video.rs` | render thread; owns the mpv `Player` |
| `chat.rs` | chat pane, emote rendering |
| `settings_view.rs` | settings sheet |
| `twitch.rs` | the worker: sign-in, follows polling, browse requests |
| `theme.rs` | **all** colour, spacing, type and motion tokens |
| `motion.rs` | the four animation shapes, and the state one of them needs |
| `diagnostics.rs` | where stderr goes when there is no console |

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
  and you block its whole bounding box, including empty space: the miniplayer
  strip is `items_end`, so occluding the strip would have left an invisible dead
  patch over the card grid above the short "stop all" pill. Toast cards and
  miniplayer tiles are occluded individually for the same reason.
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
stderr at `%LOCALAPPDATA%
ativetwitch
ativetwitch.log` before anything can
write, and keeps the previous run as `.log.old`. It works by `SetStdHandle`
rather than by a logging facade because Windows resolves that handle on *every
write* — so it catches the library crates and panic output too, with no change
anywhere else. That was verified with a throwaway before the code was written;
if you ever doubt it, verify it again rather than assuming.

Debug builds keep their console on purpose (`#![cfg_attr(not(debug_assertions),
windows_subsystem = "windows")]`), which is also the only place `--help` is
readable.

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

**A `list`'s scrollbar extent comes from *measured* items only.** gpui measures
rows as it draws them, so in a live-appending list the ones you have not looked
at contribute nothing to `items.summary().height` — which is what
`max_offset_for_scrollbar` is derived from. The visible symptom is a thumb that
changes size as you scroll, because scrolling is what does the measuring.

`ListState::measure_all()` is the documented remedy and chat uses it, but it is
only half a fix: the pass runs once, and rows spliced in afterwards are
unmeasured until drawn. Re-triggering it needs `reset()`, which also clears the
scroll pin — so doing it per message would throw away the reader's position,
which is worse than an imprecise thumb. The thumb's *position* is correct
regardless; only its size drifts. A complete fix means not using gpui's
scrollbar geometry, which is a bigger job than it looks.

Related and worth knowing: `reset()` is the **only** thing that clears the
scroll pin. Every `scroll_to`/`scroll_by` sets one, so a programmatic "jump to
bottom" built from those lands at the bottom and is then left behind by the next
message. Scrolling with the wheel re-arms auto-follow on its own, but only on
reaching the very bottom — which in a fast channel can be a long way down.

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
- **Spacing** — named by role (`PAGE_PAD`, `PANEL_PAD`, `CONTROL_PAD_*`,
  `GAP_TIGHT`, `GAP`, `GAP_SECTION`, `PANE_GAP`, `ROW_PAD_*`), not by size.
- **Type** — five roles (`TEXT_TITLE/BODY/LABEL/META/MICRO`). Label and meta share
  a size but differ in weight, so a thing you can click never looks like a thing
  you can only read. `weight_shout()` (bold) is reserved for the live badge.
- **Motion** — three durations named by job (`MOTION_HOVER`, `MOTION_ENTER`,
  `MOTION_VIDEO`) plus the waiting pulse, and two easings (`ease_fade` for
  two-way changes, `ease_enter` for arrivals). Motion says *that something
  changed*; anything long enough to wait for is too long.

Audit commands, worth re-running after UI work:

```bash
grep -ohE "\.(p|px|py|gap|gap_x|gap_y)_[0-9p]+\(\)" crates/nativetwitch/src/*.rs | sort | uniq -c
grep -ohE "\.text_(xs|sm|base|lg|xl)\(\)|FontWeight::[A-Z_]+" crates/nativetwitch/src/*.rs | sort | uniq -c
grep -nE "Duration::from_(millis|secs)" crates/nativetwitch/src/*.rs
```

The first two should return nothing outside `theme.rs`. The third will show
genuine timings — the follows poll, the toast lifetime, an mpv frame wait — but
no *animation* duration should appear outside `theme.rs`.

### Where controls live

**Nothing static is ever drawn on the video.** Static information on a moving
picture is exactly what you end up staring past for three hours, so it lives in
a header above chat instead — chat is already a panel, so it costs nothing
there. The split:

- **Chat header**, always visible, one per pane: live dot, channel name, viewer
  count, uptime, and the pane's close control.
- **Over the video**, hover-revealed only: the playback bar (pause, mute,
  volume, quality) and the single page-level "← follows" pill in the top-left.
  Point at the video and they come up; look away and the picture is all that is
  left.

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
takes the rest. Below it, the **video** gets a fixed 16:9 box and **chat** takes
the rest — a window is tall because you want more chat, not more letterboxing.
`layout::video_box_height` owns that, sized from a constant rather than from the
stream: render size follows the pane, so sizing the pane from the frame would be
a feedback loop. A test pins the invariant that the box can never fill the cell
it stacks in.

Left-anchoring the pane controls put the first pane's close button underneath
the page navigation, and since neither called `cx.stop_propagation()` a single
click closed a pane *and* navigated away. Top-right is the only anchor that
clears the corner for every grid shape `layout.rs` can derive — with four
columns, the third pane's *left* edge also lands under a centred nav.

### Browsing

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

Search runs on Enter, not per keystroke, since each one is three requests.

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

**Verify pixel correctness by dumping PNGs.** `mpv-frames`'s `dump_frames` example
writes frames to disk with stats (per-channel means, alpha minimum, non-black
percentage). A red Superman "S" on a blue suit is how BGRA vs RGBA got confirmed.

**Clean up processes.** streamlink is a child of the app and dies with it on a
graceful close, but a hard kill orphans it. `taskkill //F //IM streamlink.exe`.

---

## Open items

Roughly in the order I would take them.

1. **Line endings are mixed across the repo**, and `cargo fmt --all` normalises
   the CRLF files to LF — which rewrites twelve whole files that have nothing to
   do with your change. Until a `.gitattributes` settles it, format the crate
   you are working on (`cargo fmt -p nativetwitch`) rather than the workspace,
   and check `git diff --stat` before committing. `cargo fmt --check` also
   reports pre-existing deviations in `chat.rs` and several other crates; the
   repo has never been fully fmt-clean.
2. **Quality does not re-pick on resize.** It is chosen when a channel opens,
   using the pane size at that moment. Maximising afterwards grows the render
   buffer but not the stream quality.
3. **No sign-out**, and no way to clear a bad token except editing the field.
4. **Animated WebP** (7TV, some BTTV) may render as stills. Twitch's animated
   emotes are GIF and animate correctly.
5. **Follows poll every 60 s** with no manual refresh.
6. **Orphaned streamlink on a hard crash.** A Windows job object would close it.
7. **Never tested on a vertical monitor.** The layout derives portrait grids and
   stacks chat below video, and the logic is unit-tested, but nobody has seen it.

## Things not to redo

- Do not enable hardware decode "for performance".
- Do not let mpv upscale.
- Do not derive control visibility from `group_hover` or from `on_hover`'s value.
- Do not key animated-image element ids on position.
- Do not use `--stream-url` to skip streamlink's pipeline.
- Do not add tokio; use a thread plus an mpsc pump.
- Do not put a repeating animation on a state that can persist.
- Do not run `cargo fmt --all` here until the line endings are settled.
