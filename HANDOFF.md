# Handoff

For whoever picks this up next. `README.md` covers *using* it; this covers
*working on* it — the architecture, the traps, and the things that cost real
time to discover and would cost the same again.

Roughly 45,000 lines across seven crates. `cargo test --workspace`,
`cargo clippy --workspace --all-targets` and `cargo fmt --all --check` are all
expected to pass; if one does not, that is the change you are looking at, not
the baseline.

---

## What it is

A native Twitch client, one window by default: browse everyone you follow —
live or not — plus what is popular and what is on; search for a channel; and
watch up to four at once, each with its own chat — rearranged by dragging, one
of them given the whole window, or, on Windows, any of them popped out into a
small window of its own on top of other apps. Rust + [GPUI](https://github.com/zed-industries/zed)
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
  streamlink    supervises streamlink as a headless Twitch byte source, and
                resolves a recording and reads its playlist
  twitch-chat   read-only chat over anonymous IRC, the history backfill, and
                the replay of a recording's chat
  twitch-api    device-code sign-in, follows, top streams, categories, search,
                and channels like the ones watched, from Twitch's unofficial
                SideNav query (`recommend`)
  emotes        Twitch/FFZ/BTTV/7TV resolution + disk image cache
  settings      persisted user settings, and what has been watched
  perch         the app
```

Every crate except the last is free of UI types, deliberately — they are
testable without a window, and the video pipeline in particular was built
standalone before any GPUI existed.

App modules:

| file | role |
|---|---|
| `main.rs` | the process: first perch or a launch to hand over, the window, where stderr goes |
| `instance/` | one perch per settings file: the claim, and a later launch handing its arguments to the running one — a named pipe on Windows, `flock` and a socket on Unix |
| `launch.rs` | what a launch's arguments ask for, read the one way at startup and on a handover (pure, tested) |
| `trail.rs` | back and forward: the places behind and ahead, what a step passes over, and forgetting a place that is gone (pure, tested) |
| `stage.rs` | where each pane is drawn — a pane, a mini-player tile, a window of its own, or nowhere while another pane has the watch page (`Place`) — which panes are popped out and which one is maximized (`Stage`), the cells the watch grid draws (`cells`) and what a pane's maximize control offers (`MaximizeButton`): the one owner of the answer both windows ask before drawing a player; and where a pane moved one place along the order goes (`moved`) (pure, tested) |
| `os_window.rs` | what Perch asks of a window that gpui does not: its platform handle (`hwnd`, the one place a gpui window is asked for it — `instance::bring_forward` comes forward on it rather than looking it up itself), and keeping a pop-out above every other app (`keep_on_top`, Windows only) |
| `root/` | the app: `RootView` and its state in `mod.rs`, then one `impl` block per concern — `shortcuts`, `commands` (the palette), `follows` (the worker's events), `browsing`, `navigation` (back and forward: where the app is as a `Route`, each step recorded on the trail, and the arrows, keys and side buttons that walk it), `streams` (opening, restarting, closing, and `replace_with_video` and `replace_with_channel`, a recording swapped in for a live pane in place and the channel back in a recording's, both through `replace_slot`), `renditions` (what each pane plays, and when it restarts: the quality chosen against each pane's own height, `pane_height_for`, the upward re-pick when the grid changes, `sync_quality`, a pick from the pane's own menu, and how each changes what plays, `change_rendition` — beside a picture that covers the pane, to take over in place, and cold otherwise; and the root's half of that swap: what a start resolving beside a pane does with its events, `pending_event`, and what the pane keeps once its player has taken over or been given up on, `on_swapped` and `on_swap_failed`; and `set_pending`, the only write of a pane's pending start, which tells its player what a pick is switching it to and which a test holds the root to; tested), `panes` (where each pane is drawn, applied: `restage`, the one funnel every change of a pane's state, of which panes there are, of the page, of what is popped out and of what is maximized ends in; `set_slot_state`, the only write of a pane's state, which a test holds the root to; `video_in_main`, the only way the main window reaches a player, which a test holds the mini player and the pages to; and the maximize as the root drives it, `toggle_maximize`, `show_all_panes`, and `choose`, which takes it to the pane chosen; and `move_pane`, two panes swapping places in the order, from a header dropped on a pane or `Shift+←`/`Shift+→`), `pop_out` (a pane's picture in a window of its own, on top of other apps: the `PopOut` view, opening and closing it through `cx.defer`, `to_root`, the only way back to the root from it, and `offered`, Windows only), `broadcasts` (what a stopped live pane asks about its channel's past broadcasts, and whether it may offer to start by itself), `rewind` (a live pane's timeline: when each live player's broadcast began, `sync_live_since`, a press on it carried out at once or when the ask for the archive is answered, and the archive opened in the pane's place; and the way back: whether each recording's broadcast is still on, `still_live` and `sync_back_to_live`, and `LIVE` pressed, `back_to_live`), `pane_actions` (what a pane asks for, by its key: a press on it or one of its controls, which takes the keys back for the root first, and its player's requests; the moment a pane key brings a pane's header up over its picture; and `run_guard`, which swallows the rest of a double-click whose first press took a player out from under the pointer — More's `Pop out`, the mini player's — where the player's own guard cannot follow), `launches` (what the command line named, now and from later launches), `history` (where each recording was left, and resuming there), `prefs`, `recommended` (the rail's Recommended group: when to ask the worker, what its answer becomes, and the hooks that call it), `last_live` (when the offline follows were last live: when to ask the worker, and what its answer does), `chrome` (pills, toasts, the rail), `mini_player` (what plays on while you browse, in the corner of the page), `title_bar` (the bar Perch draws across the top of the window — the rail button, back and forward, the search box, the gear, the caption buttons on Windows — and which platform gets which shape of it), `pages` (each page only its own column; the rail beside it is drawn once by `mod.rs`) |
| `target.rs` | what a typed or pasted thing means: a login, or a twitch.tv link to a channel or a recording; `link`, its inverse and the one place a twitch.tv URL is written, and `moment`, the second a link to a recording starts at (pure, tested) |
| `browse.rs` | the picker page: home, popular, categories, search; which of them is on screen (`Discovery::place`) and which lists are still being waited on |
| `channel_page.rs` | one channel's past broadcasts, and when each was; the recording card both pages use, and a stopped pane too; a recording's poster, and `archive_of`, which finds the recording of a broadcast that just ended (tested) |
| `history_page.rs` | the history tab, its entry card and its test for unfinished (both reused by Home), and the one translation between a video and a history entry |
| `home.rs` | Home, the tab the app opens on: live follows, Continue watching (one row of the history), offline follows by when last watched, each saying when it was last live; the filter over all three, and the counts on the headings after it (`counted`) |
| `watch.rs` | the grid of panes; `Slot` lives here, made by `Slot::new`, with `PendingStart`, a start of its stream resolving beside the picture, and `PaneAction`, everything a pane asks of the root; the band a header rides over the picture on, and when it is up (`Slot::point`, `band_wanted`, tested); the layer a dragged header is dropped on (`drop_layer`, `PaneDrag`) |
| `watch/header.rs` | a pane's header — name, numbers, what is on, `muted`/`paused`, the pop-out icon that turns into Bring back, and the × — each icon naming its key — the handle a pane is dragged by onto another, and `Placement`, the one rule for where it goes: the chat panel, or over the picture with no chat on screen (tested) |
| `watch/status.rs` | a pane with no picture: `Showing`, the one reading of its state that the pane's sentence and the mini player's word both come from, and the screen drawn from it under the player — a starting pane's poster, a stopped one's next steps and what room it has for them (`next_up_room`), and `Elsewhere`, a pane whose picture is in a window of its own, with `Bring back` (tested) |
| `layout.rs` | derives grid shape from window aspect, as `Grid::of`, the one grid the watch page, the divider drag and each pane's quality read; `quality_height`, the height a pane asks a rendition for; the page's `Body`, the title bar's height and drag edge, and the mini player's tiles, how far in it floats clear of the scrollbar, and the `Room` a browse list leaves for it; where a pop-out opens, stacked clear of where the open ones really are (`pop_out_bounds`) (pure, tested) |
| `video_view.rs` | the player element: its sound, its hover, the picture; drawn as a pane, a mini-player tile, in a pop-out or not at all while another pane is maximized (`stage::Place`), moved between them only by `set_place`, and belonging to no window |
| `video_view/swap.rs` | a rendition swapped in place: the new stream started beside the one on screen inside the same view, held to it by `align` (tested) and promoted once it has a picture and, on a recording, has caught up; `route`, the one rule a stream's frame wakes and its streamlink events reach the right start by (tested) |
| `video_view/bar.rs` | the control bar over a playing picture: the seek row on a recording, or a live broadcast's timeline where the pane is wide enough for it (`timeline_fits`, tested), the icons and their key-naming tooltips, the quality pill (`pill_face`, tested — what plays, or a pick under way, breathing), the maximize control (`maximize_control`), what fits at the pane's width (`fit`, tested — the maximize folds into More after the quality pill), the one anchor menus open from; a pop-out's own right-hand end, Bring back and Close |
| `video_view/menu.rs` | the bar's menus, the quality and More (with the maximize once the bar has folded it, and `Pop out` where the pop-out is offered): which is open (`Menu`, one at a time), the box, rows that act on the press (tested), and `run_guard`, which swallows the rest of a double-click a row took (tested) — after `Pop out`, which takes the player out of the window, the root's `run_guard` does |
| `loudness.rs` | one pane's level and the Mute all hush over it: what mpv hears, and the only level ever reported to be remembered (pure, tested) |
| `seek_bar.rs` | the bar on a recording, and on a live pane its timeline, and the arithmetic behind it |
| `rewind.rs` | rewinding a live pane, worked out: whether a timeline is offered (`span`), where a press lands (`pressed_at`, never within `EDGE_SECS` of the edge), the archive of the broadcast going on now (`archive_for`), where in it a moment is (`position_in`), the ask and the press waiting on it (`Rewind`), and whether a recording is the archive of the broadcast on now (`live_now`), weighed against what the pane knows of where it came from (`Origin`), which offers the way back to live (`back_to_live`) (pure, tested) |
| `video.rs` | render thread; owns the mpv `Player` |
| `vod.rs` | positions a recording by rewriting its playlist; the keeper for one still growing |
| `chat.rs` | chat pane: rows, emotes, scrollback |
| `chat_text.rs` | what a word in a message is — link, mention or plain (pure, tested) |
| `settings_view.rs` | settings sheet |
| `twitch.rs` | the worker: sign-in, follows polling, browse requests, a stopped pane's ask for its channel's past broadcasts (which a live pane's rewind rides too), the rail's anonymous ask for channels like the ones watched (`Request::Recommend`), and the anonymous ask for when the offline follows were last live (`Request::LastLive`), both answered ahead of the session's upkeep |
| `keys.rs` | the keymap: actions, bindings, contexts, the listing, and the keys a tooltip may name (`Hint`) |
| `theme.rs` | **all** colour, spacing, type and motion tokens |
| `sidebar.rs` | the follows rail down the left, beside both pages: Pinned, Live, Recommended, then Offline folded under a count, each offline row saying when it was last live where a live row says what is on (`groups`, pure, tested) |
| `last_live.rs` | when each offline follow was last live, worked out: when to ask (`LastLive::next_ask`), what is kept, and the words ("Live 3 hours ago", `wording`, reusing `channel_page::when` past a day) (pure, tested) |
| `recommended.rs` | the rail's Recommended group worked out: the seeds (`seeds`), when to ask (`Recommended::next_ask`), and what the answers come to (`suggestions`, `reason`) (pure, tested) |
| `palette.rs` | the command palette, and what it can run |
| `controls.rs` | the one button, the variants it comes in, the icon button, the heading that folds (`fold`) and the plain one it sits among (`group_heading`), the window's caption buttons, the picture a pop-out is dragged by (`drag_layer`), and the two tooltip builders (`tip`, `full_text`) |
| `widget_theme.rs` | hands `theme.rs` to `gpui-component`'s own palette |
| `assets.rs` | the icons: the ones `gpui-component` asks the host for, and Perch's own, typed as `assets::Icon` |
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
merely for tidiness — and so does the rail's recommendations ask, which reads
Twitch's unofficial GraphQL with no token, as one more request on the same
queue. Chat's own requests stay with chat and never pass through the worker:
its IRC, its history, and a recording's replay, which is GraphQL too (see
"Chat"). The worker takes requests on a `std::sync::mpsc` channel and waits on
`recv_timeout` against the next follows-poll deadline — the wait and the
mailbox are the same thing, so browsing never queues behind the timer.
Dropping the service drops the sender, which wakes the worker immediately
rather than after the poll interval.

**And one process owns the settings file, for the same reason.** A second
perch on the same file was a second worker spending the same tokens — and a
second writer the `settings` crate's lock knows nothing about, since that lock
is per process. `instance` makes the first copy the only one: a later launch
hands its arguments over and exits. See "One perch".

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
cold restart. `Drop` cannot do it; it has no `App`. (A rendition swapped in
place keeps the view, and `render` frees the old stream's tile instead.) `cx.on_release` can, and
the `Subscription` it returns has to be kept in a field or the hook is dropped
immediately. The player's hook calls `App::drop_image(frame, None)`, which
visits every window, because the player may be drawn in a pop-out by then and
each window has an atlas of its own; a release runs as effects flush, after
whatever window was being updated is back in place (app.rs:759-776), so none
is skipped as leased. `ChatView` keeps `on_release_in` for its emote cache:
chat is only ever drawn in the main window.

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

**A player that cannot open has to say so.** The pane is `Playing` from the
moment `VideoStream::start` returns, and a view with no frame draws the
starting screen under it. When `Player::open_with` failed, the render thread
used to log it and end, and the pane said "Starting…" for good. It now fills
`stopped` with `Stopped::Failed("could not open the player: …")` and wakes the
frame channel, as an end of file does, so the view emits `VideoEvent::Stopped`,
`stream_stopped` retires the player, and the pane offers `Try again`. A
recording whose playlist cannot be written already did this. Not yet the
loop's own exits: a frame that fails to render, a buffer that does not match
its size, and mpv's `Shutdown` still end the thread without a word, leaving
the last frame up as if paused.

**`mpv_render_context_render` blocks until the frame's display time**, up to
`video-timing-offset` (50 ms default). That wait is what keeps video timed to
audio, so it is wanted — but calling it on the UI thread stalls the whole GPUI
frame loop. It runs on `video.rs`'s thread for that reason. A benchmark that
reads exactly 16.66 ms is measuring this, not CPU.

**A paused stream redraws when its size changes.** The render thread renders
when mpv says a new frame is ready, and a paused player sends none, so a
paused pane that grew — the window made larger, a pop-out resized or brought
home, a swap promoted while paused — went on stretching its last small frame
until it played or seeked. Now, with the pause applied and the size asked for
different from the last frame's, the loop skips the wait and renders at once
(`video::redraw_now`): with no new frame queued, mpv draws the one it holds
again ("If no new frame is available, the previous frame is redrawn",
render.h), and does not block, since nothing new is presented. Once per size,
so it never spins; and never from a reposition until the new file's first
frame, because loading a file can free the frame mpv holds (vo_libmpv's
`reconfig` and `uninit`), and a redraw with none draws an empty picture. Not
yet seen in the app: that the software renderer's redraw is the sharp frame
rather than a black or stale one. If it is not, take the redraw out and list
a paused pane as soft after growing, until it plays or seeks, under Known
limits.

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

**Quality is chosen again when the grid changes, and only ever upwards.**
`RootView::sync_quality` runs after a pane opens or closes, when the rail
toggles, and 750 ms after the last window resize (`observe_window_bounds`, so
fullscreen and maximise count) — and changes only a pane whose Auto or Fixed
choice now names a *sharper* rendition; a pane that shrank keeps what it has,
and a choice made from the pane's own menu is left alone. Changing rendition
means resolving the stream again, since streamlink cannot switch mid-stream;
on a pane with a picture the new one plays beside the old one and takes over
in place (the next trap), so it costs a second stream for a few seconds
rather than any black: worth it for a picture that was soft, not for a CPU
saving in a pane that hides nothing — which is also why the re-pick waits for
a resize to settle rather than running per event. Each pane is measured on its
own, by its key (`RootView::pane_height_for`, in `root/renditions.rs` with the
rest of the quality code): its cell of the one grid, in physical pixels
(`layout::quality_height`), measured as its share of the body's height with
the seams left in (`Grid::share_height`), as panes always were. The cell as
cut is a pixel or two shorter in two rows, and renditions are picked by exact
heights, so measuring that would move a pane at the edge to a softer
rendition. The mini player is not one of the sizes it chooses for:
`pane_height_for` measures the watch grid whichever page is up, so a tile
never feeds it, the rendition playing in the corner is the one you go back
to, and going back to watching never restarts a stream.
A player that is started cold off the watch page — a quality change from the
settings, a re-pick of a pane with no picture yet — is born where its pane is drawn
(`video_view::Start::place`, from `RootView::place_of`): a tile, as the one it
replaces, or in its pane's pop-out, with that window's focus
(`RootView::focus_of`). A pane in a window of its own is measured by that
window, not by the cell it keeps at home: `PoppedOut::height`, the picture's
height in that window's own physical pixels, which the pop-out's bounds
observer measures at its own scale (another monitor's may differ from the
main window's) and hands the root through `to_root`. A change of it runs the
same 750 ms settle a main-window resize does (`pop_out_moved`, then
`on_window_resized` with the main window), and so does opening one, which
may open where a large one closed. Upwards only, as everywhere, so a small
pop-out keeps what it played at home; bringing a pane back re-picks for its
cell straight away (`pop_in`), and a rise is a swap in place. A maximized
pane is measured by the whole body, the one cell `RootView::grid` has while
it lasts, so being given the page re-picks it upward at once
(`maximize_changed`). The panes maximized away are measured by the grid they
come back to, every pane's, and `sync_quality` leaves them until it does, so
the maximize never moves them; meanwhile they go on decoding at the size they
were last drawn at, which is the quality rule's price, not a saving anybody
asked for. Their cells can grow while they are away — a resize, a pane
closed — so the grid coming back chooses again, upwards only as ever:
`maximize_changed` does, and so does `restage`, once it is done, for a
maximize that ends there (a popped pane coming home, the maximized pane
closed), which it is called without a window for.

**A rendition change keeps the picture: the new stream is swapped in under
it.** When a pane whose picture covers it changes rendition — the re-pick
above, or a pick from its menu — `RootView::change_rendition` starts
streamlink again *beside* the stream on screen (`How::Beside`, kept as
`Slot::pending`) and leaves `Slot::supervisor` running. **Never kill a pane's
supervisor while its player is live:** the old mpv is reading that relay,
and with the relay gone it hits end of file, which `stream_stopped` reads as
the stream ending — it retires the pane and drops the new start with it. Every start is
numbered from one process-wide counter (`Slot::generation`), and both its
streamlink events and its player's frame wakes carry the number;
`video_view::route` sends each to the stream on screen, the one getting
ready, or nowhere, so a superseded start's last events cannot reach the pane.
A pending start's `Resolving` changes nothing — setting `Starting` would drop
the picture — and its failure leaves the pane as it was (`pending_event`).
When it resolves, the new player is started inside the same `VideoView`
(`VideoView::begin_swap`): silent, with the view's own `SizeHandle`,
publishing no position, and a recording `SWAP_LEAD` (4 s) ahead of a playing
pane or at a paused one's place. Pause, volume and seeks act on the player on
screen alone and are never forwarded; instead the pure, tested
`video_view::swap::align` holds the new player to the one on screen on every
wake of either and on a 100 ms tick.

What `align` does: a live stream takes over on its first frame, since two
low-latency sessions sit at different distances from the edge and cannot be
lined up. A recording is held still once it has a frame until the pane
reaches it, and takes over when it is at most 0.1 s ahead and 0.5 s behind;
fallen behind (it took longer to open than its lead), it is sent further
ahead each time; far further ahead than any lead it was given, the pane
jumped back, and it is sent just ahead of the pane again; with the pane
paused, it takes over within 1.5 s ahead and 0.5 s behind and is otherwise
sent to the pane's place. Those windows sit inside the chat replay's seek
slack (one second back, two forward), so the replay keeps its lines. After
three sends it takes over wherever it is, logged "unaligned", and a seek of
the user's starts the count over rather than giving up. A send is waited for
until the player has played on from its target (0.2 to 1 s past it) or for
3 s: its position says the target as soon as its render thread takes the
seek, between frames, while the picture is still the old one for the second a
reopen takes. And the new player is only ever paused
after it has a frame, since whether mpv draws a first frame for a file it
opens paused is unverified.

Taking over (`promote`) carries the pause and the level the pane is heard at
(`Loudness`, so `M` and Mute all's hold survive, and `--volume` does not come
back), and a seek of the user's the old player has not applied yet
(`VideoStream::take_seek`). That last is easy to have: `VideoStream::seek_to`
only fills a slot, which the render thread takes once a pass, and a paused
player's pass sits up to 200 ms waiting for a frame that never comes; a
stopped thread checks `stop` before it reaches the slot, so the arrow press
on a paused recording just before the hand-over would be lost. `align`
decided on the pane's place before that seek, so the new player takes over
there and then goes where the user asked. Then it publishes the new player's
position, swaps the stream and adopts its
frame pump, drops the old stream, and emits `VideoEvent::Swapped`; only then
does the root make the pending start the pane's (`on_swapped`), killing the
old streamlink after its player has been stopped. `first_frame` is left
alone, so nothing fades in again, `covers` stays true and no poster comes
back. `render` frees the old stream's tile, from the one window drawing the
player, when the first frame with the new id arrives. A swap whose stream
stops, whose channel closes, or that is not done in `SWAP_CEILING` (45 s)
leaves the old player playing and emits `SwapFailed`; the root drops the
start, and for a pick from the menu says so in a toast and hands the pane's
override back to what plays. A pick is seen to be under way from the press,
since the menu closes on it and the resolve alone takes seconds: with nothing
changing on screen, a working switch was taken for a broken one. Its start
carries what it asked for (`Restart::Pick`, a `video_view::Switching`: the
rendition, or for the settings' row what the settings pick now, or for a
stream whose renditions they cannot choose from their choice in a word that
fits the pill's room, `settings_view::quality_word`: `Auto`, `Best`), and
`RootView::set_pending`, the only write of `Slot::pending`, tells the player
(`VideoView::set_switching`, a mirror like `set_chat`); a test counts the
root's writes of the field. Not the view's own pending swap, which begins only
once streamlink has resolved, and which `begin_swap` drops and makes again.
Meanwhile the pill names the rendition picked and breathes
(`motion::waiting`), in the lifted `theme::text()` rather than its resting
colour, which at the bottom of a breath would fall to about 2.2:1 over a white
frame (a test in `controls` measures it), with a `Switching to 480p30` tooltip
under an id keyed on it; the menu marks the row the pick was made from,
breathing; More's folded row says `Quality · switching to 480p30`, and More
breathes while the pill is folded into it; and a pane's bar stays up with the
pointer gone, then for `theme::SWITCH_LINGER` (1.5 s) after the switch ends
either way, on a one-shot timer, so the new rendition or the old one back
beside the toast can be read. Not a pop-out's bar, which has no pill and no
More to say why it stayed up. The press itself hides nothing: it closes the
menu, which may let the bar go, and the switch holds it up again in the same
event, and `motion::Fade::set` takes a flip undone before any frame drew it
back to the id the last frame drew under, so the bar does not blink. The same
pick again (`RootView::pick`, `again`) leaves the start under way, and a
player it may have lined up, alone; the same rendition from the other row
moves only the menu's mark, and `on_swapped` marks the menu by the override as
it is then. A re-pick that supersedes a pick of the settings' row still
resolving, its pane grown meanwhile, stays a pick of what the settings want
now, counted from the press (`re_pick`), so the bar does not fall quiet as if
it had failed. Otherwise a re-pick is silent throughout. A re-pick that fails
waits for the next growth, with no retry loop. A re-pick that resolves to the
rendition already playing is dropped, and `sync_quality` leaves a pane alone
while a start for a rendition at least as tall resolves beside it
(`wants_swap`). Still cold: opening, `Try again`, the settings sheet's quality
and credential restarts (a swap per pane there would be two players and two
Twitch sessions per pane at once), and a pane whose picture does not cover it
yet. Each step is in the log under the pane's key — `starting 1080p60 beside
480p`, `holding 1080p60 at 616.40 for the pane at 613.90`, `reseeking`, and
`swapped 480p->1080p60 after 3930 ms; position 616.31->616.40`, with `, then
700.00` when it carried a seek and, for a pick, `(6120 ms since the pick)`
after the first count, which is the wait somebody sat through, streamlink's
resolve included — and `SWAP_LEAD`, `SWAP_CEILING` and `MAX_RESEEKS`, all
three estimates, are to be tuned from it. `SWAP_LEAD` has only the new
player's open to cover, since the pane's position is read once streamlink has
resolved (`pending_event`): a swap's first `holding` line says what was left
of the lead when the new player had a picture (its position less the pane's,
2.5 s in the example), and a first `reseeking` from behind the pane says the
lead was not enough. Not the `swapped` line's time: it runs from the new
player's start to the hand-over, so on a playing recording it comes out at
about `SWAP_LEAD` whenever the lead was enough, however quickly the player
opened. Not yet known: whether Twitch serves two live sessions on one token
(if it does not, a live swap fails and the pane keeps what it plays, and live
panes would go back to cold restarts), and what the audio does at the moment
of taking over, which cannot be captured.

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

### Recordings

A past broadcast is the one source the app positions itself, and the reason is
in the player, not the app.

**ffmpeg's HLS demuxer cannot seek a fragmented-MP4 playlist, in the libmpv
builds people have.** Twitch keeps two kinds of recording: transport-stream
playlists (`.ts` segments, `forsen` for one) and, for every channel on its
newer encoding path — most large ones — fragmented MP4 (`#EXT-X-MAP` with
`init-0.mp4`, then `.mp4` segments). Seeking the first works. Seeking the
second leaves mpv in `seeking=yes` for good with no packets delivered, and
`seek` in any precision, `start=`, hardware decode on or off and the cache on
or off all measured the same on the mpv.net build (mpv 0.37, early 2024). What
*does* work is a playlist whose first segment holds the target, with a `start`
inside that first segment. So `vod.rs` positions a recording by rewriting:
`streamlink::playlist` reads the playlist once, on the resolve worker, and it
arrives in `StreamEvent::Ready`; each seek writes a playlist that starts at the
segment holding the target, with absolute URLs and the init segment, and
reloads the player on it — `Player::load`, which is `loadfile <file> replace
start=<within>`. The player, its render context and its atlas tile all stay. A
reposition lands in about a second, which is what a native seek cost on the
`.ts` kind, so both kinds go the same way on purpose: one path, one set of
tests.

Three things that took finding:

- **mpv reads a local `.m3u8` as one of its own playlists** — a list of files
  to play one after another — and never hands it to the HLS demuxer, so the
  recording arrived as thousands of ten-second entries and the first ended with
  reason `Redirect`. `demuxer=lavf` forces ffmpeg's demuxer. And ffmpeg refuses
  to follow a local file to the network unless told: `protocol_whitelist` on
  `demuxer-lavf-o`, whose value has commas in it inside an option that is a
  comma-separated list, so it is quoted with mpv's `%N%` length prefix.
  `Recording::mpv_options` builds both.
- **The demuxer keeps the playlist file open, and Windows will not rename over
  an open file.** Every reposition writes a new file,
  `<video>-<quality>-<player>-<n>.m3u8` under the temp directory, and deletes
  the previous one once `FileLoaded` says the new one is in; anything still
  held is retried at the next reposition and at teardown. `<player>` is a
  number no other player of the run has (`vod::playlist_label`): a player
  deletes what it wrote when it tears down, and two players of one recording
  under one name — a pane restarted at the rendition it had, whose old
  player's thread is still letting go, or a second player started beside the
  first — deleted and rewrote each other's files. Nothing reads a label back
  but `sweep_scratch`, which clears the directory whole.
- **A broadcast still being recorded is a playlist with no end marker that grows
  by a segment every ten seconds.** ffmpeg re-reads such a playlist when it
  runs out of segments, and it re-reads a *file* too. So the keeper thread
  fetches the playlist once per segment and *appends* what is new to the file
  the player is reading — appending never disturbs what the demuxer has read —
  and appends `#EXT-X-ENDLIST` when Twitch does, after which the player reaches
  the end the ordinary way and the pane says "Finished". `live_start_index=0`
  keeps the demuxer from starting three segments from the end of a playlist
  with no end marker, which is its default for anything live. And the
  rewritten playlist says `#EXT-X-PLAYLIST-TYPE:EVENT` while it grows, as
  Twitch's does: ffmpeg treats an unfinished playlist as unseekable unless it
  is an EVENT one, and an unseekable playlist silently drops the start offset,
  so every jump landed on a ten-second boundary until the tag went in.

A reposition is known to have landed by mpv's `path` property changing to the
new file — not by the first `MPV_EVENT_FILE_LOADED`, which with two quick
repositions can belong to a file that has already been replaced, and would
have positions read against the wrong base for a moment.

The position the bar shows is `base + time-pos`, where `base` is the seconds
before the current file's first segment and `time-pos` is per file, rebased to
zero by mpv. Between a reload and `FileLoaded`, `time-pos` still belongs to the
old file and is ignored; the target is stored the moment the seek is asked for,
so the bar does not sit on the old position while the player reopens. mpv's
`duration` is not read at all — it is the length of the file being read. The
length comes from the playlist (`vod::Extent`), grown by the keeper, and the
bar's extent adds the seconds since the keeper last said, so a growing
recording's end moves smoothly rather than in ten-second steps.

**A player reports its position to itself, and to the pane only while it
publishes.** Every position a player learns — where it opens, `time-pos`,
where a seek sends it — goes through `video::Positions::report`, which always
writes the player's own handle and writes the pane's shared one (what the
chat replay, the history and a link to the moment read) only once the player
publishes. The render thread reads its own: reopening after a long pause and
the stall watchdog are about this player's place in its file.
`VideoStream::position` answers with it too, and so does the seek bar, which
asks the player on screen. A pane's one player publishes from the start
(`StartOptions::publish`, as `start_stream` asks), so nothing changes for it;
the gate is for a second player of the pane started beside the one on screen,
whose position moving would otherwise reset the replay to a moment nobody is
watching and note that moment in the history. The quality swap starts one
(see "A rendition change keeps the picture" under Video), and publishes it
as it takes over (`VideoStream::publish_position`). `Positions::publish` hands the pane
where the player is at that moment, not where it next reports. Such a player
is also handed the pane's `SizeHandle` (`StartOptions::size`): a stream keeps
the handle it is given rather than a copy of the size, so two players of one
pane follow one probe and the second renders at the pane's size from its
first frame.

The replaced file's end arrives as `MPV_EVENT_END_FILE` with reason `Stop`,
which `Stopped::from_end` already ignores — the same reason a closing pane
sees. `Eof` is the only end that finishes a recording, and the pane reads it
as "Finished" rather than "ended the stream" by looking at what it was playing
(`watch::showing`, which the mini player's tile word reads too).

`mpv-frames` deliberately has no `seek`. `seek_to_live` is the percent seek for
the live edge; an absolute one was added for recordings and removed when the
measurement above came in, because a command nothing can use is a trap.

**A recording paused for a while can come back to a dead connection, and
hang.** The main file is local, so mpv runs no network cache: the demuxer reads
about a second ahead (`demuxer-cache-duration` sits at 1.0) and keeps its
connection to the CDN between segments. A pause therefore leaves a connection
idle, usually halfway through a ten-megabyte segment. In a five-minute pause
against the real CDN the connection was closed while it waited, cleanly —
ffmpeg's keep-alive request failed at once and it opened another, and play
went on. Dropped *silently* instead — a router, a VPN or a sleep forgetting the
flow, simulated with a local proxy that stopped answering on the connections it
already had — the read simply waits: playback resumed for the two seconds
already buffered, then sat at `paused-for-cache` for the two minutes it was
watched. mpv could not apply its sixty-second network timeout to the demuxer
("Could not set AVOption timeout" in its log); some timeout did fire a minute
in, and only moved the demuxer on to asking for the next segment down the same
dead connection. The fix is the user's own workaround, automated: a reopen at
the same position is fresh connections and lands in about a second. So a pause
of `vod::LONG_PAUSE` or more resumes by reopening where it is, and a recording
that is not paused and has not moved for `vod::STALL` is reopened where it is
too — except within `vod::EDGE` of a broadcast still being recorded, where
waiting is the point. Both were measured against that proxy — the resume
reopened in 1.4 s, and in the app a stall mid-play was reopened by the
watchdog and carried on — and neither fired in minutes of ordinary playing,
pausing and background play. Both say so in the log.

**Rewinding a live pane opens its broadcast's recording, still being
made.** A live pane's bar draws a timeline above its buttons, from the
broadcast's `started_at` to now, with the recording's own seek bar
(`seek_bar::element`, its scrub and its hover label); a press let go back
along it asks the root to go to that moment (`PaneAction::Rewind`), which
finds the broadcast's archive and replaces the pane with it at that
position (`replace_with_video`, as `Watch from the start` does), so from
then on the pane is an ordinary recording. The rules are `rewind.rs`, pure
and tested; the root's side is `root/rewind.rs`; the drawing is
`video_view::bar::live_row`. What took deciding:

- **The press is a moment on the wall clock, not seconds into the
  archive.** The broadcast starts at `started_at` and its archive at its
  own `created_at`, seconds later, or hours later after a reconnect split
  the broadcast and the archive holds only the last part. The moment is
  what both agree on: the position is the moment less the archive's start,
  clamped to its start and to half a minute short of how long it has been
  going (`rewind::position_in`). A moment before a split archive began
  opens at its start.
- **The archive is found by `channel_page::archive_of`, asked about now.**
  An archive that started by now and is listed as going on until within
  `STILL_GOING_SECS` of now, the broadcast's id choosing among them
  (`Slot::broadcast`, read off the live list when the picture arrived), and
  failing that, one wearing Twitch's placeholder picture, which says it is
  being made now however stale its listed length (`rewind::archive_for`).
  None is no answer, never an older broadcast's.
- **Trap: after a restart, the broadcast before is still "going on".** A
  broadcast that ended at 18:00 is listed as going on until within
  `STILL_GOING_SECS` of now, and may still wear the placeholder picture, so
  at 18:06, three minutes into a restart and before Helix lists the new
  archive, `archive_of` alone picks the old one as the newest candidate, and
  a stale `Slot::broadcast` names it outright. It would open the wrong
  recording at its end, and be kept. So `archive_for` first drops every
  archive whose listed end (`created_at` plus its length) falls more than
  `BEGUN_SLACK_SECS` before the broadcast's start, which the press carries
  (`rewind::Moment::since`, the start the player's timeline was drawn from);
  then it asks the rest. A broadcast that restarts while the pane plays on
  moves that start without the pane playing again, so the archive in hand
  is kept with the start it was found for, and a press on any other start
  asks afresh (`Rewind::press`).
- **Looked up on the first press, never polled.** The ask is the stopped
  pane's own `Request::Broadcasts` (`request_broadcasts`), sent only when a
  press finds no archive in hand; the press waits on it in `Slot::rewind`
  and is carried out when the answer comes, a later press taking its place.
  A found archive is kept for the broadcast it was found for, and the
  slot's `Rewind` is made afresh whenever the pane begins to play. Not
  finding one is not kept: Twitch may list it a minute later, so the next
  press asks again. Signed out there is no token to ask with, and a toast
  says `Sign in to rewind`; no archive says `No past broadcast to rewind
  into`, and a failed ask says it could not look, with the reason in the
  log. The pane stays live through all three.
- **Both asks share one answer type, routed by order.** The worker answers
  `Broadcasts` by login alone, one request after another, so in the order it
  was asked, and always exactly once. A pane asks for its rewind only while
  it plays and for itself only once it has stopped, so a pane waiting on
  both — one that stopped with a rewind's ask out — asked for the rewind
  first. `on_broadcasts` therefore gives an answer to a pane whose rewind is
  asking before one whose `archives` is waiting, and a rewind's answer for a
  pane no longer playing carries nothing out. That leaves the asks a pane
  forgets when it begins to play again: a stopped pane asks, plays again
  before the answer, then a press asks for its rewind, and the first answer
  to come is the stale one, a list from before the new broadcast. So
  beginning to play counts each ask it forgets (`Slot::stray_answers`), and
  `on_broadcasts` lets that many answers for the channel go by before
  routing; `forget_asks` puts the count back to none with the worker that
  owed them.
- **An archive kept since an earlier press is listed shorter than it is.**
  Opened as listed, `channel_page::in_progress` would read it as finished
  once its listed end was ten minutes behind, and the history would note it
  that way. So it opens as long as the time since it started
  (`rewind::as_of`); the player's own extent comes from the playlist anyway.
- **When the broadcast began reaches the player as a mirror**,
  `VideoView::live_since`, on `ChatButton`'s pattern: `Start::live_since`
  when the player is made, and `set_live_since` from
  `RootView::sync_live_since` after every answer from the worker, since any
  list may be the first to say it. A list that loses the channel takes
  nothing away. The timeline is offered (`rewind::span`) only past
  `rewind::EDGE_SECS` of uptime and for a start no more than 48 hours back,
  Twitch's longest stream, which is what a snapshot from an older broadcast
  would fail. A press within `EDGE_SECS` of the live edge does nothing: the
  archive runs about that far behind live.
- **It grows with the bar's own redraws.** A live picture redraws its view
  at its frame rate, and the timeline reads the clock as it draws; nothing
  ticks for it. A paused live pane's timeline moves whenever anything else
  redraws the window — chat, the pointer — since `render` runs on every
  draw.
- **It takes a row, not room on the buttons'.** `bar::fit` gives nothing
  up for it; `bar::timeline_fits` leaves it off a pane too narrow for the
  two times and a 96px track, which the smallest pop-out still clears.

**And `LIVE` on the recording goes back to the live edge.** A pane playing
the archive of a broadcast that is still going on — rewound into, or opened
from a channel's page or the history — has a `LIVE` pill at the right-hand
end of its seek row, where the live timeline says `LIVE`
(`video_view::bar::back_to_live`). A press replaces the pane with the
channel, cold, as opening a channel is (`PaneAction::BackToLive`,
`RootView::back_to_live`, `replace_with_channel`), so it lands at the live
edge with the live chat. What took deciding:

- **Still live is read off the lists, never asked.** The channel is in a
  live list (`stream_info`: the follows, popular, a category, a search),
  and the recording is the archive of the broadcast that entry describes
  (`rewind::live_now`): an archive, not finished as listed before the
  entry's `started_at` (the same `going_at` test `archive_for` rules the
  broadcast before a restart out with), from a start no more than 48 hours
  back, and with the entry's stream id where both carry one. The pane's
  copy of the archive is the one it opened with, its length maybe hours
  stale, which does not matter: a length only grows, so a listed end at or
  after the start is a real one. The entry's id, unlike `Slot::broadcast`
  in a rewind, is as fresh as its start, so it is trusted to tell a
  snapshot of an earlier broadcast from the one on now.
- **Not every list is fresh, so three more things are weighed**
  (`rewind::back_to_live`, from `RootView::listed` and `Slot::origin`).
  Popular, a category and a search are fetched once and kept, and their
  entry for a broadcast that ended hours ago passes `live_now` on the time
  rule and the id rule alike. So a followed channel the follows poll has
  among the offline follows is off, whatever those lists say. A recording
  opened by `Watch from the start` or `Watch here` carries the id of the
  broadcast its stopped pane saw end (`Slot::seen_ended`,
  `Origin::Ended`), and a list a poll behind that still names that id
  offers nothing. And a recording a rewind opened carries the start of the
  broadcast it came from (`Origin::Rewound`), which stands in when no list
  carries the channel any more — a live pane opened from a category keeps
  its timeline after another category replaces that list, and should keep
  its way back too.
- **And the player's own word: the archive is still growing.** The bar
  draws the pill only while `seek_bar::Timeline::growing`, which the vod
  keeper turns false once Twitch writes `#EXT-X-ENDLIST` into the archive's
  playlist. That takes the pill off a finished archive whichever list is
  stale; it costs the first part of a broadcast a reconnect split, whose
  playlist is finished while the stream is on.
- **A mirror on the player, kept up after every answer.**
  `VideoView::back_to_live`, written by `Start::back_to_live` and by
  `set_back_to_live` from `RootView::sync_back_to_live`, which runs beside
  `sync_live_since` after every answer from the worker. Unlike the start, a
  list that loses the channel takes it away, so the pill goes at the next
  follows poll after a followed channel ends. A press is not checked
  again: one that lands after the stream ended, before the lists or the
  playlist caught up, opens a pane that says the channel is off, which is
  the news.
- **In a pop-out, the pane comes home first,** as Bring back brings it
  (`replace_slot` does what `pop_in` does): the pop-out finds its player
  by the pane's key, which the new slot does not carry. So `LIVE`, or a
  rewind, pressed in a pop-out closes that window and the pane plays on in
  the main window, which comes up on the watch page if it was browsing
  with the mini player off — there the pane would come home to nothing
  drawing it, and `restage`'s deferred `retire_homeless` would stop the new
  slot, whose key is not popped. The new pane is chosen (`choose`), so
  while another pane is maximized this one takes the watch page rather
  than playing where nobody sees it.
- **The recording's place goes into the history first,** as a close writes
  it: `replace_slot`, the swap both replacements share, notes every
  recording pane before it drops one. If the channel is already live in
  another pane, that pane is chosen and the recording stays, as
  `replace_with_video` does for a recording already open.
- **No return to live by itself at the end of the archive.** A rewound
  recording that catches up with what Twitch has written waits there, as
  any recording still being made does near its end (`vod::EDGE`, where the
  stall watchdog stands aside), until the playlist grows. It does not jump
  back to live: the pane would change under somebody who paused to read
  chat, and the end of an archive still being made is a few seconds of
  buffering, not an end. `LIVE` is the way back.
- **Room from the seek track, not the buttons.** The pill sits in the seek
  row, so `bar::fit` gives nothing up for it and the pop-out's bar, which
  has the seek row, has it too; the track reports its laid-out bounds, so
  a scrub still lands under the pointer.

**Why not streamlink's relay for a recording.** Its server answers one GET with
a body that never ends, ignores `Range` and sends no `Content-Length`, so a
seek would be a process restart. And the `--stream-url` rule is about ad
filtering in *live* playlists: a recording's playlist has no stitched ads —
checked tag by tag across 2,150 segments and four more samples. Without the
auth-token cookie, Twitch's playback token caps a recording at 1080p
(`AUTHZ_NOT_LOGGED_IN` for the 1440p and 4K tiers) and refuses
subscriber-only ones; the cookie the settings already hold lifts both, and did.

A recording named by a link — `twitch.tv/videos/<id>`, on the command line or
pasted into the palette — is read by `target.rs` and looked up through
`Request::Video`, since the pane needs the video's title and channel and the
link carries neither; it opens from the link's `?t=`, if it has one — or else
where the history says it was left — and a link given before sign-in waits in
`RootView::linked_videos` for the session.

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

**streamlink does not exit when the stream ends.**
`--player-external-http-continuous` defaults to `yes`, so when a broadcast
finishes streamlink closes the HTTP response and goes back to waiting for the
next request — forever, holding a process and a port. Its supervisor sends
nothing, because nothing happened to it. The only signal that the stream is over
is mpv's `MPV_EVENT_END_FILE`, which is why `mpv-frames` decodes that event's
`reason` and `video.rs` publishes it: without that the pane kept its last frame
on screen and a finished stream was indistinguishable from a paused one.
`RootView::stream_stopped` drops the supervisor, which is what kills the
process that is still waiting.

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

**The settings sheet saves only what it owns.** The sheet used to work on a
copy of the whole of `Settings` taken when it opened, and its `Saved` replaced
the root's settings with that copy wholesale — so whatever the app wrote while
it was up went back. Less can land under the sheet than it looks: its scrim
covers the rail, the title bar's veil covers the rail button, and the sheet's
key context keeps `B`, the volume keys and `Ctrl+K` off. The palette is drawn
under the sheet and used to open there, where a channel watched from it wrote
`recent` and its rail row folded the rail; see "Keyboard". What still gets
through is `recent`, from a handed-over launch. Pins, volumes, the rail and
the window's placement cannot change while the sheet is up — the window is
written only on the way out — but they are why the sheet owns a list of
fields rather than the root keeping a list of exceptions: whatever the app
writes outside the sheet, now or later, has to survive saving it. The sheet is
handed `settings::SheetFields` — the client id, the auth-token cookie,
quality, chat history and the mini player — and hands the same type back, so
it never holds the rest to hand back; `Settings::adopt_sheet` takes those
five and nothing else, and `adopting_the_sheet_keeps_everything_it_does_not_own`
holds it to exactly that, pins, volumes and placement included. A field the
sheet gains does not compile until both ends handle it: the panel builds the
type with a struct literal, and `adopt_sheet` takes it apart with no `..`.
The sign-in is kept apart from all of this by `save_preferences` (above).

**An older build drops what it does not know.** Every save writes the fields
the running build has and nothing else: serde ignores an unknown field on load,
so it is never written back. Run a build from before `pinned` existed after
this one — `instance` stops two copies at once, not one after the other — and
its first save writes the file without the pins. Nothing in this build can
prevent that, and a field added later costs the same against this one. Keeping
fields a build does not know would take a `#[serde(flatten)]` catch-all on
`Settings`, which nothing has needed yet.

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
  occluded individually for that reason, and the mini player carries
  `block_mouse_except_scroll` on its own box and nothing larger. It floats over
  the browse grid, which is the arrangement that failed twice before: a strip
  of 220px thumbnails in the bottom-right corner covered two cards at 1000px
  and let clicks through to them, and the docked now-playing bar that replaced
  it never had room for more than a 96px muted thumbnail. Floating works now
  for two reasons together — the box blocks the pointer from what is under it,
  and every browse list leaves `layout::mini_reserve` at its foot through
  `browse::scroller`, so its last row can always be scrolled out from under
  the player. The player is a fixed `MINI_PLAYER_WIDTH` and grows by rows, so
  four streams make it taller rather than wider. It stands in from the page's
  right edge by `layout::mini_player_right`, past the list's scrollbar, so
  the whole track stays the list's to drag; gpui-component keeps the track's
  width private, so `theme::SCROLLBAR_WIDTH` copies it, and a test fails when
  `Cargo.lock` moves the library on. And it is placed against the page's own
  column rather than the row the rail is in, so in a narrow window it is cut
  off at its left rather than reaching over the rail's last rows, which have
  no room at their foot to be scrolled out from under it. At a narrow window
  it still covers part of the visible grid until that is scrolled.
- **`block_mouse_except_scroll` for overlays on the browse page**, so the wheel
  still reaches the grid underneath; plain `occlude` for the modal, where the
  page behind should not scroll either.

`browse.rs`'s `cx.stop_propagation()` on "+ Add" is *not* the same pattern and
must stay as it is: that button is a descendant of the card, not an overlay, and
occluding it would kill the card's own hover and the group-hover that reveals it.

Hover probes are immune to all of this: `gpui::canvas` inserts no hitbox and
reads `window.mouse_position()` directly, so the video and pane probes keep
working through any occluder.

**An invisible element still blocks the pointer if it occludes.** `invisible()`
returns from paint before any listener is registered (div.rs:1809), but the
hitbox went in during prepaint (div.rs:1676-1680), `BlockMouse` and all. So a
faded-out overlay that occludes goes on swallowing presses where nothing is
drawn. The player's control bar occludes only while its fade
`is_visible()`. A real pointer over the picture always has the bar up, so this
mattered to synthetic input and to a press in the fade's last moments, but the
next overlay may not be so lucky.

**An occluder hides presses from the root's `track_focus`**, which is
hover-gated like any mouse-down listener (div.rs:2025-2037), so whatever
occludes over the watch page has to hand the keys back itself, or a cursor
left in the search box keeps them and `Space` types a space. The control bar
and the player's menus do it with `capture_any_mouse_down`
(`VideoView::return_keys`), in the capture phase on purpose: gpui-component's
`Slider` thumb calls `stop_propagation` on its press
(gpui-component-0.5.1 slider.rs:464-466), so a bubble-phase handler on the bar
never hears a drag of the volume.

**A menu's rows act on the press, never on a click.** A menu dismisses itself
with `on_mouse_down_out` on its anchor, which fires in the capture phase when
the press is outside the anchor's *own* bounds (div.rs:226-236), and a menu
hanging above its button is outside them. So a press on a row closes the menu
first. gpui synthesises a click from the press and the release, and fires it
from the listeners of the frame current at the release (div.rs:2213-2245),
which, a frame or more after the press, no longer has the row in it. The
quality menu's rows were `on_click` until phase two, so a pick could only
take if the release came before the next frame (read from source, not
reproduced). The press, though, is dispatched against the frame it landed
on, and its bubble phase still finds the row, so `menu::menu_row` acts in
`on_mouse_down`. A test reads `menu.rs` and fails on an `on_click` in it.
The box itself occludes, since it rises out of the bar's own hitbox, or a
press on a row would also reach the picture's double-click.

**The second press of a double-click lands on whatever is there by then.**
The platform counts a press near the last one, soon enough, as the next of
a run, by time and place alone (windows/window.rs:1049-1060), and every
press is hit-tested against the frame on screen. A row acts on the first
press and its menu is gone by the second, which then reaches what was under
the row: the picture, whose double-click is fullscreen; a recording's seek
track, which seeks; or the quality menu that More's quality row has just
opened in its place, which would pick a rendition and change the stream.
So a row acts only on a run's first press and marks the run
(`VideoView::row_run`), and `menu::run_guard`, a window-level listener held
ahead of the bar, stops every later press of that run in the capture phase,
before anything else hears it, which also leaves its release no click to
make. The next run's first press clears the mark. A pane's press making it
active guards against the same platform rule (`watch::pane`).

That guard is the player's, and is drawn only while the main window draws
the player. More's `Pop out` takes the player into a window of its own on
the first press, guard and all, and the second press of a double-click
then reached the cell under the closed menu: its `Bring back`, which sent
the pane straight back, or its header's icons. The count carries on in the
main window even once the pop-out has taken activation. The mini player's
pop-out controls do the same to their tile, or to the whole player, and
leave a card or another tile under the pointer. So each of those takes the
run for the root (`RootView::take_rest_of_run`), whose own window-level
listener, held ahead of everything the root draws, swallows the rest of it
by the same rule (`RootView::run_guard`). The bar's maximize control marks
the run on the player's own guard as a row does (`row_run`): the pane grows,
or the grid comes back, under the pointer, and the second press would land
on a picture, whose double-click is fullscreen. A control that stays put and
only toggles — the header's pop-out icon turning into Bring back — takes no
run, like every other toggle: a double-click there is two presses of it.

**A tooltip keeps the words it came up with.** gpui builds the tooltip's view
once, when its delay runs out, keeps it in the element's state, and sets that
same view on the window every frame the pointer stays (div.rs:1662-1664,
2279-2316); the builder a later frame hands over is not asked again. So a
control whose words follow a state it changes keys its element id on that
state too — the bar's `bar-pause`/`bar-play`, `bar-mute`/`bar-unmute`, the
chat and fullscreen glyphs, the mini player's `mini-mute-all`/
`mini-unmute-all`, the rail's pin and the gear signed in or not — and the
first frame drawn under the other id drops the open tooltip with the rest
of the old element's state. The next to come up, on the next move, has the
new words. Mute all went on saying "Mute all" after a press until the
pointer left, which is how this was found. `controls::tip` is the one
builder for a tooltip of plain words: `.tooltip()` takes a builder, not
text, and only once per element (div.rs:536-549).

**A tooltip outlives a pointer that leaves the window**, for the reason the
caption buttons stay lit: it checks it is still wanted against the last
position gpui saw, which a pointer leaving never updates. Taking the builder
away clears it in prepaint (div.rs:1660-1668), so the player's bar gives its
tooltips only while `window.is_window_hovered()` (`bar::bar_icon`), and a
pane's header does the same (`watch::header::pane_header`, handed the flag
by `watch::page`): the right-hand pane's × sits `ROW_PAD_X` from the
window's edge, and over the picture an invisible band is still prepainted,
so its tooltip would outlast the band itself. The title bar and the mini
player sit on the window's edge as well and do not gate theirs yet.

**The clipboard is the app's, not a window's.** Writing it is
`cx.write_to_clipboard(gpui::ClipboardItem::new_string(..))` (app.rs:1041),
which needs no focus and no window, and opening a link in the browser is
`cx.open_url` (app.rs:1078). Perch writes the clipboard in one place,
`RootView::on_pane_action`'s `CopyLink`, followed by the toast that says it
did. Each platform has its own `write_to_clipboard` under gpui's; the macOS
one is compiled by CI's Mac leg and has never been run for Perch.

**`gpui_component::init(cx)` must run before any widget**, and `Root::new` must
wrap the window's first view or overlays have nowhere to render.

**The title bar is Perch's own, and the platform's behaviour has to be earned
back piece by piece.** `TitlebarOptions::appears_transparent` hides the
platform's bar; `root/title_bar.rs` draws the replacement. Every one of these
fails silently, and each was read out of gpui 0.2.2's source:

- **The bar must `.occlude()`.** The root's `track_focus` installs a
  mouse-down handler that calls `prevent_default` on every press it hears
  (div.rs:2025-2037), and the Windows backend reports a non-client press whose
  default was prevented as handled (events.rs:976) — so `DefWindowProc` never
  sees it. Without the occluder a drag, a double-click to maximise and the top
  resize edge all do nothing.
- **`WindowControlArea::Drag` goes only on an empty spacer** that is a sibling
  of every control, never an ancestor. The hit test answers with the first
  window-control hitbox under the pointer in paint order, parent before child
  (window.rs:1138-1141), so a drag area around the controls answers for them.
- **The drag and caption areas start `layout::drag_top` down.** gpui asks for
  a window control before it asks Windows about the frame, so an area reaching
  y=0 takes the top resize edge with it. When maximised `drag_top` is 0: gpui
  skips its `HTTOP` band there (events.rs:918), and a pointer flung to the top
  of the screen has to land on the bar. A caption button lights that area and
  nothing above it. Lit to the top, the strip over it turned Close red under a
  resize cursor, and a press there — the start of a resize, whose release the
  platform's size loop keeps — left the button looking pressed for having
  done nothing (`controls::caption_button`; read from source, not watched).
- **Caption buttons carry no handlers.** No `on_click`, no `stop_propagation`,
  no focus: anything that handles the press stops the platform acting on it,
  by the same events.rs:976. And never reach for `zoom_window`, which
  maximises but never restores (windows/window.rs:790-798), or
  `remove_window`, which skips `on_window_should_close` and so never saves the
  window's place.
- **Caption buttons light only while `window.is_window_hovered()`.** gpui
  hears the pointer leave the window as a flag and a repaint, never a move
  (events.rs:318-327), so the next frame hit-tests the last position it saw.
  The buttons sit in the corner the pointer usually leaves by, so Close would
  stay red with the pointer off the window. A press is worse: a press on a
  caption button is non-client, gpui captures nothing for it
  (events.rs:986-997), and one dragged off the window and let go outside never
  comes back as a release, so `.active` would hold until the next release
  anywhere in the window. `caption_button` drops its hover and press styles while the
  window is not hovered, and its id follows the flag, so the first frame drawn
  then forgets the press with the rest of the element's state. What is left:
  the flag is one bit for the whole window, and if Windows sends the page's
  leave after the first move over a caption button, a pointer that crosses
  onto one in a single move and stops leaves it dark until it moves again.
  Read from source; not yet watched with the real cursor.
- **Nothing that takes a press of its own sits in a drag area**, the search
  box least of all: a press in a drag area is the platform's, and the moves
  after it arrive as non-client moves with no button held (events.rs:938-942),
  so drag-selecting the box's text would break. The box and the rail button
  are siblings of the spacer, at its left, and being a sibling is not enough
  on its own. Their group shrinks with the bar, and below the button plus
  `SEARCH_MIN_WIDTH` the box ran on past the group under the spacer. The
  spacer is painted later, so its drag area answered the platform's hit test
  there, which broke drag-selecting, and in a narrow enough window the box
  covered the whole strip and then the minimise button, where a press only
  focused the box. The group is `overflow_hidden`, which clips hit testing as
  well as painting (window.rs:779), so the box is cut off at the strip
  instead. While the sheet or the palette is up the group sits under an
  `occlude`d veil (`title_bar_leading`). The veil stops only the pointer. The
  keyboard is kept off the box because `toggle_settings` takes focus back to
  the root when the sheet opens. `Ctrl+,` stands aside for no text box, so
  without that it opened the sheet with the cursor still in the box: `Enter`
  ran a search behind the sheet, which on the watch page left the page, and
  `Esc` could not close the sheet. Between the veil and the focus, nothing in
  the bar changes the page behind a modal. The gear, the drag strip and the
  caption buttons stay live.
- **Modals, toasts and the mini player live in the root's id-less content
  wrapper** — the player inside the page's column there — under the bar, so
  their `inset_0` starts below it by construction, and the wrapper is
  `overflow_hidden`, so nothing in it can grow back over the bar. That
  matters because `block_mouse_except_scroll` does not take a drag area or a
  caption button out of the hit test: gpui goes on collecting hitbox ids past
  it (window.rs:775-796) and answers the platform with the first
  window-control area among them (window.rs:1138-1141).
  A toast laid over the spacer would drag the window; and the mini player,
  anchored to the bottom and growing upward, reached the bar in a window under
  about 280px tall, where pressing a picture moved, maximised or closed the
  window. The clip bounds hit testing as well as painting — each hitbox is
  intersected with its content mask (window.rs:779) — and adds no hitbox of
  its own. Tooltips and a text box's menu are drawn by the window after the
  tree, so they still reach past it.
- **A drag that must survive the pointer leaving its element listens on the
  window.** An element's `on_mouse_move`/`on_mouse_up` hear only the pointer
  over its own unblocked hitbox (div.rs:173-187, 263-273), so the root's went
  deaf over the occluded bar. The divider drag is followed by
  `RootView::drag_listeners`, a hitbox-less `canvas` that registers
  `window.on_mouse_event` as it paints; gpui captures the mouse on a press
  (events.rs:451), so the window hears the whole drag. The mouse's back and
  forward buttons are heard the same way (`RootView::side_buttons`) for the
  same reason: over the bar, a toast, the mini player or a pane's control
  bar, the root's own handlers hear nothing.
- **An `svg` paints only with its own `text_color`** (svg.rs:110) and inherits
  none, so a glyph never given one draws nothing. `controls::icon_button`,
  `caption_button` and `fold` colour the glyph themselves and lift it with
  `group_hover`.
- **macOS drags only by AppKit's native strip.** `window_control_area` is a
  no-op there in 0.2.2, so the spacer asks for `titlebar_double_click` by
  hand — through `on_click`, which needs an id, so the spacer's `.id()` is
  unconditional: adding one inside `.when(cfg!(..))` changes the element's
  type halfway through the chain and does not compile.

**A pop-out is a second window, and a player may be drawn by only one window
at a time.** A pane popped out (`root/pop_out.rs`) is the same `VideoView` in
another gpui window, and gpui gives every window its own sprite atlas, its own
focus and dispatch tree, and its own frame-to-frame element state. Every one
of these was read out of gpui 0.2.2's source; the spike that put them to the
test together, what its live checks found and what is still to be checked,
is recorded in `OVERHAUL-DECISIONS.md`:

- **One window per `VideoView`.** `render` skips `update_image` for a frame
  it already holds, so a second window drawing the view would freeze on its
  first tile, and two probes would fight over the render size. The main window
  reaches a player only through `RootView::video_in_main`, which is none while
  the pane is popped, and the pop-out only through `pop_out_video`, which is
  some only then; both ask the same `Stage`. A debug build asserts it in
  `VideoView::render` (`drawn_in`). Reading a player while drawing counts
  too: a window that reads an entity in a frame is redrawn whenever it
  notifies (app.rs:2034-2056), so the main window must not so much as ask a
  popped player whether it is paused, or it redraws at the video's rate.
- **`VideoView::set_place` is the only move.** It frees the tile in every
  window, deferred to the same flush, and notifies, which dirties every window
  that drew the view last frame — so none re-presents a scene naming a freed
  tile (a dedicated texture panics, directx_atlas.rs:299-305), and the window
  the view returns to uploads the current frame rather than painting its own
  stale tile. `RootView::restage` is its only caller.
- **A player belongs to no window.** Its frame pump is `cx.spawn`, not
  `spawn_in(window)`, and its release is `on_release` with an all-window
  `drop_image`, not `on_release_in`.
- **Pop-outs open and close only through `cx.defer`.** `open_window` draws
  before it returns (app.rs:943-978), and the pop-out's render reads the root,
  which is leased inside a root update; and a window cannot update itself
  re-entrantly (app.rs:1361-1386). Nothing in a pop-out calls a root method
  directly: `pop_out::to_root` defers, then goes through the main window, so
  root methods always get the main window's `Window` — quality is measured
  against the main window's body, and a restart's pump binds to it. What
  only the pop-out's own window can say, how tall it draws the picture at
  its own scale, it measures itself and hands over as a value
  (`PoppedOut::height`), never by the root reaching into its window.
- **The pop-out is dragged by its picture**, `controls::drag_layer`: an
  occluding layer from `layout::drag_top` down, with no press listener, under
  the bar, which occludes only while it is up. The same rules as the title
  bar's strip, above, for the same reasons. Its one listener is an `on_hover`
  that wakes a repaint: under it, nothing else hears the pointer arrive, and a
  paused picture sends no frames. A move on the caption is a non-client move,
  which gpui still hears as hover (events.rs:925-927). Crossing between the
  picture and the bar, either way, is the one place that may show: the
  picture is caption and the bar is client, and Windows sends a leave
  whenever the pointer crosses from one to the other — `WM_NCMOUSELEAVE`
  leaving the caption, `WM_MOUSELEAVE` leaving the client area. Both clear
  the window's hovered flag (events.rs:58, 318-327) before the next move
  sets it again, and the bar follows `is_window_hovered` (`VideoView`'s
  probe), so it could blink for a frame. Read from source, not yet seen; if
  it is, hold the bar up for `MOTION_HOVER` after the pointer was last
  inside the player, in the pop-out alone.
- **Neither `WindowKind::PopUp` nor `Floating` is topmost on Windows**
  (windows/window.rs:402-404). The pop-out is `Normal`, and
  `os_window::keep_on_top` sets `HWND_TOPMOST` and clears the
  `WS_MAXIMIZEBOX` gpui gives every resizable window, so a double-click on the
  picture-caption cannot fill the screen. Windows ignores `WindowOptions.focus`,
  so a pop-out takes activation when it opens. On macOS `PopUp` is a panel that
  hides whenever the app is inactive, so the pop-out is not offered there
  (`pop_out::offered`).
- **Closing the main window closes every pop-out** (`main`'s should-close,
  `close_pop_outs`). `WindowsWindow::drop` queues `DestroyWindow`
  (windows/window.rs:509-523), so the main window goes first, and the process
  exits when the last pop-out's `WM_DESTROY` empties the window list
  (windows/platform.rs:718-745). A pop-out's own close brings its pane back
  instead, by returning false from its should-close.

Dragging a pane's header onto another pane, which swaps the two, is gpui's
own drag and drop (`on_drag`, `drag_over`, `on_drop`), and four things about
it are not what they seem:

- **A drag is the app's, not a window's.** `App::active_drag` is one value,
  and every window paints its preview over everything it draws
  (window.rs:2055-2060) — a pop-out included, over its picture. So the
  header's drag is drawn as nothing (`cx.new(|_| gpui::Empty)`), and the
  outline on the pane under the pointer is the cue. Hover goes with it:
  while anything is dragged, `on_hover`, `hover` and `group_hover` report
  nothing in any window (below). gpui does not say what a drag carries
  either, and the volume slider drags too, so the root keeps which pane is
  being dragged itself (`RootView::pane_move`), set from the drag's start
  (`PaneAction::BeginMove`, sent from the `on_drag` constructor) and cleared
  by the window-level release in `drag_listeners`, by a drop, and by `Esc`
  (`cx.stop_active_drag`).
- **A drop needs a layer of its own, and it must occlude.** `on_drop` fires
  only over the element's own hitbox (div.rs:2089-2121), and a pane's cell is
  covered by things that block the pointer — the player and its bar, a menu,
  the band over the picture, chat — so a listener on the cell hears a drop
  only in the gaps. While a header is dragged every other pane's cell gets
  a last child over all of it that occludes and carries the `drag_over`
  outline and the `on_drop` (`watch::drop_layer`), mounted only for the
  drag, so it never takes a press meant for what is under it.
- **A click that ends a drag is ignored.** A press on the header's name or
  one of its icons can start the header's drag, since they do not block
  their parent's press. gpui clears their pending click on the next frame
  once a drag is under way (div.rs:1569-1577), but a drag that begins and
  ends between two frames still clicks — the name would open twitch.tv. So
  each of the header's controls does nothing while `cx.has_active_drag()`.
- **Windows has no grab cursor.** gpui maps `OpenHand` and `ClosedHand` to
  the arrow there (windows/util.rs:113-135), so nothing about the pointer
  says a pane is being carried: the target's outline is all there is, and
  it is enough.

**Neither `group_hover` nor `on_hover` means "the pointer is over this."** Both
resolve through the same expression:

```rust
let is_hovered = has_mouse_down.borrow().is_none()
    && !cx.has_active_drag()
    && hitbox.is_hovered(window);
```

`cx.has_active_drag()` is *app-wide* — it reads `App::active_drag`, one value
for every window (app.rs:1939-1941) — and gpui-component's `Slider` drags via
`on_drag`, so touching the volume slider makes every hover listener in every
window report false, pop-outs included, and the one whose control bar holds
that slider too. And
because `on_hover` only fires on a `MouseMoveEvent`, leaving the window is
invisible to it: the last move it saw was inside, so the controls stayed up.

Hover is therefore **measured, not reported**: a `canvas` probe already runs each
frame for render sizing, and it now also asks
`window.is_window_hovered() && bounds.contains(&window.mouse_position())`. The
`on_hover` listeners that remain exist only to wake a repaint — a paused stream
sends no frames, so without them nothing would ask the probe to run again. Their
*value* is ignored on purpose. Do not wire it back up. An element that
`.occlude()`s over a probed area hides the pointer from those listeners, so
it needs a wake-up of its own while it blocks, as a pane's header band has.

**A view flowed into a flex item can be laid out at nothing, and stay that
way.** A pane's chat used to be an in-flow block, `size_full`, inside the
`flex_1` box under its header (`watch::pane`). With two panes side by side
(each stacked: picture over chat) in a 2400 by 1100 window, both chats went
blank, no rows and no "connecting", as soon as both pictures were 16:9
renditions, which put each video box at 603.28 px. They stayed blank, through
new messages and wheel scrolls, until the layout changed. A pane whose
rendition was 852x480 (aspect 1.775, a box a pixel taller) did not do it.
Logging `ListState::viewport_bounds` and a `canvas` in chat's root showed
the root laid out at 0 by 6 px every frame while the box around it measured
1073 by 407. It looked like a reorder or quality-swap bug, because a swap to
1080p or a resize is what usually brings the sizes about.

The fix is placement rather than flow: the box is `relative`, and chat sits in
an `absolute().inset_0()` layer inside it. An absolute child takes no part in
measuring its parent, and is laid out once, against a box already sized.
Measured on the same launch-then-widen sequence: blank 3 runs out of 3 before,
0 out of 3 after, alternating builds. The cause inside taffy was not traced to
a line. The likely candidate is that taffy 0.9.0's layout cache is keyed
without the parent's size, so a size measured under one constraint can be
returned under another. taffy 0.12.0's changelog adds the parent size to the
key and calls it "necessary for correctness". gpui 0.2.2 pins taffy 0.9.0. Any
other view that fills a flex item by flowing a percentage-sized block into it is
exposed in the same way. If one shows up empty with a healthy parent, place it
the same way. The video box sidesteps a related problem by making the player a
flex item (see the comment on `video_pane` in `watch::pane`).

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
Windows sampler reads is dead code there. With a pane popped out, `visible`
and `renders_per_s` still describe the main window alone: `visible` looks at
the process's first visible top-level window that is not topmost, which a
pop-out always is, and the count is the root's renders, not a pop-out's.
`focused` does not: it asks whether the foreground window is any of perch's
(`is_focused`), a pop-out included, and a pop-out takes the keyboard when it
opens (see "Known limits"), so it reads 1 then with the main window behind.
Making it main-only would mean comparing the foreground window with the one
`top_level_of` finds.

Three columns do most of the work. `by_thread` attributes the time by *thread
name*, which Windows keeps for anything Rust or libmpv named — so `main` is the
UI thread, `mpv-render` is `video.rs`, `worker`/`demux`/`vo`/`core` are libmpv,
`image-cache` is downloads. `renders_per_s` separates work from spinning: a
static page repainting sixty times a second is a bug and looks identical to a
busy one in a CPU number. And `child_pct` is streamlink and its python, because
Task Manager folds a parent's children into its row — so a stream left playing
in the mini player reads as perch using CPU when none of it is perch's.

What it said the first time it ran, which is the baseline to compare against:

| state | CPU (of one core) | renders/s |
|---|---|---|
| follows page, nothing playing | 0.3-2.5% | **0** |
| follows page, one stream still playing | ~50% | 60 |
| watch page, one stream | ~110% | 120 |

The first row is the important one: gpui does not repaint a page that has not
changed, so an idle perch is genuinely idle. Every bit of the rest is the
*stream*, and going back to the follows page does not stop one — the pane
becomes a tile in the mini player, still with its sound, and mpv keeps
decoding at full source resolution, because the render size follows the
element but the decode does not. The second row was measured with the old
96px thumbnail; the tile is larger, so measure again before quoting it.

**A windowed app still gets console windows from its children.** streamlink is a
console-subsystem program, so every one we spawn came with its own console — and
since the app itself no longer has one, those were the only console windows a
user ever saw. `streamlink::command` sets `CREATE_NO_WINDOW`. Windows still
pairs a `conhost.exe` with each child; check for *visible windows*, not for the
absence of conhost, or you will conclude the fix did not work.

**The icon is an embedded resource**, stamped on by `build.rs` via
`winresource`, because gpui 0.2.2 has no window-icon API and Windows takes the
taskbar and Alt+Tab icons from the executable anyway; the title bar is
Perch's own and draws none. It needs `rc.exe` from the Windows SDK; a build
without one warns and produces an icon-less binary rather than failing.
`assets/make-icon.ps1` regenerates the `.ico` — entries up to 128px are DIBs
and 256 is a PNG, because GDI+ cannot read a PNG-payload entry back, so a
PNG-only file is one you cannot open to check.

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

The flip side: GPUI keeps that state only from one frame to the next. An
element missing for a frame comes back as a stranger, so a `Fade` that is
not drawn for a while replays its last flip from the start when it is drawn
again. That is a bar long since hidden fading out all over again, over the
picture. Keep a faded wrapper mounted on every frame, or start its fade over
with `Fade::hidden()` when its element goes away, as `VideoView::set_place`
does for the bar on every change of place (`let_go`): a mini-player tile never
draws it, and a pop-out is another window, whose animations start from
nothing. A pane's band, the header over its picture with chat hidden, does
both: `watch::pane` mounts it on every frame, empty when the header is in the
panel, under an id keyed by `pane_id` so a pane closing beside it never hands
it another pane's fade; and `RootView::restage` resets to hidden the `header`
of every pane whose cell is off the screen — every one while the browse page
is up, and every one but the maximized pane's while a pane has the watch
page — since nothing draws it there. It lets go of each such pane's chat
hold too (`ChatView::let_go_hold`): `sync_hold` runs only while a chat is
drawn, so one put away with the pointer over it would hold back every row
that arrived, past the cap on the rows, for as long as it was away. And on
the watch page it counts each such pane as pointed at already
(`Slot::hovered`), so its cell coming back under a pointer that has not
moved is no rising edge to take the keys with.

**Element ids are namespaced by every ancestor that has one**, plus an implicit
`ElementId::View(entity_id)` per entity. So `"controls"` is unique inside a
`VideoView` even with four of them on screen, but anything rendered by
`RootView` — every pane, every toast — has to carry its own discriminator.
`Fade::apply` composes the caller's id with the flip count via
`ElementId::NamedChild` rather than replacing it, so both survive.

**A repeating animation never stops asking for frames.** `motion::waiting` is
only ever attached to a state that ends. "Offline" and "failed" deliberately sit
still: a pulsing error is a permanent 60 fps repaint, and it reads as progress
when there is none. So does the `Start when they go live` switch on a stopped
pane, which is a setting rather than something being waited for, however long
the channel stays off. `Watch from the start`, while the archives are being
asked for, is the still `controls::waiting` shape rather than a pulse.

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
  must not collapse into each other. Mute all and the compact mini player never
  write a level at all, 0 or otherwise: a pane's `Loudness` keeps the level
  the user chose apart from the hush Mute all lays over it, and only a chosen
  level is ever reported to be remembered (`loudness.rs`, tested there). The
  hush is per pane and for the session — it outlives a player rebuilt under
  it, ends at that pane's first deliberate change of level, never un-mutes a
  pane muted by hand, and a pane opened afterwards starts audible. Keys go
  through `settings::channel_key`, because the app does not agree with itself
  about case: Helix says `forsen`, the command line says whatever was typed.
- **Spacing** — named by role (`PAGE_PAD`, `PANEL_PAD`, `CONTROL_PAD_*`,
  `GAP_TIGHT`, `GAP`, `GAP_SECTION`, `PANE_GAP`, `ROW_PAD_*`), not by size.
- **Controls come from `controls.rs`.** There were ten hand-rolled buttons in
  six shapes — the tokens were shared the whole time and the component was not,
  which is the same drift one file down. A control that needs a shape not on
  that list is a new variant there, not an eleventh `div`, declared in
  `variants!` so the contrast test measures it the moment it exists. The same
  file owns
  the things that are not buttons but were drawn by hand in three places each:
  `live_dot`; `tag`, the passive `muted` / `paused` word in a pane header; and
  `badge`, a fact drawn on a picture — a thumbnail's viewers, a recording's
  length, the seek bar's time — which was three recipes at three paddings, two
  of them with a white of their own. The settings sheet uses these too: it was
  the one place with the widget library's buttons, and so the one place a
  primary control was filled rather than bordered. A picture rather than a
  word is `icon_button`, the same variants in a square, and the title bar's
  gear wears `Variant::Chrome` — no fill until the pointer is on it.
  `icon_waiting` is that square with nothing to offer — back with nowhere to
  go: a `text_dim` glyph with no hover, no pointer and no handler, in the
  same place so the bar does not shift, and never a control faded to look
  disabled (see "Things not to redo").
  `caption_button` is the window's minimise, maximise and close: drawn here,
  pressed by the platform (see the title-bar trap). `fold` is a heading that
  folds away the rows under it — the rail's offline follows — with a chevron
  that says which way it is and no fill until hovered. It is built on
  `group_heading`, the box the rail's `Pinned`, `Live` and `Recommended`
  headings sit in, so the fold and the headings above it share one recipe
  rather than two that had to be restyled together. None of the headings
  carries an icon, so the third, added later, does not either. What a
  control says under the pointer is `tip`, one line of words with any key in
  it from `keys::Hint`, or `full_text` for the rest of a line cut short —
  never a `gpui_component` `Tooltip` built at the call site — and a control
  whose words follow a state keys its id on that state too (see the GPUI
  traps).
  The player's bar builds every icon through three helpers in
  `video_view/bar.rs`: `bar_icon` is an `OnVideo` icon button that is
  handed its `tip` only while `window.is_window_hovered()`, so that gate
  holds by construction rather than by each caller remembering it;
  `act_button` adds a click that closes any open menu and then acts;
  `menu_button` adds a click that toggles its menu and closes nothing
  first. Both act on a click, unlike a menu's rows, which act on the press
  (see the GPUI traps). Callers build `tip` through `keys::Hint` where
  there is a key — More has none, and the quality is a pill of words with
  no tooltip but while a pick is under way (`bar::pill_face`) — and pass
  ids keyed on the state the glyph shows
  (`bar-pause`/`bar-play`); the bar takes the keys back for the root in the
  capture phase (`VideoView::return_keys`), so no button has to.
- **Casing** — words on controls are sentence case: `Refresh`, `Open
  settings`, `Save`, `+ Add`, `← Back`, `Try again`, the browse tabs, the
  channel page's shelves. That covers a control's stand-in too — the
  `Loading…` a Load more row says while its page is out, chat's `Chat
  paused` where its jump-to-live pill goes, a switch's `On` and `Off`. A
  thing you read is written as a sentence: titles and headings, notices,
  field labels, tooltips, palette rows. Proper nouns keep their capitals
  either way, and a domain keeps its case (`Search Twitch for “…”`, `Open
  twitch.tv/activate`). There was no rule at first, and three controls and the
  settings sheet's buttons drifted into title case; then the rule was that a
  thing you click is lowercase, until the UI overhaul turned it round. The
  line a pane words itself while it starts or after it stops is a sentence
  too (`Starting…`, `Forsen is offline`, `Finished`), since it sits over a
  sentence-case pill and beside the mini player's words for the same
  states. Not swept yet, so do not take them for the rule: the passive words
  that state a fact — a pane header's `muted`, `paused` and `replay` tags,
  chat's `deleted`, toasts and chat's notices, the palette's kind column —
  are still lowercase, and so is a failed pane's line, which is the failure
  in the words of whatever failed (streamlink, mpv, the streamlink crate or
  the player), passed through as is by `Showing::sentence`.
  `channel_page::kind_tag` is also read mid-sentence, in a palette row's
  `(replay)` and a history byline, so it cannot simply take a capital.
- **Layout reads a `layout::Body`, never the viewport.** The body is the window
  less the rail and the title bar when each is drawn — neither is in
  fullscreen (`layout::rail_shown`, `layout::title_bar_height`) — and it
  is a type made in one place — `RootView::body` — so nothing laying out a page
  can be handed the viewport by mistake. The watch grid was, once: with the
  rail out, every stacked pane carried a 66px black band under its picture,
  because its 16:9 box was derived from a cell wider than the one it was drawn
  in. `a_body_is_the_viewport_less_the_rail` keeps that number, and
  `a_body_is_also_less_the_title_bar` holds the rows to the room under the
  bar. The bar itself is drawn at `layout::title_bar_height`, the number the
  body takes off — whether there is a bar at all included — so the two are one
  decision (`the_bar_is_as_tall_as_the_room_the_page_leaves_it`).
- **The watch grid is `layout::Grid::of(body, cells)`, and nothing else.**
  Its rows, columns, cell size and whether a cell stacks its chat come from
  that one call, made by `RootView::grid`, which hands it to the watch page
  and reads it for the divider drag (`effective_video_share`,
  `on_mouse_move`) and for each pane's quality (`pane_height_for`). The cells
  it is cut for are `RootView::cells`, from `stage::Stage::cells`: every
  pane's, or the maximized pane's alone, which the page draws through the
  same `watch::pane` in a grid of one; `pane_height_for` alone measures a
  pane maximized away by the grid it comes back to. The
  page, the drag and the quality used to work the shape out for themselves,
  four copies of one sum, so a change to how cells are counted had four
  places to miss. `grid_shape` and the cell helpers are private to
  `layout.rs`, so a fifth cannot start; `one_cell_gets_the_whole_body` and
  `the_grid_is_cut_from_the_cells_it_is_given` hold the grid to the cells it
  is given. Whether a cell stacks is judged on its share of the body before
  the seams come out, as `grid_shape` scores shapes and as the page always
  stacked, not on `cell_width / cell_height`: within a few pixels of
  `PORTRAIT_ASPECT` a seam tips the cell as cut the other way, and the share
  keeps the switch where it was (`a_grids_portrait_flag_is_its_cells_share`,
  `a_seam_does_not_tip_a_cell_into_stacking`). A pane's quality is measured
  on the share too (`share_height`, held by
  `a_panes_quality_is_measured_with_the_seams_left_in`); the page and the
  drag use the cell as cut. Everything the page sends back about a pane — a
  press, its hover, a divider pulled (`watch::ResizeStart`) — names the pane
  by its key, never by its place in what was drawn.
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
  the flat floor it replaced left pure blue at 2.7:1. Text on a picture is
  held to the brightest picture there is, a white frame under its wash
  (`theme::over_white`), by
  `every_tier_drawn_on_a_picture_reads_over_a_white_frame`. Two washes, two
  jobs: `video_chrome()` (`#000000e0`) carries every tier — text 13.3,
  `text_muted` 5.8, `text_dim` 4.7, accent 5.2, danger 6.8:1 — and is what
  the player's control bar sits on; `overlay()` (`#000000cc`) carries
  `text()` alone (10.2:1), because `text_muted`, `text_dim` and the accent
  all fail on it over white, and is for badges and the dim over a starting
  pane's poster, under the pane's name in `text()`. `Variant::OnVideo`
  rests at `text_muted` and lifts to `text()` under the pointer — the hover
  wash alone is four percent white and all but invisible on the bar — and
  `controls.rs`'s `sits_on` measures it on that white frame under
  `video_chrome()`, not on black, where light text never fails.
- **Motion** — three durations named by job (`MOTION_HOVER`, `MOTION_ENTER`,
  `MOTION_VIDEO`) plus the waiting pulse, and two easings (`ease_fade` for
  two-way changes, `ease_enter` for arrivals). Motion says *that something
  changed*; anything long enough to wait for is too long.

Audit commands, worth re-running after UI work:

```bash
# From the repo root. Every source file the crate has, in every directory,
# new ones included before they are added.
files=$(git ls-files --cached --others --exclude-standard ':(glob)crates/perch/src/**/*.rs')
grep -ohE "\.(p|px|py|gap|gap_x|gap_y)_[0-9p]+\(\)" $files | sort | uniq -c
grep -nE "\.(p|px|py|pt|pb|pl|pr|gap|gap_x|gap_y)\(px\([1-9]" $files | grep -v "/theme.rs:"
grep -nE "\b(rgb|rgba|hsla|hsl)\(" $files | grep -v "/theme.rs:\|/widget_theme.rs:"
grep -nE "\.text_(xs|sm|base|lg|xl)\(\)|FontWeight::[A-Z_]+" $files | grep -v "/theme.rs:"
grep -nE "\.(w|h|min_w|max_w|min_h|max_h|size|top|left|right|bottom)\(px\([1-9]" $files | grep -v "/theme.rs:"
grep -nE "Duration::from_(millis|secs)" $files
grep -nE "\.opacity\(0(\.0*)?\)" $files | grep -v "/motion.rs:"
```

The list comes from git rather than from a glob, because the glob was
`*.rs root/*.rs` and quietly skipped `instance/` — and would have skipped
the next directory too. Keep the `:(glob)` magic: in a plain pathspec `**/`
needs a slash after `src/`, so it drops every file at the top of the crate.

The first four should return nothing outside `theme.rs`; the tests that
measure text over a picture build their backdrop with `theme::over_white`
rather than a grey of their own. The second
and third are newer than the rest: the first only ever caught gpui's named
spacings, so a `.py(px(3.))` and an `rgb(0xffffff)` sat in two card badges
through every audit until a review read the code. The fifth, literal sizes, is
newer still, and is a ledger rather than a clean sheet: it shows the debt that
was there when it was added, and the list should only get shorter. That is the
`max_w(px(420.))` on the text of an empty list's notice and of the sign-in
code's (both in `browse.rs`), the settings sheet's `w(px(480.))`, and chat's
one-pixel time-break rule, `h(px(1.))`. The quality menu's `min_w(px(120.))`
was on it and is `theme::MENU_MIN_WIDTH` now, and the volume slider's
`w(px(120.))` and its figure's `w(px(38.))` are `theme::VOLUME_SLIDER` and
`VOLUME_FIGURE`, which `bar::fit` reads too. A zero (`px(0.)`) is left out on
purpose: the divider seams and the seek bar's time label hang off zero-sized
anchors by design. The sixth will show genuine timings — the follows poll, the
toast lifetime, an mpv frame wait — but no *animation* duration should appear
outside `theme.rs`. The last should return nothing: a control at zero opacity
still takes clicks (see "Things not to redo"), and only `motion` fades one
there, on its way to `invisible()`.

### Where controls live

**Nothing static sits on a playing picture.** Static information on a moving
picture is exactly what you end up staring past for three hours, so it lives
in the pane header on the chat panel instead — chat is already a panel, so it
costs nothing there. What does come over the picture comes because something
asked for it: the pointer on the pane, one of the bar's menus held open, or a
key that has just chosen the pane. A pane with no picture — starting,
offline, ended, a player before its first frame — is not a picture, and its
header rests over it. The split:

- **Pane header**, one per pane (`watch/header.rs`): a live dot *when the pane
  is actually showing a picture*, the channel name — which opens twitch.tv,
  the way out of a chat that is read-only by design — viewer count, uptime,
  `muted` and `paused`, and at the end of a right-hand cluster two icons:
  where the pop-out is offered (`root::pop_out_offered`), out into a window
  of its own on a playing pane and `Bring back` on a popped one
  (`header::PopOutButton`), both naming `P` through `keys::Hint::PopOut` and
  each with its own id, so a tooltip never outlives the press that changed
  it; then the pane's ×, a `Destructive` icon whose tooltip names `Ctrl+W`
  through `keys::Hint::Close`. With more than one pane its bottom border
  marks the one the keyboard is talking to, and the whole header is a
  handle: dragged onto another pane, the two swap places
  (`RootView::move_pane`; see the GPUI traps and Keyboard). Under that row,
  what is actually on: the title and the game, joined the way the numbers
  above them are, clamped to one line, with the whole of both a hover away
  through `controls::full_text`. The live numbers go when the stream ends —
  they come from a list that will not know for another minute, and an uptime
  still counting beside "ended the stream" is the same lie the frozen last
  frame used to tell. Where it goes is one rule, `watch::Placement::of`, the
  same in both arrangements: with chat on screen it sits on the chat panel,
  above chat beside the picture and below the picture when stacked; with
  chat hidden, or none to show, it rides the top of the picture on a band of
  `video_chrome`, the bar's wash, which every text tier passes on over a
  white frame. The band is drawn by `watch::pane`, mounted every frame —
  empty while the header is in the panel — and faded by `Slot::header`, the
  slot's so it outlives a player rebuilt by a cold restart. It is up
  while the pointer is on the pane, during a pane key's reveal, and at rest
  whenever there is no picture to cover (`band_wanted`); it steps aside while
  one of the bar's menus is open. VideoView owns the bar's hover; the slot
  owns the band's, worked out every frame in `Slot::point` from the pane's
  probe, which also reports the rising edge that makes a pane active. While
  up the band occludes, and a press on it makes the pane active in the
  capture phase, since the pane's own mouse-down cannot hear through it.
  For the same reason it carries its own `on_hover` wake-up while up
  (`band-layer`): the pane's and the player's listeners cannot hear the
  pointer through it either, and with every pane still, nothing else would
  repaint for a pointer crossing onto or off the band — the pane it came
  into would not become active, and the band and bar it left would stay up.
  Over a pane with no picture there is no bar, so the band's header also
  offers chat back (`Show chat`, the chat glyph crossed out), and the status
  screen starts `theme::BAND_ROOM` down so the resting band never covers its
  words.
- **Over the video**, only while the pointer is on it or one of its menus
  is open, or on a pane while a quality somebody picked is under way and
  for a moment after (`video_view::bar_wanted`): the control bar
  (`video_view/bar.rs`). At the left play or
  pause, the speaker — crossed out whenever the pane is silent, Mute all's
  hold included — the volume slider
  and its figure; at the right the quality pill, with two panes or more the
  maximize control, then chat, fullscreen and More.
  Chat is crossed out while hidden, and drawn still, with a `No chat replay`
  tooltip, for a recording that has none, so the cluster keeps its shape. More
  pops the pane out where the pop-out is offered, opens the pane on twitch.tv
  and copies its link, a recording's at the moment it is at (`Copy link at
  1:02:03`). Every icon lifts under the pointer and its tooltip says what a
  press does, and the key that does the same where there is one, from
  `keys::Hint`; More has no key, and the quality pill is words with no
  tooltip. While a pick is under way the pill names it instead and
  breathes, with a `Switching to …` tooltip, and the menu marks the row it
  was made from (the swap trap under Video). A narrow pane drops the figure, then the slider, then
  folds the quality into More's first row and the maximize into the row
  after it (`bar::fit`, from the probe's width); play, the speaker and the
  right-hand buttons never go. The maximize control — `Maximize`, or `Show
  all panes` on the pane that has the page, each with its own id and both
  naming `Z` through `keys::Hint::Maximize` (`bar::maximize_control`) —
  stands between the pill and chat and is not one of the cluster's
  buttons: it folds where they never drop, so a narrow pane's bar runs no
  further past its edge for it, and with one pane it is not drawn and takes
  no room. `bar::RIGHT_BUTTONS` counts the cluster's buttons — `button_row`
  lays them out as an array that long — so a control added there narrows
  the bar sooner; it keeps a commented slot for phase 4's guide, and never a
  pop-out, which is the header's and More's, since this cluster has no room
  and never drops a button. What is
  the pane's rather than the player's — chat, the link, popping out — goes
  up as `VideoEvent::Pane` and is resolved by key in `root/pane_actions.rs`,
  the route the header's close takes; the clipboard is written there and nowhere
  else. The maximize control reads `VideoView::maximize`, a mirror of
  `stage::MaximizeButton` that only `Start` and `RootView::restage` write.
  The chat glyph reads `video_view::ChatButton`, a mirror of the slot's
  `chat_hidden` that only `Start` and `RootView::toggle_chat` write. Point at
  the video and the bar comes up; look away and the picture is all that is
  left. Nothing page-level is drawn over the panes.
- **Under the video**, the status screen (`watch/status.rs`), absolute over
  the whole pane and drawn before the player, until the picture covers the
  pane. A player draws nothing before its first frame — no backdrop, no word
  of its own; the pane and the mini player's tile paint the black behind it
  — and `VideoView::covers` turns true only once that frame has faded in
  (`theme::MOTION_VIDEO`), so `watch::showing` reads a player before then as
  still starting and the first frame fades in over the poster rather than
  out of black. A rendition change on a playing pane never brings this
  back: the new stream takes over inside the same player, which already
  covers (`video_view::swap`); only a cold restart does. Starting, the screen is the picture being waited for: the
  channel's live preview (`browse::stream_preview`, the card's own URL and
  cache entry, resolved in `watch_page` from `stream_info` after the
  retired-preview drain), or a recording's own thumbnail
  (`channel_page::video_preview`; Twitch's processing placeholder is no
  poster, and borrows the channel's live preview instead), under the
  `overlay()` dim with the name and a breathing line in `text()` — on black
  without one, in `text_dim`. Stopped, it says why and what next: offline,
  the channel's last broadcast as a recording card (a pill when the pane is
  short, nothing when it is a sliver: `status::next_up_room`), `Start when
  they go live` as a gpui-component `Switch` where it is offered, and `Try
  again`; ended, `Watch from the start` (waiting while the archives are
  being asked for, absent when none matched), `Try again`, `Close` and the
  switch. The tile says the same in a word, from the same reading.
- **The title bar** (`root/title_bar.rs`), on both pages and gone in
  fullscreen: the rail button, back and forward — each drawn waiting while
  the trail has nowhere to go that way — and the search box at the left,
  then the drag strip, then — while nobody is signed in — what the sign-in
  is waiting for in words ("Enter CODE at twitch.tv/activate" has to stay up
  on either page while you go and do it), the gear, and on Windows the
  caption buttons. The gear is the account control too: once signed in, who
  is rides on its tooltip rather than taking room in the bar. A search typed
  in the box on the watch page leaves it for the results the way `Esc` does
  (`run_search`), and `Ctrl+F` reaches the box from either page.
- **The browse page's tab strip**: the four tabs, then Refresh, left-aligned
  so the toasts at the top-right reach Refresh only in a narrow window — and
  the browse toast offset (`TAB_STRIP_HEIGHT`) keeps them off it even then.
- **The rail** (`sidebar.rs`): a row is a click — a live channel watches,
  followed or recommended, anyone else opens their page — and under the
  pointer it reveals more at its right-hand end, over the viewer count where
  there is one: the pin, filled on a pinned row, where it unpins, and `+` on
  a live row while something plays and there is room beside it. Both stop
  propagation, or the row under them would fire as well. A recommended row
  has no pin, so with no room for `+` its count stays put rather than
  giving way to nothing. The offline group's heading is `controls::fold`,
  which unfolds it for the session.

- **The mini player**, on the browse page while something plays
  (`root/mini_player.rs`): the pictures, and under them a bar with what is
  playing — names and how many are paused, in words, never over the video —
  and icon controls for all of it: Mute all / Unmute all, `Pop out` where
  the pop-out is offered and a pane here has a player to move, Back to
  watching and Stop all. `Pop out` (and `P` on the browse page) pops every
  pane shown into a window of its own (`pop_out_all_shown`), each placed
  clear of the others by `layout::pop_out_bounds`, and the mini player goes
  once none is left at home; with nothing left to pop out, `P` there brings
  every popped pane back (`pop_in_all`). Whether there is anything to pop
  out is one answer, `mini_pop_out_offered`, which both the control and the
  key read: a pane here that is starting, off, ended or failed has no
  player to move. A click on a picture goes back to
  watching with that pane active, even when the pointer lands on another
  pane's video: a pane's
  measured hover makes it active on a rising edge, so `go_watch_pane` counts
  every pane as already pointed at for the first frame. A `×` revealed on the
  picture under the pointer closes that one pane through `close_slot`, looked
  up by key at the click, so the per-stream close the docked bar had is not
  lost; beside it, where the pop-out is offered, an icon pops that one pane
  out. Both stop propagation, or the tile would go back to watching too, and
  both sit in one row that is hidden until the pointer is on the tile. The
  players in it are tiles (`stage::Place::Tile`, through
  `VideoView::set_place`): no control bar, no hover, no double-click
  fullscreen. Becoming a tile also closes a pane's open menu and starts its
  bar's fade over from hidden, and `RootView::restage` does the same for each
  pane's band, so coming back never replays a fade (see Motion). A pane in a
  window of its own is not in the mini player at all (`mini_slots`): it is on
  screen already. With the mini player off, leaving the watch page stops
  every pane but those, and so does turning it off while browsing — one rule
  for both, `RootView::retire_homeless`, since nothing draws a pane at home
  there. A pane that comes home from its window while browsing so is stopped
  by the same rule (`restage`), as it would have been had it been home when
  the page was left, or it would play its sound with nothing drawing it;
  Bring back is the exception, and brings the watch page up with it
  (`pop_in`). So a double-click on a picture is
  two separate things: the first click puts the watch page under the second,
  which lands on whatever the page has in that corner — chat, in most
  layouts, so nothing more happens. Only a pane with no chat beside it puts
  its video there, and then it is that pane's double-click, fullscreen.
  Either way the second press leaves the keys with the pane whose picture was
  clicked: a pane's press makes it active only on the first press of a run
  (`watch::pane`). The player's controls take focus back for the root,
  since the player blocks the root's own mouse-down.
- **A pane's pop-out** (`root/pop_out.rs`), Windows only: the picture alone,
  dragged by itself, with the same bar over it under the pointer — play, the
  speaker, the volume and a recording's seek row or a live pane's timeline —
  whose right-hand end is Bring back and a red Close (`bar::pop_out_cluster`)
  and nothing else: no quality, chat, fullscreen or More, and no
  double-click. Its pane's chat and
  header stay in the main window, whose cell offers `Bring back` too. The
  ways out are the header's icon, More's `Pop out`, `P`, the palette and a
  mini tile's icon, and the mini bar's `Pop out` for every pane it shows; the
  ways back are the pop-out's Bring back, the cell's, the header's icon, `P`
  in either window and the palette. The window's own close (`Alt+F4`, the
  taskbar) brings the pane back; the bar's Close and `Ctrl+W` close it.
- **A pane given the window** (`stage.rs`, `root/panes.rs`), with two panes
  or more: `Z`, the bar's maximize control, More's row once that has folded,
  and the palette's `Maximize …` give one pane the whole watch page, chat and
  all. The grid draws its cell alone (`Stage::cells`), through the same
  `watch::pane` in a grid of one, so its element path is unchanged and
  nothing remounts. The others are drawn as nothing —
  `stage::Place::Offstage`, through `VideoView::set_place` like any change
  of place — and play on with their sound, at the size they were last drawn
  at, redrawing no window while the palette is closed: the page reads only
  the panes it draws, but the palette reads every player in the main window
  to know which can offer `Choose quality for …` (`palette_entries`), and so
  redraws the main window at their frame rate while it is open. Nothing rather
  than a strip or thumbnails: a strip of names is static chrome that takes
  height from the pane given the page, and so lowers the rendition it is
  picked; moving thumbnails would be the focus layout that chat per pane
  rules out; and a pane drawn small asks its player for a small frame, which
  comes back soft when the grid does. Choosing a pane — `1`–`4`, `Tab`, its
  tile, its palette row `Choose quality for …`, opening one already open,
  bringing one back from its window — moves the maximize to it
  (`RootView::choose`); a popped pane chosen keeps the keys in its own
  window and leaves the maximize where it is. `Z` or the control again,
  `Esc` and the palette's `Show all panes` show every pane
  (`show_all_panes`). Adding a pane (`open_channel`, `open_video_at`), the
  maximized pane closing or being the one left, a popped pane coming home
  because its stream stopped (`Stage::retain`, from `restage`), or popping
  the maximized pane out (`Stage::pop_out`) ends it too; a recording swapped
  in for it keeps it (`Stage::rename`). However it ends, the grid comes back
  under a pointer that has not moved, and each pane that was off the screen
  counts as pointed at already (`sync_presentation`), as on the way back
  from a tile, so the pane now under the pointer does not take the keys; and
  should the pane the keys talked to have closed meanwhile, they fall back on
  the first cell the grid draws (`RootView::active_slot`), never on a pane
  drawn nowhere. Leaving the page keeps it, and the mini player shows every
  pane meanwhile. For the session only.

The watch page used to float a back pill and, with the rail folded away, the
control that brought it back, in its top-left corner, revealed with the video
controls by the panes' hover; the first pane's header kept a reserve clear for
them. Both left the page — the rail's for the title bar, as its button — and
the reserve went with them. An earlier arrangement put the channel name and
close over the video at the pane's top-right; that went, and with it the
collision that let one click both close a pane and navigate away. The name
and the × are over the video again with chat hidden, as the whole header on
the band along the top, and the collision cannot come back with them: nothing
page-level floats over the panes any more, so the top edge is the pane's.

A pane with chat hidden used to keep its header as a strip above the picture
beside the video, because nothing was drawn over the video at all, and losing
the strip would have left a pane with no name and no way to close it but the
keyboard. The strip cost the picture its height and moved it every time `C`
was pressed; the band costs neither. Stacked, a pane with chat hidden keeps
the box a pane with chat has, with `Chat hidden · press C` (or `No chat replay
for this video`) in the space below, rather than giving the picture the whole
cell, which would put the picture out of line with its neighbours the moment
`C` was pressed. A cell stacks only narrower than `PORTRAIT_ASPECT`, narrower
than any landscape stream, so with the divider where it is derived the box
already gives a 16:9 picture the cell's width, and the whole cell would add
only letterbox. `a_stacked_cell_never_has_room_to_widen_a_landscape_picture`
holds the two constants to that. Two cases do give up some width, and are
left: a 4:3 stream in a cell just narrow enough to stack is capped a few
percent short, and a dragged divider keeps the box the user chose, smaller
or not — the divider still works with chat hidden, and its share is every
pane's. A vertical stream would gain from the whole cell, and is left capped.

Everything in the pane header but the name comes from a `LiveStream` — the
same record the browse cards use — looked up by login at render time rather
than copied onto the `Slot`, so there is one source and it cannot go stale.
`RootView::stream_info` walks every live list the app holds, follows first and
then popular, the open category and the last search, because a pane opened from
Popular used to have no numbers and no title at all: the panes most likely to be
somebody you had never watched before were the ones the header said least about.
Failing those, the rail's recommendations, which are no `LiveStream`s: the
header reads a `watch::LiveInfo` borrowed from either (`RootView::live_info`),
and the name comes from `RootView::channel_name`, which asks the same lists,
the offline follows and the recommendations; see "Browsing".
Resolved once per pane per frame in `watch_page`, not per pane inside the page,
which would be the same walk four times over. The same record gives a starting
pane its poster — the preview its browse card shows — and an ended one the id
of the broadcast it was showing (`LiveStream::id`, kept as `Slot::broadcast`).
A channel opened purely by name —
the palette, the command line — appears in none of those lists, and the header
correctly shows the name alone. Filling *that* gap needs a
`GET /helix/streams?user_login=…` per channel.

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
the page navigation that then floated in that corner, and since neither called
`cx.stop_propagation()` a single click closed a pane *and* navigated away.
Top-right is the only anchor that cleared the corner for every grid shape
`layout.rs` can derive — with four columns, the third pane's *left* edge also
landed under a centred nav.

### Browsing

Follows are **two lists that never merge**. `LiveStream` means *is live*, and
three things read it that way — the went-live toasts, the card's viewer count,
and the pane header's live dot — so an offline channel sitting in that vec
would be wrong in all three at once. Offline follows are `Channel`s, and they are
drawn as names rather than cards: a card is mostly a picture, and an offline
channel has none worth showing — a thumbnail stale by hours, or a profile
picture that costs another request per refresh and says nothing. Names also
pack, so a hundred follows is five rows instead of a wall of grey rectangles.
Clicking one opens the channel's page — its past broadcasts, with its chat one
click away from that page's bar, since chat connects whether or not anyone is
streaming.

**A channel's page is three lists**, past broadcasts, highlights and uploads,
behind a switch of pills: `ChannelPage::shelves`, one `Listing` each, so each
keeps its own cursor and a hundred daily broadcasts do not bury the highlights.
A kind is fetched the first time it is shown, and again whenever it is shown
empty. `channel_page::in_progress` is archives only: its second signal is a
listed end close to now, which a highlight cut or a video uploaded minutes ago
has too. Neither kind has a chat replay, so they open as picture alone, and the
pane header's tag names the kind — `replay`, `highlight`, `upload`.

Both followed endpoints paginate — see the Networking trap — and
`/channels/followed` is the one whose `first` defaults to 20 rather than 100:
forget the parameter and a long follows list quietly shows a fifth of itself.

**Refresh means "this list", not "follows".** One control, beside the tabs,
whichever list is up, because the discovery tabs are otherwise fetched once and
kept forever, which is right for a page you glance at and wrong for one left
open all evening.
`Request::Follows` is intercepted in `run` rather than handled in `serve`,
because `serve` cannot see the poll timer: answered there, the poll just done by
hand would be repeated automatically seconds later, for two of everything. And a
failed poll is `FollowsError`, never `Error` — the UI turns `Error` into
`SignIn::Error` and blanks the page, which is far too much to say about one
dropped request. It is only surfaced when somebody actually pressed the button;
the minute-by-minute poll fails to stderr, because an hour-long outage should
not be sixty toasts about a list that is still on screen.

Three lists on one page — home, popular, categories — because they are the
same question asked three ways, so they share one grid and one card (Home adds
a row of the history's recordings between its live cards and its names). Only
categories look different, and only because box art is 3:4 rather than 16:9.
Opening a category *replaces* the page rather than nesting inside the tab, so
there is only ever one thing to scroll. Which takeover is up is
`Discovery::place`, the one copy of the order they stack in — a channel's
page over a search over a category over the tab — read by the page, refresh,
Load more, `Esc` and the trail alike.

**Back and forward walk a trail; `Esc` steps out.** `Alt+←`/`Alt+→`, the
title bar's arrows and the mouse's side buttons go back through the tabs,
categories, searches, channel pages and the watch page you have been on, the
way a browser does. The trail (`trail.rs`, driven from `root/navigation.rs`)
is history and nothing more: a `Route` is *read* from `page` and `Discovery`
and never kept beside them, so there is no second account of what the window
shows to fall out of step with the first. `RootView::record` wraps the
functions that move the view — `go_to_tab`, `show_tab`, `run_search`,
`on_browse_action`, `open_channel_page`, `open_channel`, `open_video_at`,
`go_browse`, `go_watch` — so a card, the rail, the palette, a toast, a
launch, `Esc` and a context bar's back pill are all steps without knowing it.
Nested calls record one step, and it needs no `Window`, because the search
box searches from a subscription that has none. Going back replays a route
through the same functions with recording held off, doing only what differs:
back from the watch page to the list that opened it asks Twitch nothing. A
takeover whose list has been replaced since is asked for again and opens at
the top — a new category, search or channel page always does
(`ScrollHandle::set_offset`), rather than at whatever offset its shared
handle was left at; the tabs keep their own lists and scroll positions all
session. A route is its identity: a channel's id filled in by the first
reply, or a switch of shelf, is not a new place (the hand-written
`PartialEq`). The watch page drops off the trail both ways once nothing
plays — `retire_slots` forgets it — and a watch page left with nothing on it
is never recorded, so back never stops on an empty page. `Esc` and the
context bars' `← Back` / `← Categories` keep their meaning, out of the
takeover to its tab or else to watching, and back takes either back. A
fifth pane is refused where you are, with the toast; the page used to flip
to watching before the count was checked.

**The fourth tab, history, is the app's own memory and asks Twitch nothing.**
It is `settings::history`, kept in `history.json` beside the settings rather
than in them: it is written every fifteen seconds while a recording plays, and
the file with the sign-in in it is better rewritten as seldom as possible.
`root::history` does the writing — an entry the moment a recording opens, so
one that turns out to be gone is still findable; its place every fifteen
seconds while it moves, and never while it stands still, so a pause does not
rewrite the file or reorder the list; and when its pane goes. That last one is
why `RootView::retire_slots` is the only way panes are closed: the position
lives on the slot, and a `slots.clear()` anywhere else takes the last few
seconds with it. The window closing writes it too. Opening a recording from
anywhere — its channel's page, the tab, the palette, a link with no `?t=` —
asks `resume_point` first, so a card on a channel's page picks up where it was
left as well; within a minute of the end (`history::finished_at`) counts as
watched, and starts from the top. An entry keeps what a pane needs to play the
recording again, so the tab opens one without a lookup; the price is that its
title and picture are as fresh as the last listing that mentioned it —
`refresh_history` takes a channel page's listing as the newer word, and
Twitch's "still recording" placeholder is dropped once the player says the
recording has finished, or it would say "streaming now" on the card for good.
With nothing typed, the palette leads with the newest part-watched recording
when its channel also leads the recents, which is what it looks like when the
last thing opened was that recording: Ctrl+K then Enter carries on with it.

**Home is the landing page, and the history's front row.** The browse page's
first tab (`Tab::Home`, the default, so the app opens on it) was the Following
tab; it is now who is live, then Continue watching, then the offline names,
under one scroller and one filter (`home.rs`). Continue watching is the
History tab's own cards (`history_page::entry_card`) for the entries the tab
itself calls unfinished (`history_page::unfinished`, which is `!finished`), in
the history's order, which is already most recently watched first. It is cut
to one row — `browse::columns` at the card width every grid uses, at least
one and at most `CONTINUING_MAX` (6) — and "Show all" goes to the History tab
(`Action::ShowTab`) rather than unfolding a second copy of it. It needs no
sign-in, so with no follows it sits above the sign-in prompt or the loading
notice. The offline names go by when you last watched them
(`home::by_last_watched`): `Settings::recent` first, then the history's
channels, then everyone never watched by name. Joining the two is exact,
since a channel that fell off the eight recents was watched before every one
still in them. That order lives in its own list, `RootView::home_offline`,
beside the rail's name-ordered `offline`, and is sorted only where the rail's
is — a poll while nothing is held, and the hold's release
(`follows::home_offline_after`, `hold_live`) — never at draw time. A first
cut sorted it every frame, and it broke the hold: a channel whose stream had
just ended joined the end of the names, then jumped to where you last
watched it, usually near the top since you had probably watched it live,
shifting every name after it under the pointer. Each of those is a pure
function with tests, in `home.rs` and `root/follows.rs`.

**Each offline name says when it was last live, and each heading how many.**
"Live 3 hours ago" sits beside the name inside Home's pill (`text_dim`,
lifting to `text_muted` with the name under the pointer, since `text_dim` on
the hovered pill measures 4.44:1), and on the rail's offline rows as the
second line, where a live row has its game — the row is the avatar's 30px
either way, which two 15px lines exactly fill, so it costs no room. The data
is `twitch_api::recommend::last_broadcasts` (the `users(logins:)` query on
the same anonymous GraphQL endpoint as Recommended, a hundred logins a
request), through `Request::LastLive`, answered in `run` ahead of the
session's upkeep like `Recommend`. When to ask is `last_live::LastLive::next_ask`:
every offline follow on the first `FollowedChannels`, then only logins not
asked about since — `on_streams` calls `went_live` with the live list, which
drops a live channel's answer and its asked mark, so it is asked about once
at the first poll that has it offline again — and everyone once
`last_live::REFRESH` (fifteen minutes) is up; one ask at a time, only signed
in, and never at every poll. A failure is one stderr line (`last live: …`)
and the names without words; answers in hand stay. A refusal
(`twitch_api::Error::QueryRefused`, carried as `RecommendError::Refused`) is
one line too and sets `refused`, after which `next_ask` asks nothing for the
session. Twitch's answer is `lastBroadcast.startedAt`, when the last stream
*started*, so on its own a channel that just ended a six-hour stream would
read "Live 6 hours ago". `went_live` therefore takes the poll's `Utc::now()`
and keeps it per login in `seen_live` (kept through a full answer), and
`words` counts from the later of that and `startedAt`; with no answer yet it
counts from `seen_live` alone. A stream that ended before launch is still
counted from its start — the query could add the last archive's `createdAt`
plus `lengthSeconds` if that ever matters. The words count hours for
the first day and then hand over to `channel_page::when` ("Live yesterday",
"Live 3 days ago", "Live last week", "Live on 1 Jul"). A channel Twitch
already says is live, or one that never broadcast, says nothing. The answers
live on the root (`RootView::last_live`) keyed by `channel_key` and go to
`home::Lists` and `sidebar::Rail`. The headings read "Live now · 23",
"Continue watching · 5", "Offline · 103" (`home::counted`), counted after the
filter; Continue watching counts every match, including those behind "Show
all" (`Continuing::total`). "Live now" still only appears beside another
section.

**Forgetting can be taken back.** `History::forget` and `clear` return what
they took — each entry with the place it stood (`history::Forgotten`) — and
the toast that says so carries it as `ToastAction::Undo`; its `Undo` hands it
to `History::restore`, which puts each back where it was. One opened again in
between is already back with a newer place, and keeps it. An entry is only
ever *added* by opening: noting a place moves one the history has, so a
recording still playing after a clear stays off rather than creeping back. The
history's cards name their kind when it is not a past broadcast — the list has
held highlights and uploads since a channel's page offered them, and a
minute-long highlight read as a broadcast barely begun.

Everything the user does there arrives as one `browse::Action` rather than one
callback per control: the page is generic over its owner, so each extra closure
would be another type parameter threaded through every helper.

Three things worth keeping:

- A category's streams are dropped if the reply arrives after the user has left
  it, and so are a search's and a channel's videos. Without that check a slow
  response repopulates the page behind them.
- **Loading and errors belong to the list that asked.** Every browse request
  names the list it fills — `Request::list_key`, a `twitch::ListKey` that
  leaves out the `after` cursor, so a page of a list and the list are one —
  and `Discovery` waits on lists by key. `pending` holds an entry per request
  out and each answer takes one away (`finish`), so a refresh sent while the
  first ask is still out is a second answer to wait for. `is_loading` and
  `shown_error` ask only about the list on screen (`shown_key`). That key is
  read off `Place::first_page`, the request that fills the list on screen,
  through `Request::list_key`, so the list the page waits on and the list the
  request is waited on as come from one derivation rather than two matches
  kept agreeing by hand; `fill_shown` and `refresh` ask for that same first
  page. The reply arms in `follows` still name their list from the fields the
  reply carries back. With one flag for everything, a reply for a list you
  had left took "Loading…" off the one you went to, and a failure said
  "Could not reach Twitch" on whichever list was up when it landed — both
  routine once back and forward made leaving a list mid-request ordinary.
  `BrowseError` carries the key for the same reason; `serve` reads it before
  the request is taken apart.
- `RootView::fetch` refuses to wait when there is nobody to answer — before
  sign-in, or after the worker has stopped — and says why on the list that
  asked. A request made while signed out would otherwise sit in the queue
  behind the device-code poll and pulse "Loading…" indefinitely.
  `fill_shown` asks for whatever list is on screen once sign-in lands — a
  tab, a category, a search or a channel's shelf — so one opened while
  signed out does not go on saying it needs a sign-in that has happened.
  The other way round, the worker's terminal `Error` empties `pending`: it
  can stop with a request taken off the queue and never answered, and a key
  left waiting would pulse "Loading…" for good, since `fill_shown` does not
  ask again for a list it thinks is on its way.

Lists are fetched once per tab and kept, and grow a page at a time on Load
more — see "Paging, and what is not paged" for the cursor's route out to the
caller.

**The live follows hold still while pointed at.** They are sorted by viewers,
and a poll a minute used to re-sort them, swapping neighbours whose counts
crossed — so a rail row or a card could change between aiming and clicking.
Now a probe on the rail and one on Home (`RootView::holding`, a
canvas measuring the pointer against its own bounds, the way chat's hold is
measured) report whether the pointer is over them, and while either is,
`on_streams` merges the poll into the order on screen (`follows::keep_order`):
the ones still live keep their places with the new numbers, the ones that
ended go, the ones that started join the end. When the last is let go,
`hold_live` sorts by viewers again with `twitch_api::by_viewers`, the one rule
every list of streams is sorted by. A list that is not on screen is released
by the page that is not drawing it — the watch page for Home, a
folded rail for the rail — because an unpainted probe says nothing, and the
pointer was always on the card that opened the watch page.

The offline names hold the same way, since both lists show them: a fresh
`FollowedChannels` goes through `follows::offline_after`, which filters out
whoever is live and, while either probe is pointed at, merges with the same
`keep_order` — so somebody whose stream ended joins the end of the names
rather than landing in the middle and pushing the rest down. Home's copy of
the names (`home_offline`) goes through `follows::home_offline_after`, held
the same way. On release `hold_live` puts the rail's back in name order with
`twitch_api::by_name`, the one rule `followed_channels` sorts by, and Home's
in last-watched order with `home::by_last_watched`.

**A went-live toast is the way to the channel**, not only news of it:
`Toast::action`, watch from the text and `+ Add` from the pill beside it. And
a pane that stalled — streamlink said the channel was off, or the broadcast
ended — is retried by `on_streams` when a poll lists the channel live and the
pane says it is due (`Slot::due_to_start`, tested). That is the pane's `Start
when they go live` switch, made visible on its status screen: on by default,
per pane, for the session, and offered only signed in for a channel you follow
(`RootView::start_offered`), because the follows poll is the only thing that
can fire it. A `started_at` later than `Slot::stalled_at` is a broadcast the
pane has not tried. The timestamp is the whole trick: the list is up to a
minute behind the pane, so a stream that has just ended is still on it, and a
retry keyed on presence alone would find it gone and turn "ended" into
"offline" for nothing. One exception, for a pane that says *offline* while the
poll lists a broadcast that began before it stopped: it asked in that
broadcast's first seconds, before streamlink could find it, and gets one try
per broadcast, remembered in `Slot::retried_for`.

**A stopped live pane asks what the channel broadcast**: `Request::Broadcasts`,
the five newest archives (`twitch_api::recent_videos`), asked on Offline and
on Ended (`root/broadcasts.rs`), asked for any pane that missed out when
sign-in lands, forgotten when the worker is replaced, and forgotten when the
pane plays again so the next stop asks afresh. Every stop asks, answer in
hand or not — a pane nothing starts can sit offline through a whole
broadcast, and `Try again` afterwards should offer that one — and the answer
in hand stays on screen until the new one comes (`Lookup::Refreshing`), as it
does when a repeat fails or its worker is replaced. Its own request rather than
`Request::Videos`, which reads the same endpoint: that one fills a channel
page's shelf and ends the page's wait, and its failure is a `BrowseError`
said on the page, whose handler would also end whatever the page was waiting
on. A pane's answer is the pane's, failure included, inside
`TwitchEvent::Broadcasts`. An offline pane offers the newest as a card; an
ended one finds the broadcast that ended with `channel_page::archive_of` —
archives started no later than the end and listed as going on until near it,
the stream id choosing among those and never reaching past them, since it
comes from whichever live list last carried the channel and an old snapshot
carries an old broadcast — and offers `Watch from the start`. Either plays in
place: `replace_with_video` swaps the slot for a new one, keeping its place in
the grid, with no `MAX_PANES` refusal and no trail step; the live chat and the
pane's `Start when they go live` go with it, while Mute all's hold stays
(`Slot::take_over_from`). A recording already open is made active instead.

**Search** is four requests behind one result, and each has a reason:

- `/search/channels` answers with a **profile picture and no viewer count** — a
  different shape from every other list in the app. So only its logins are kept,
  and they go back through `/streams` to become ordinary stream records. One
  extra round trip buys cards identical to every other list. Helix takes up to
  100 `user_login` parameters, so it stays one request.
- The same endpoint again without `live_only`, for the channels that are not on.
  It answers live and offline alike with an `is_live` flag; the offline ones
  are kept as `Channel`s — names, like offline follows — and open the channel's
  page. A separate call rather than the first one without `live_only`, because
  that one's live results are the proven part, and offline matches ranked by
  relevance could crowd them out of a page. It is best-effort: a failure costs
  the names, not the results. The names are re-ranked (`name_rank`): the
  channel whose name was typed first, then names that start with it, then the
  rest in Twitch's order — its relevance put `Asmongold` twentieth, behind
  every `Asmongold_Vevo` that shares the letters.
- Categories are searched in the same breath, because a name like "zomboid" is
  as likely to mean the game as a channel.

Results show **live channels first**, then offline ones, and categories are capped at
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

The three chords on the command key — `Ctrl+K`, `Ctrl+,`, `Ctrl+R` — are the
exception, and do not stand aside for a text box (though `Ctrl+K` waits for
the settings sheet to close; see "Keyboard"). They type nothing, and no
gpui-component widget binds them, so the guard only ever cost something: a
search leaves the cursor in the title bar's box, and the palette was dead
until the page was clicked. They are still scoped to the app rather than
`None`, so a widget that ever claimed one would win it back. `keys` has a
test asking gpui's own keymap.

**Avatars are a second request.** `/streams` carries a stream's preview, not the
channel's picture, so the rail gets its faces from `/users` — batched at Helix's
hundred per request, sent after the live list rather than with it, and merged
into what the UI already holds so the rail fills in rather than blinking. Live
follows are looked up on every poll, because that list is short and changes.
Offline follows are looked up once a session (`twitch::unpictured`, the
worker's `pictured` set), after the followed list is handed over: a hundred
offline follows cost two requests on the first poll and none after it until
somebody new is followed, where asking every poll would have cost as many
requests again as fetching the offline list does. A failed lookup is not
marked, so the next poll tries again. A recommended row brings its own
picture, 70x70 already, and goes through the same `ImageCache`.

**The rail's Recommended group reads Twitch's unofficial sidebar query.**
Helix has nothing like it. `twitch_api::recommend` asks the website's
persisted `SideNav` query, anonymously and on the website's Client-ID, for the
live channels whose viewers also watch a given channel — five a seed when
measured — and `rank` puts the shelves together, channels several seeds agree
on first, less everyone followed, live or offline, and every open pane's
channel. Its module docs carry the hash, the forum's word on the endpoint and
the fixtures. The perch half is three files: `recommended.rs` works it out
purely, `root/recommended.rs` runs it, and `sidebar.rs` draws it, under Live
and over the offline fold.

The seeds are the open panes, most recently watched first, then
`Settings::recent`, each once through `channel_key` and capped at
`recommended::SEED_LIMIT` (six), because each seed is one request and the
worker makes them one after another with every browse request queued behind.
`Request::Recommend` is answered in `run` ahead of `keep_session_fresh`, beside
the interception of `Follows`: it carries no token, and a refresh Twitch turned
down — which ends the worker — should not be what an anonymous ask finds out.
But the worker reads requests only once sign-in has got it into its loop, so
the root asks only while `SignIn::SignedIn`, and only while the rail is
unfolded, since nobody sees the group folded away. One ask at a time, decided
purely by `Recommended::next_ask`: every seed once the last full ask is
`recommended::REFRESH` (five minutes) old, otherwise only the seeds not asked
about since — a pane opened on a new channel is one request, not six — and
nothing while an answer is out, so an answer for an older set of seeds is used
and the new seed asked after it rather than beside it. `update_recommended`
ranks and asks, and is called wherever the seeds, the follows or the answers
change: a pane opened (`open_channel`, `open_video_at`) or closed
(`retire_slots`), `on_streams`, `FollowedChannels`, `SignedIn`, the rail
unfolding, and the answer itself. The follows poll's call is the backstop for
the interval; it asks once in five polls, not at every one.

Failures stay quiet, the stance chat replay takes. `Error::QueryRefused` — a
retired hash, or anything else Twitch will not run — empties the group for the
session and says so once on stderr. Anything else, a 4xx included, keeps the
last good answers, says why on stderr, and is asked again at the next full
ask; a seed whose ask failed is not asked again before then. An empty answer
hides the group, and so does having no follows list yet: without one a
followed channel cannot be told from the rest, and would be offered for the
seconds the follows take. The worker's terminal `Error` and a new client id
forget the ask that was out (`forget_asks`), or no ask would ever go again.
The rows hold still under the pointer with the follows, merged by
`root::recommended::hold` and ranked afresh by `hold_live` on release. `hold`
is `follows::keep_order` and one thing more: a row just opened in a pane is a
seed now, and left out of every fresh ranking, so it is kept where it stands,
lit as watching like a clicked follow, rather than dropped from under the
pointer with the next row sliding up for a double-click's second press to
land on. A recommended row is a live row with the reason in the accent where
the game would be — `recommended::reason`, "Like forsen", "Like forsen and
nymn", "Like forsen and 2 more", by the name a seed writes itself as when a
list knows it — a click that watches and a `+` that adds, and no pin: a pin
for a channel nobody follows is a bare login that cannot say whether it is
live (see "Known limits"). A pane opened from the group is on a channel no
Helix list has, so what it says about it — the name in its header, status
line, mini player and palette row, and the title, game and viewers — comes
from `Recommended::channel`, every channel the answers have named this
session, through `RootView::live_info` and `channel_name`; the reasons that
later name it as a seed use the same name. A recommendation knows no start
time and carries no preview, so that pane has no uptime and starts on black.
Nothing is saved, and the palette does not offer recommendations yet.

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

**New rows wait while the pointer is over the pane.** Not by pinning the list:
a pin at the bottom of a bottom-aligned `list` is cleared by its next layout
pass, and pinning at the first visible row means reproducing `scroll_by`'s
arithmetic. The rows are held in `ChatView::held` and pushed when the pointer
leaves, which changes nothing about the list while it is being read; only
while following live, since a pane scrolled back already holds its own
position. The check is measured — `viewport_bounds` against `mouse_position`
— like every hover here, so a pointer that leaves the window without a move
event still releases the rows at the next repaint. A `CLEARMSG` greys the row
it names rather than removing it, keyed on the `id` tag `ChatMessage` carries.

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

**A recording's chat is replayed from Twitch's own query.** There is no Helix
endpoint for it. `twitch_chat::replay` asks `POST https://gql.twitch.tv/gql`
for the persisted query `VideoCommentsByOffsetOrCursor` (sha256
`b70a3591ff0f4e0313d126c6a1502d79a1c02baebb288227c582044aa76adf6a`) — by
`contentOffsetSeconds` for the first page and by `cursor` for the rest — with
the mobile Client-ID `kd1unb4b3q4t58fwlpcbzcbnm76a8fp`. The website's own id
(`kimne78kx3ncx6brgo4mv6wki5h1ko`) fails Twitch's integrity check on every
cursor page (`IntegrityCheckFailed`); the mobile one is what TwitchDownloader's
chat downloader has used since May 2023, and should it ever gain the same
check, asking again by the last comment's offset and dropping duplicates by id
is gap-free — the page for offset N starts a few seconds before N, measured —
and is what `Source::next` falls back to. A page is fifty-odd comments spanning
a few seconds of a busy chat, sorted by `createdAt`; `first` is capped at 100
and does not enlarge one. Thirty requests back to back answered in about 0.3 s
each with no throttling and no rate-limit headers; the replay keeps one in
flight and paces refills at two a second regardless. An offset past the end
answers `comments: null` beside a "service error", a highlight or an upload an
empty list, and a bad id `video: null`. Replay exists for a broadcast still
being recorded, about thirty seconds behind live. If the hash or the id ever
stops working, the pane shows one notice row and the picture keeps playing;
`crates/twitch-chat/src/fixtures/video-comments.json` is a page as Twitch sent
it on 9 September 2026, trimmed, and is what the parser is tested against.

**Emote positions in a replay count UTF-8 bytes.** `emote.from` and the range
inside `emote.id` (`emoteID;from;to`) are byte indices: after `🎣 ` they say 5
where the IRC tag says 2. They are not read. The `emotes` tag the pane already
understands is built by counting characters across the fragments, and a test
in each crate holds it to an emoji — one that the tag is built right, one that
`emotes::tokenize` reads it back to the same place. A sub notice arrives as a
plain comment — Twitch's sentence followed by the note, `source: CHAT`, no
marker — and renders as one. A deleted account leaves a null `commenter`, and
the comment is skipped.

**A replay is scheduled, not streamed.** The thread reads the pane's position
ten times a second, keeps a minute of comments ahead of it, and emits each as
`ChatEvent::Message` when the position passes it. Twitch keeps offsets to the
second, so a busy second's fifty lines are spread evenly across it rather than
landing as a block on the tick — `Schedule::release`, which also counts the
lines of that second already said, so a page that adds more does not bunch up
what is left. A seek is a discontinuity: the position moved back by more than
a second, or forward by more than the time that passed plus two. It reloads —
the buffer goes, the twenty seconds before the target are fetched (at most
three pages, then one ask at the target itself) — and only then does the pane
get `ChatEvent::Reset` followed by that backlog, so a seek swaps one
conversation for another rather than blanking the pane for the round trip. A
pause is not a seek: the position simply stops. Once the comments run out the
thread asks again every ten seconds from the last offset, which is how the
replay of a broadcast still being recorded grows.

**The position is the pane's, not the player's.** `video::PositionHandle` is
made in `open_video_at`, lives on `Source::Video`, and is handed to every
`VideoStream` the pane starts, so a quality change — a new player, cold or
swapped in place — carries
the position across and the replay sees playback continue rather than a jump.
It is made already holding the place the pane opens at
(`PositionHandle::starting_at`): the replay starts following it seconds before
any player exists, and a pane resuming three hours in used to load the first
minute of chat and then the right one. "Watch again" starts a player at zero,
which the replay reads as a seek back and reloads from the start.
`VideoStream::start` writes the start position into the handle at once, so
neither the bar nor the replay sits on the last player's position for the
seconds a player takes to open — when the player publishes from the start,
as a pane's one player does; one that does not leaves the handle to the
player on screen (see "A player reports its position to itself" under
Recordings). The history reads the same handle, which is
why a pane still opening, or one whose player has stopped, is noted where it
is rather than at zero.

**Two events only a replay sends, and one that means something else.**
`ChatEvent::Reset` clears the rows and keeps the colour map — same chat, same
people. `ChatEvent::Unavailable` is one notice row — "no chat replay for this
broadcast", or a video that is not on Twitch — after which the thread retires.
`Connected` is the join on a live pane and puts a row in it; for a replay it is
the buffer being ready, puts no row (a row stamped with today would draw a
time break between two of last Tuesday's minutes), and is what turns an empty
pane's pulsing "loading chat replay…" into a still "nothing said here yet". A
request that fails says "chat replay interrupted: … — retrying" once per
streak and backs off to thirty seconds; the pill at the bottom of a scrolled
replay says "↓ Newest", since nothing about it is live.

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

**A pop-out has a focus of its own, and keys of its own.** Every gpui window
has its own focus and dispatch tree, and a handle no element of a window
tracks reaches only that window's outermost node (window.rs:4016-4024), where
nothing is bound. So each pop-out's view (`root::pop_out::PopOut`) tracks a
handle of its own, takes it back the way the root does, and reports
`keys::CONTEXT_POPOUT`: `Space`, `M`, the arrows, `P` to bring the pane back
and `Ctrl+W` to close it, and nothing of the watch page's — `Esc` least of
all, since a stray press would move video between windows. Its player hands
the keys back to that handle, not the root's (`VideoView::set_place`). The
three chords scoped to the app alone match in there too and do nothing, since
nothing in the pop-out handles them. `P` on the watch page pops the active
pane out, or brings it back. On the browse page, which has no active pane,
it is the mini player's: every pane it shows out, each into its own window,
or, once it has none left with a player to move — whether it shows no pane
at all or only panes that are starting or stopped — every popped pane back
(`on_toggle_pop_out`). Which of the two is the answer the mini bar's `Pop
out` is offered by (`mini_pop_out_offered`), so the key pops out exactly
when the bar offers to.

**`Esc` on the watch page takes back one thing at a time.** A pane's header
being dragged is let go first, where it was, with nothing moved; then an
open menu closes, then a pane given the whole page shows every pane again
(`show_all_panes`), and only then is the page left (`on_go_browse`). While a
pane has the page, choosing another — `1`–`4`, `Tab`, its tile, its
palette row `Choose quality for …` — moves the maximize to it
(`RootView::choose`), so `Space`, `M` and `Ctrl+W` always act on the pane on
screen, or on a popped one in its own window, never on one drawn nowhere;
once the pane they talked to has closed, they fall back on the first cell
the grid draws, the maximized pane's while one has the page
(`RootView::active_slot`), not on the first pane. `Z`
is bound on the watch page alone: the browse page draws every pane as a
tile, and a pop-out's pane is not on the page to fill it.

**The title bar's search box outlives a change of page.** While the box was
in the browse header, leaving that page took it off screen and
`on_focus_lost` handed the keys back to the root. In the bar it is on both
pages, so nothing does that any more, and a cursor left in it — where a
search leaves it — makes every key on the watch page stand aside for it,
Space typing a space into the search. So whatever the root's own mouse-down
cannot hear takes focus back by hand: the bar's buttons, a toast
(`act_on_toast`), the mini player and the side buttons. A pane opened with
no press on the page at all — a launch handed over, a linked recording
arriving — takes it back in `RootView::show_watch_page`, but only from the
search box, so a field on the settings sheet keeps the cursor. A press on a
pane's own control bar or menu, which hide it from the root, gives the keys
back from the player (`VideoView::return_keys`, holding the root's handle
from `Start::focus`); see the GPUI traps for why it listens in the capture
phase.

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
- **A modal replaces the page name in the context rather than adding to it**,
  so a page-scoped shortcut cannot fire through one and no binding has to
  remember to write `!Modal`. The settings sheet's context also says `Sheet`
  (`keys::CONTEXT_SHEET`), for the one key that has to tell it from the
  palette: `Ctrl+K` is bound on `Perch && !Sheet`, so it closes the palette
  it opened and does nothing over the sheet. The palette is drawn under the
  sheet, so `Ctrl+K` there used to open a box nobody could see with the
  keyboard in it: typing went into the hidden box, and `Enter` opened a
  channel behind the sheet. The two never stack now — the gear and `Ctrl+,`
  already put the sheet in the palette's place — and
  `the_palette_key_waits_for_the_settings_sheet_to_close` asks gpui's keymap.

There is deliberately **no transient feedback** for pause, mute or volume. Each
one announces itself through the thing it controls, so a flash of UI would only
be saying what you already know. What there *is* now is a standing one: the
pane header carries `muted` and `paused` tags, read off the `VideoView` at
render time. Those used to be visible only while the pointer was over the
video, so a channel saved muted opened silent with nothing on screen to say so.
With chat hidden they are again — the header is on the band over the picture,
which shows only while the pane is pointed at — and that is accepted under
"nothing static on the picture": the bar's speaker and play glyphs say the
same on the same hover. The quality is deliberately not there — it is on the
control bar, and a 340px header with a name, a count, an uptime, a tag and the
× in it has no room for a fifth thing; the count reads `358 · 8h 20m` beside
the live dot, the card's shape, for the same reason. The shortcut list lives
in the settings sheet and is read from `keys::SHORTCUTS`, beside the
bindings, so a documented key is a bound one. The README's keyboard table is held to the same list:
`the_readme_lists_every_shortcut` wants a row in it for each label the sheet
shows, the label verbatim and in backticks as the row's first cell. It looks
for the cell, `` | `Esc` | ``, rather than the label, because most labels are
in the README's prose too and would hide a dropped row. Off macOS only, since
the README writes `Ctrl`, `Alt` and `Shift` once and says what a Mac draws
instead.
The `RUNNING` pages are shorter on purpose and are kept in step by hand.

The one exception to "no transient feedback" is **which pane the keys talk
to**. A pane with chat hidden has its underline on the band, so it shows only
while that pane is pointed at — and pointing at a pane is what makes it
active, so the pointer can never answer the question. So `1`–`4`, `Tab` and
`Shift+Tab` with more than one pane, `C` hiding a chat, and a pane moved by
`Shift+←`/`Shift+→` or a header drop, bring that pane's band up for
`theme::HEADER_REVEAL` (`RootView::reveal_header`), underline and all when it
is marked. One pane at a time: a reveal takes it off the others.
It sets `Slot::revealed`, which `band_wanted` reads, and a timer clears it —
the toasts' pattern, numbered by `reveal_epoch` so an earlier reveal's timer
cannot take down a later one (`reveal_is_current`). A header in the panel is
always on screen and gets none. Never on `Space`, `M` or the plain arrows,
which answer for themselves; `keys.rs` says the same at the top. Whether brief
chrome after a key or a drop suits is a product call, so the reveal can be
dropped as a unit: its callers are `reveal_active` in `shortcuts.rs`, `toggle_chat` in
`pane_actions.rs` (where `reveal_header` itself lives) and `move_pane` in
`panes.rs`.

The controls are the fourth place a key is named. The title bar's and the
player's bar's tooltips — `Settings (Ctrl+,)`, `Pause (Space)`, `Mute (M)`,
`Volume (↑ / ↓)`, `Hide chat (C)`, `Fullscreen (F / F11)` — and a pane
header's × (`Close (Ctrl+W)`) never spell a key: each names a keystroke as a
`keys::Hint`, in the binding grammar, and borrows the label the sheet shows
for it, and `every_hint_names_a_listed_and_bound_key` holds every hint to a
listed key bound to the control's own action. `Volume` names `up` and so
reads the row's whole `↑ / ↓`; `Fullscreen` names `f` and reads `F / F11`;
`Close` names `secondary-w` and so reads `⌘W` on a Mac. A hint arrives with
the first control that names it, since an unused one is dead code.

**Fullscreen** is `f` on the watch page, `F11` on either, and a double-click on
the video. The title bar goes with it, and `layout::title_bar_height` gives
its height back to the page — and with it the search box, so `Ctrl+F`, bound
on both pages because the box is over both, does nothing there:
`on_focus_search` returns early rather than focusing a box that is not drawn,
whose focus would only be lost on the next frame and handed back to the root.
The rail goes too, folded or not (`layout::rail_shown`, read by `body` and the
render alike). Its only control is the button in the bar, and left on screen
in fullscreen it was a column the mouse could neither fold nor bring back. So
`B` and the palette's rail row do nothing there either: `toggle_sidebar`
returns early, rather than flipping a setting nothing on screen shows, which
would only surface on leaving fullscreen. The mouse's way out is a
double-click on the video, which brings the bar back.
The double-click lives on the pane's root element; the control bar over it is
`.occlude()`d while it is up, so a double-click on one of its controls does
not also reach it.
A single click deliberately does nothing there — it is how a pane is made the
active one, and pausing on a click would turn choosing a pane into stopping it.

**`Shift+←` and `Shift+→` move the active pane** one place along the order
`1`–`4` count, swapping it with its neighbour exactly as dropping its header
there would (`RootView::move_pane`), and only where a header can be dragged,
with two panes or more on the page. Along the order rather than across the
screen — in a 2×2 grid `Shift+→` from the second pane lands bottom-left —
since the order is what every pane key counts. Clamped at either end
(`stage::moved`) rather than wrapped, which would renumber every pane at
once. Shift and an arrow rather than `[` and `]`, which need AltGr on some
layouts; the plain arrows stay time and volume, since a binding matches its
modifiers exactly. The pane moved keeps the keys, and every pane counts as
pointed at already, so the one the swap put under a still pointer does not
take them. The order is the session's: nothing saves it.

**`1`–`4` and `Tab` choose the pane the keys talk to**, through
`keys::ActivatePane` — the one action here that carries data, derived with
`no_json` because nothing builds this keymap from a file. `PANE_KEYS` is sized
by `MAX_PANES`, so a fifth pane cannot arrive without a key. `Esc` on the
browse page is `StepOut`: out of a channel page, a search or a category, else
to whatever is playing — the other direction from the watch page's `Esc`,
each scoped to its own page. It was `Back` until there was a trail, and was
renamed so the two meanings cannot share a name: `StepOut` goes up from
where you are, however you got there, where `NavigateBack` goes to wherever
you were before. The watch page's `Esc` closes a pane's open menu before it
leaves: the menus are hand-rolled (`video_view/menu.rs`), so they have no key
context to catch `Esc` themselves, and `RootView::on_go_browse` asks every
pane to close its menu first. A pane has one menu open at most, and every one
opens from the bar's right-hand cluster, which is also the menus' one anchor.
A press anywhere else closes it — `on_mouse_down_out` on that anchor rather
than the menu, or the press on the button that opened it would count as
elsewhere and the click after it would open it straight back up. A menu's
button toggles it and closes nothing first, for the same reason; the rows act
on the press (see the GPUI traps). Everything else inside the anchor closes
the menu itself, since the dismiss never hears it: the chat and fullscreen
buttons before they act (`bar::act_button`), and the still chat glyph of a
recording with no chat replay. Only the narrow gaps between the cluster's
controls leave a menu open. The quality menu has no key of its own: the
palette's `Choose quality for …`, offered once something is typed for each
pane with a picture up, opens it, and with it the bar, which an open menu holds
up wherever the pointer is. From the browse page that goes back to watching
first, and any other pane's menu closes. A pane still waiting for its first
frame is offered no row and `open_menu` refuses it: it draws no bar, so the
menu would be invisible and would still take the next `Esc`. The rows come after every `Close` row,
because the letters of "quality" answer to most short queries and `c quin`
should stay a close. More's two rows are in the palette by name as well, after
the quality's and for the same reasons — `Copy link to …` and `Open … on
twitch.tv`, for every pane, a stopped one included, since it still has a
channel or a recording to hand out. They go down the route More's own rows
take (`RootView::on_pane_action`), so the link and the toast are the same.

**Back and forward are `Alt+←` and `Alt+→`** (`⌥` on macOS), as well as
the title bar's arrows and the mouse's side buttons. The keys are bound
twice, on the watch and browse predicates, never on `anywhere`, which a
modal's context also satisfies; and they carry `TYPING` although they are
chords, because Option and an arrow moves the cursor a word in a Mac text
box. The listing gives them two rows — `Alt+← / →` would not fit the key
column — and `alt!` writes the modifier the way `secondary!` does, held to
the keystroke beside it by `the_listing_names_the_modifier_it_binds`. The
side buttons are heard by `RootView::side_buttons`, a window-level listener
like the divider drag's (see the GPUI traps). It acts in the capture phase,
on the press, since a Mac's swipe sends no release; once it has moved the
app it stops the press, so nothing under the pointer takes it too, and takes
focus back for the root by hand. A press with nowhere to go is left to land
as any click would (`go_back` and `go_forward` say whether they stepped):
over the page the root's mouse-down takes the keyboard back from a text box,
as a left click there does, and only over the box itself, the bar or an
overlay that blocks the pointer does the box keep the cursor. With the sheet
or the palette up it does nothing, like the keys and the bar's arrows — all
three ask `RootView::modal_open`. On Windows it is deaf over the
bar's empty strip and the caption buttons; see "Known limits".

**The window remembers where it was.** `Settings::window` is written from
`on_window_should_close` with the platform's restore bounds, so a maximised or
fullscreen window is saved as the size it would un-maximise to. On open it is
used only if some display still intersects it — a monitor unplugged since is
the common way to lose a window — and otherwise the default is centred and
fitted to the primary display, which the old fixed 1600×920 was not: it opened
with its bottom edge off a laptop screen.

The switch to a client-drawn title bar costs one jog. On Windows the saved
bounds are gpui's client area, worked out from the frame with the border
offset gpui measures, and turned back into a frame the same way on open. That
offset shrank when the platform's caption went, so the first launch after the
switch opens about a caption height shorter (derived from
windows/window.rs:1259-1325, not observed). After that it holds, with one
exception, also derived from source and not observed. With the platform's
caption gone, gpui's frame is thicker at the top while maximised than while
windowed (events.rs:1509-1544), and gpui measures the offset again only at
creation, on a DPI change and on a system settings change (window.rs:471,
events.rs:786, events.rs:1116) — never on a restore. A settings change that
lands while the window is maximised, such as the taskbar or a display moving
the work area, leaves the maximised offset in use after the restore, and a
close before the next re-measure saves the restore bounds about 7px shorter
and 4px lower at 100% scale, more when scaled; each time it happens the window
comes back a little smaller. The fix is a patch to the vendored gpui (skip the
re-measure while maximised), which has not been made; see "Known limits".
With the native frame the two offsets were the same, so nothing drifted.

The window has a least size, `root::window_min_size`, set in `main`: as wide
as the title bar with nothing that must stay on it pushed off — on Windows
the caption buttons, the gear, the least of the drag strip and the rail
button and arrows — and as tall as the bar. Without one, Windows let the
window narrow until Maximise and Close ran off its right edge, where nothing
could press them, which the platform's own caption never allowed.

**On Windows the display has to be named.** gpui keeps saved bounds only when
their centre is on the display in `WindowOptions::display_id`, and with none
that is the primary — so a window closed on a second monitor opened at a
default size on the first, every time, while the check above passed.
`main::home_display` picks the display holding the centre and hands its id
over. Not on macOS, where gpui reads the bounds *relative* to the display it
is given: the saved bounds are global, so naming one would shift the window by
that display's origin.

**One perch.** `instance::claim` runs first thing in `main` — before
`capture_stderr` too, which starts by renaming the log and would move a running
copy's log out from under it. The claim is the operating system's, so a crash
leaves nothing to clear: on Windows a named pipe, whose first instance is
created with `FILE_FLAG_FIRST_PIPE_INSTANCE` and so cannot be created twice —
the test and the claim are one call; on Unix `flock` on `instance.lock`, with
`instance.sock` beside it to listen on. The pipe is named for a hash of the
settings directory (FNV-1a, not std's hasher, which may change between
releases): pipes are machine-wide, and a fixed name would hand one user's launch
to another user's window. Its handle is not inheritable, so a streamlink that
outlived perch could not keep the claim.

A later launch connects, sends its arguments as typed — `launch` reads them on
arrival, the same reader startup uses — and waits for one byte back. That
byte comes only once the arguments are queued for a window that still exists:
`main` closes the queue the moment the window starts to close, and a launch
arriving after that is turned away, tries again, and becomes the next perch as
soon as the old one lets go, rather than being answered by a process on its
way out. One that is not answered within five seconds exits, because running
anyway is the race the guard exists to stop. What is handed over opens beside
whatever is playing (`RootView::open_targets`) — the rule startup follows too,
where the first target is alone only because nothing else is open. A handed-over
`--volume` becomes the session's override, as it would have been for the
session it would have started, and `--help` becomes a toast.

**Coming forward without a keystroke.** gpui's `Window::activate_window` on
Windows earns the foreground by pressing Alt through `SendInput`: a real
keystroke, into whatever had the keyboard. Instead the launch that hands over —
which Windows lets take the foreground, because the user just started it —
passes that right to the running copy before it says anything
(`AllowSetForegroundWindow`, on the pid `GetNamedPipeServerProcessId` names),
and the running copy restores itself if minimised and calls
`SetForegroundWindow` on its own handle (`instance::bring_forward`, on the
handle `os_window::hwnd` reads through `raw-window-handle`, which a pop-out
is kept on top through too). It is what
Chromium's process singleton does. The launch opens the pipe with
`SECURITY_IDENTIFICATION`, so whoever might have made a pipe of that name first
learns who called and cannot act as them.

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

**One perch at a time is enforced now.** A build started while another perch
runs on the same settings hands its arguments over and exits — `cargo run`
included, which then seems to do nothing but bring the other window forward. A
debug build says so in its console. Close the running one first; the two were
never safe together, since they spent each other's sign-in.

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

`GetWindowRect` includes invisible resize borders at the sides and bottom —
trim ~8px there. There is no caption to trim at the top any more: the title bar
is Perch's and part of the client area. `SetForegroundWindow` is often refused
by Windows; use topmost instead. This loop caught the pill/chat overlap, the
chat-off-screen bug, and the channel-order verification.

**A test build can run beside the user's own perch.** Start
`target/debug/perch.exe` with `APPDATA` and `LOCALAPPDATA` pointed at scratch
directories: settings come from `APPDATA` (`settings::default_path`), the image
cache from `LOCALAPPDATA` (`root::image_cache_dir`), and the one-perch pipe is
named for a hash of the settings directory (`instance/windows.rs`, `key`), so
the two share no files and nothing is handed over.

**With a pop-out open, the process has two top-level windows.** Find them
with `EnumWindows` filtered by the process id, not `MainWindowHandle`, which
may pick the pop-out: the main window is titled `perch` and a pop-out
`<channel> · perch`. A pop-out is topmost (`GWL_EXSTYLE & WS_EX_TOPMOST`), so
a test that leaves one open leaves it over everything: demote it
(`SetWindowPos(HWND_NOTOPMOST)`) or close it at the end of a run.

**A tile left behind shows up as video memory that does not come back.**
Each window has an atlas of its own, so a move between windows or a restart
that failed to free its frame leaves a tile resident, and the place to see it
is the process's dedicated video memory: `Get-Counter
'\GPU Process Memory(pid_<pid>*)\Dedicated Usage'`, read before and after
ten cold restarts (the settings sheet's quality) with a pane popped out,
should come back to where it was within noise.

**A swap says what it did in the log**, under the pane's key: `starting …
beside …`; then, only for a recording not yet lined up with the pane,
`holding` or `reseeking` (and `unaligned after 3 reseeks; taking over
anyway` if it gives up lining up); then `swapped A->B after N ms; position
a->b` — `after N ms (M ms since the pick)` for a pick from the menu, M being
the wait from the press — or `couldn't swap to …` and why, or `called off the swap to …` when
the root moved on from it (see the swap trap under Video). A live pane, and
a recording that opens close enough, go straight from `starting` to
`swapped`. The log is `perch.log` in a release build and the console in a
debug one. A pane that went back to `Starting…` for a quality change was
started cold, not swapped: its picture did not cover it yet
(`restart_how`), or the change came from the settings sheet, which always
restarts cold (`restart_stream`).

**Check the title bar's hit tests with `WM_NCHITTEST`, after a posted
`WM_MOUSEMOVE` and a short wait.** gpui answers from the last mouse event it
processed (window.rs:1133-1141), not from the message's own point, and a sent
message jumps the posted queue — so a probe sent straight after the move reads
the previous position. Never post a bare `WM_NCLBUTTONDOWN` with `HTCAPTION`:
`DefWindowProc`'s move loop would follow the real mouse.

**Posted input cannot raise what the pointer raises, and what holds the bar up
for it lasts one press.** The first posted `WM_MOUSEMOVE` makes gpui call
`TrackMouseEvent` (windows/events.rs:1254-1273), and with the real cursor
somewhere else Windows answers with a `WM_MOUSELEAVE` that clears the window's
hovered flag (events.rs:318-327). Everything that waits on the pointer reads
that flag — the player's bar and a pane's band through their probes, every
gated tooltip — so none of it comes up. Hit testing is geometry alone, though
(window.rs:775-797, `HitboxId::is_hovered` at 500), so a posted press lands on
whatever is drawn under it. That is the way in: the palette's `Choose quality
for …` opens a pane's quality menu, and an open menu holds the bar up wherever
the pointer is (`VideoView::sync_controls`). Each opening is good for one
press. A press on a row or on any of the bar's controls closes the menu, which
ends the hold, and the bar fades out within `theme::MOTION_HOVER` — except
after a row that picks a quality other than what plays on a pane whose
picture covers it, which holds the bar up, with no blink at the press, until
the switch ends and `theme::SWITCH_LINGER` after (a pick naming what already
plays, or one on a pane with no picture yet, which starts cold, holds
nothing); only More
over the quality menu, the pill over More and More's quality row put one menu
in place of another and keep it. So open the menu again from the palette
before each press. Rows, the pill, More, chat and fullscreen act before the
fade starts, and so do the slider and the seek row, which act on the press
(the seek row holds the bar while its thumb is held, and seeks on the release
wherever it lands; a live pane's timeline, the same row, rewinds on the
release instead, which replaces the pane). Play and the speaker act on a
click, and their press closes the menu in the capture phase, which flips the
bar's fade and so re-keys everything under it (`motion::Fade::animation_id`):
a frame drawn before the release forgets the press, and no click comes. Post
their down and up back to back, since gpui paints only once the posted queue
is empty. Never post two presses in a run where the bar was: the first lets
the bar go, which stops it blocking the picture at once, so the second reaches
the picture, where a double-click is fullscreen. A stopped pane's band is up
at rest, so its × and its screen's controls take posted presses with no help.
Tooltips and the hover lift need a real pointer. Read from source; never post
a press on `Open on twitch.tv`, the pane header's name or the palette's `Open
… on twitch.tv`, which open a browser.

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
racing a restart — changing quality in the settings sheet, still a cold
restart, puts every pane back into `Starting` with the window already open
and stable. A pick from a pane's own menu or the palette no longer does: on a
pane whose picture has faded in, it swaps the new rendition in place and the
pane never leaves `Playing`.

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
follows filter, window placement, past broadcasts and their chat replay, the
September 2026 round of streamlining — recents and open-by-name in the
palette, links on the command line, pane keys, clickable toasts, auto-retry,
the quality re-pick, chat holding still under the pointer — and the watch
history that followed it — where each recording was left, resuming there from
anywhere, the history tab, the time under the pointer on the seek bar, and
getting a stalled recording going again — are built, as are a channel's
highlights and uploads beside its past broadcasts, offline channels in search,
the live follows holding their order under the pointer, undo for forgetting a
recording, and one perch at a time, a later launch handing over to it. So is
rewinding a live pane: a timeline on its bar from the broadcast's start to
now, a press back along which opens the broadcast's recording, still being
made, at that moment in the pane's place, and `LIVE` on that recording to go
back to the live edge (see "Rewinding a live pane" under Recordings).

So is the first phase of the UI overhaul: a title bar Perch draws itself,
holding the rail button, back and forward, the search box and the gear; back
and forward through tabs, categories, searches, channel pages and the watch
page, from the bar, `Alt+←`/`Alt+→` and the mouse's side buttons; a mini
player in the corner of the browse page that keeps each stream's sound, with
Mute all; the browse tabs as a strip with Refresh at its end; loading and
errors kept to the list that asked; the rail as Pinned, Live and Offline,
pinned from the rail itself; a settings sheet that saves only what it owns;
and sentence case on every control.

So is the second, the player. The bar over a playing picture is play, the
speaker, the volume, then the quality as a pill of words, chat, fullscreen
and More, which opens the pane on twitch.tv or copies its link, a
recording's at the moment it is at. Its icons' tooltips name their keys
where they have one; it sits on one wash every text tier passes on over a
white frame, and it gives way as a pane narrows. Its menus
take a press, one at a time, and hand the keys back to the root. A pane's
header has an × that names `Ctrl+W`, and with chat hidden it rides a band
over the top of the picture instead of a strip above it, coming up with the
pointer and for a moment after a pane key. A starting pane shows the picture
it is waiting for and fades in over it; an offline one offers the channel's
last broadcast, played in its place, and the follows auto-start as a `Start
when they go live` switch; an ended one offers that broadcast's recording
from the start. One builder writes every twitch.tv link, and the palette
reaches a pane's quality, `Copy link` and `Open on twitch.tv` by name. The
two bugs phase 1 left for it are fixed: Mute all's tooltip kept its old
words after a press, and a list asked for while signed out said it could not
reach Twitch.

So is the third, multiview and pop-out. On Windows a pane's picture goes
into a small window of its own that stays on top of other apps — from its
header, More, `P`, the palette or its mini-player tile, or every pane the
mini player shows at once, each in its own window and placed clear of the
others — while its place, its number and its chat stay in the main window,
whose cell offers it back; its quality follows that window's size (see "A
pane's pop-out" under "Where controls live", and the pop-out trap). With two
panes or more, `Z`, the bar's maximize control or the palette gives one pane
the whole watch page while the others play on unseen ("A pane given the
window"), and dragging a pane's header onto another, or `Shift+←`/`Shift+→`,
swaps the two for the session. A rendition change on a pane with a picture
no longer goes black: the new stream plays beside the old one inside the
same player and takes over in place (the swap trap under Video). Along the
way a player mpv cannot open says so and offers `Try again`, a paused
picture redraws at its new size, and the watch grid is cut in one place,
`layout::Grid::of`.

**What is left of the overhaul is agreed in outline**, and each phase so
far left a seam for the parts still to come. An omnibox takes the place of
the title bar's search box, which is one element in `title_bar_leading` so
it can be swapped whole. A guide (phase 4); the bar's right-hand cluster
keeps a commented slot for its button, and `bar::RIGHT_BUTTONS` counts the
cluster, so a button added there narrows the bar sooner — which is why
phase 3's maximize control stands before the cluster and folds into More
after the quality pill rather than joining it, and why the pop-out went in
the pane header and More. The Recommended group in the rail, read from
Twitch's unofficial `SideNav` query with the unpublished-query risk the chat
replay already carries, is built (see "Browsing" and "Known limits"); the
palette and the guide do not show recommendations yet, and
`recommended::Recommended::shown` is what they would read. Sound and chat
stay per pane throughout.

Left over from phase 1, smallest first:

- The passive words the casing pass left alone — tags, toasts, chat's
  notices, and a failed pane's line, which is the failure in its own words;
  see "Casing". Phase 2 put the rest of a pane's status lines in sentence
  case.
- Pin from a card, the channel page or the palette. `palette::entries` takes
  seven positional arguments, its tests call it thirty-odd times, and it
  wants an inputs struct before it takes an eighth. Pinning a channel you do
  not follow needs the worker to poll it, since it asks Twitch about follows
  only.
- A look at the title bar on a Mac: the traffic-light position, the room
  left for it and the bar's height are guesses. If dragging by AppKit's strip
  alone is not enough, porting Zed's `start_window_move` into the vendored
  gpui is the fix (a `PERCH PATCH`, and `scripts/verify-vendor.sh` told about
  the file). `Cmd+[` and `Cmd+]` for back and forward were left unbound.

Left over from phase 2, smallest first:

- The title bar's and the mini player's tooltips are not yet gated on
  `window.is_window_hovered()`, as the player's bar and the pane header's
  are; see the GPUI traps.
- The bar's and the status screen's room are estimates, to be tuned against
  a capture of real panes: `theme::QUALITY_PILL_ROOM` (see "Known limits",
  item 24) and `theme::STATUS_ROOM`, which decides when an offline pane's
  last broadcast shrinks from a card to a pill; too small, and a card in a
  short pane pushes `Try again` out of the box.
- Arrow keys between a menu's rows, and a key of its own for a pane's
  quality. The palette's `Choose quality for …` is the keyboard's way in
  for now.
- `Start when they go live` lasts for the session and is per pane. Keeping
  it per channel would be a setting.

Left over from phase 3, smallest first:

- The swap's three estimates, `SWAP_LEAD` (4 s), `SWAP_CEILING` (45 s) and
  `MAX_RESEEKS` (3), are to be tuned from its log lines (see "Working on
  it"). If Twitch turns out not to serve two live sessions on one token,
  live panes go back to cold restarts, and the way to keep them from going
  black is to copy the old view's last frame into an image of its own and
  draw it as a still under the starting screen until the new picture
  covers.
- The pop-out on macOS and Linux. It is compiled and not offered
  (`pop_out::offered`): on macOS gpui's `PopUp` is a panel that hides
  whenever the app is not in front, and hover on it follows the window
  being active (window.rs:1727-1737). A Mac has to show what it takes —
  most likely a `PERCH PATCH` clearing `hidesOnDeactivate`, with
  `scripts/verify-vendor.sh` told about the file — before it is offered.
- The pop-out as a tool window, with no taskbar entry, and saving where
  pop-outs open, the pane order and the maximize across sessions: all left
  out on purpose (see "Known limits").
- Shared Chat's source tags, saying which channel's room a line came from
  in a shared chat. The proposal's multiview paragraph has them; phase 3's
  scope did not, and they are `twitch-chat`'s first.

Ranked by what would be noticed, roughly:

1. **A live link at a moment.** A live `Copy link` at a moment is the
   rewind's mapping (`rewind::archive_for`, `position_in`) with
   `Slot::rewind`'s archive, once found. Getting back to live from a rewind
   is built (`LIVE` on the recording; see "Rewinding a live pane").
2. **Buffered range and muted-audio spans on the seek bar**, from
   `demuxer-cache-time` and the `-muted` segments a playlist names.
3. **Watch the channels in none of the lists.** The pane header reads from
   every live list the app holds, so a pane opened from popular, a category
   or a search carries its numbers, title and game. A channel opened by name
   still carries none: nothing has ever fetched it. `GET
   /helix/streams?user_login=…` per open channel, on the follows poll's
   minute, would fill it, and would also keep a title that changes
   mid-stream honest, which the snapshot does not. The same poll is what
   `Start when they go live` waits on for a channel you do not follow — the
   switch is hidden there today — and it would give such a channel a
   starting poster and an ended pane its broadcast's id. It touches the
   worker's loop and adds a standing Helix request each minute, which is
   why phase 2 left it.
4. **Badges in the chat gutter** — sub, mod, VIP. The tags already arrive and
   are parsed into the map; nothing reads them.
5. **Reply context lines.** `reply-parent-*` tags arrive too.
6. **Highlight rules** that wash the row background rather than colouring a
   word. The wash already exists for events.
7. **Rebindable keys.** `keys::bindings` is a plain `Vec<KeyBinding>` built
   from constants; the work is a UI and a settings shape, not a mechanism.
8. **Sign-out.** There is no way to clear a bad token except editing the field.
9. **The auth-token cookie off argv.** It is documented as a tradeoff, but a
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
3. **A quality change runs two streams for a few seconds.** The rendition
   is chosen again, upwards only, whenever a pane grows, and streamlink has
   no way to switch mid-stream, so the new rendition is resolved and played
   beside the old one until it takes over in place (the swap trap under
   Video): two streamlink sessions, two decoders and two audio outputs for
   the overlap, and a live cut that cannot be lined up, so it can step a
   little. Whether Twitch serves a second live session on one token is
   unverified. The settings sheet's quality and credential changes still
   start every pane over cold, each back to its starting screen until its
   new picture arrives, and a pane that shrinks is left on the rendition it
   has.
4. **No sign-out**, and no way to clear a bad token except editing the field.
5. **Animated WebP** (7TV, some BTTV) may render as stills. Twitch's own
   animated emotes are GIF and animate correctly.
6. **`ImageCache::new` is still on the pre-window UI thread.** It is now
   bounded rather than unbounded — `scan_and_prune` trims the permanent
   directory to 256MB oldest-first and deletes `.part` debris in the same pass
   it indexes, so startup no longer gets slower with every run — but it is one
   `read_dir` plus a stat per file, done synchronously in `RootView::new`.
   Deleting `%LOCALAPPDATA%/perch/images` is always safe.
7. **Orphaned streamlink on a hard crash, on macOS.** Windows ties every
   child to a job object (`streamlink::job`) that the kernel closes with the
   process, however it died; nothing equivalent is wired up on macOS.
8. **Never tested on a vertical monitor.** The layout derives portrait grids and
   stacks chat below video, the logic is unit-tested, but nobody has seen it.
9. **The offline follows list has no cap.** Someone following several hundred
    channels gets several hundred names. There is a filter now, on the tab and
    in the palette, so they can be found; the wall is still a wall. The rail
    folds it under its count and builds no rows folded, but unfolded it is the
    whole wall again — up to a thousand rows, none of them virtualised.
10. **Every settings save is a read-modify-write of the whole file.** Runs of
    changes — a volume drag — are coalesced into one write after they settle
    (`RootView::save_settings_soon`); single changes still write at once.
11. **Chat history depends on somebody else's server.** If it is down the pane
    opens blank, which is what it did before the feature existed. Failures go to
    the log rather than the pane, on purpose.
12. **Shortcuts are not rebindable.** `keys::bindings` is a plain list built
    from constants; see "What to build next".
13. **The palette scrolls with no visible scrollbar.** It is a window of eight
    rows around the selection, so arrowing scrolls it; the wheel works and
    nothing says so. The settings sheet, the browse lists and the rail now draw
    gpui-component's `Scrollbar` over a tracked `ScrollHandle`.
14. **Pins know only what the follows lists know.** A pin for a channel no
    longer followed shows as its bare login, live or not: the worker reads
    the settings once when it starts and asks Twitch about follows only, so
    nothing polls such a channel. Pinning is from the rail only, and a
    recommended row offers no pin for that reason, so a channel you do not
    follow can be pinned only by hand in `settings.json`; and an offline row
    has a picture only if this session saw the channel live.
15. **A recording's thumbnail is 320x180**, the one size Twitch serves for a
    video, scaled up onto a card that is wider than that — and over a whole
    pane, dimmed, while a recording opens. A live channel's preview is the
    card's 440x248, soft over a large pane for the same few seconds.
16. **A jump inside a recording takes about a second**, because it is a reopen
    rather than a seek — see the Recordings trap for why that is the only
    kind that works — and while a recording is still being made a viewer who
    has caught up with the edge waits up to ten seconds for the next segment,
    which is what the live pane is for. Resuming after a pause of half a
    minute or more pays the same second, on purpose — the same trap says why.
17. **Chat replay rides an unpublished Twitch query** and a Client-ID that is
    not the app's own — see the Chat section. There is no sanctioned
    alternative; the website itself has no other path, and TwitchDownloader
    has depended on the same hash and id since 2023. If either stops working
    the pane shows one notice row and the video keeps playing. And the replay
    of a broadcast still being recorded runs about thirty seconds behind live,
    so a viewer who has caught up with the edge sees no chat there; the live
    pane is for that.
18. **The rail's Recommended group rides an unpublished Twitch query too**:
    `SideNav`, by its persisted hash, on the website's Client-ID — see
    `twitch_api::recommend` and "Browsing". Twitch's developer forum has said
    third parties should not use that endpoint. If the hash is retired the
    group goes for the session and stderr says so once; a block would most
    likely arrive as a 4xx, which is read as passing — the list kept, asked
    again in five minutes — since none has been seen to tell it apart. Each
    seed is one request on the worker, so a full ask of six holds the browse
    requests behind it for as long as six requests take. And the reason
    takes the game's place on the row, so what a recommended channel is
    playing is not on the rail, whose rows have no tooltips. When the offline
    follows were last live comes from the same endpoint
    (`twitch_api::recommend::last_broadcasts`); it breaks the same quiet way,
    the names losing their "Live 3 hours ago" (except those the polls saw
    live this session) and stderr saying why, and a refusal stops it asking
    for the session.
19. **The history is only as fresh as the last listing.** An entry keeps the
    title and the picture from when it was last opened or listed, so a title
    edited since shows the old one until the channel's page is opened again.
    And it is written every fifteen seconds while a recording plays, so a
    crash picks up at most that much early; closing a pane or the window
    writes it at once.
20. **A Mac window drags only by AppKit's own strip.** gpui 0.2.2 ignores
    window-control areas on macOS, so only the part of Perch's title bar that
    AppKit's native strip lies under moves the window. A double-click anywhere
    on the bar's empty stretch is handed to the platform by hand. The
    traffic-light position, the room left for it and the bar's height are
    guesses until someone tunes them on a Mac.
21. **The mouse's side buttons do nothing over the title bar's empty strip
    or its caption buttons, on Windows.** Those answer the platform's hit
    test as non-client areas, so a press there arrives as `WM_NCXBUTTONDOWN`,
    which gpui 0.2.2 does not translate (events.rs:84-91). Everywhere else in
    the window — the bar's own controls included — they go back and forward.
    Accepted rather than patched in the vendored gpui.
22. **The saved window size can creep smaller on Windows.** Worked out from
    the vendored gpui and not observed: a system settings change or a DPI
    change that lands while the window is maximised leaves gpui measuring its
    frame as maximised after the restore, and the next close saves the
    restore bounds a few pixels shorter and lower than the window was (about
    7px and 4px at 100% scale), each time it happens. The fix belongs in
    `vendor/gpui` — re-measure only while not maximised — and waits on a
    decision to patch it; see "The window remembers where it was".
23. **A live pane's link has no time.** More's `Copy link` and `Open on
    twitch.tv`, and the palette's, give a live pane's channel. A moment in a
    broadcast still going is a moment in its archive, which the pane knows
    only once a rewind has found it; see "What to build next", item 1.
24. **The bar can run past the narrowest pane.** `bar::fit` drops the volume
    figure, the slider and the quality pill, but play, the speaker and the
    right-hand buttons stay, about 194px with the padding, and four beside
    panes in a small window can be narrower. The quality pill's room is an
    estimate (`theme::QUALITY_PILL_ROOM`), so a long rendition name can run a
    few pixels past a pane right at the edge of fitting.
25. **Posters exist only for channels in a list.** A starting live pane shows
    its channel's preview only when a Helix list the app has fetched carries
    the channel; one opened by name waits on black, and so does one opened
    from the rail's recommendations, whose answers carry no preview. The
    preview is a snapshot, minutes old at most.
26. **`Start when they go live` is only for channels you follow, signed
    in.** The follows poll is what starts a pane, and it lists only followed
    channels; a pane on any other channel has no switch. A poll of the open
    panes' channels would widen it — see "What to build next", item 3.
27. **`Watch from the start` depends on Helix listing the archive.** A channel
    that does not keep past broadcasts offers nothing, and so does one whose
    archive Helix has not listed yet, or lists as ending more than ten minutes
    before the broadcast did (`STILL_GOING_SECS`). A broadcast split by a
    reconnect plays only its last part from the start.
28. **A pane with chat hidden says `muted` and `paused` only while pointed
    at.** Its header rides the band over the picture, which is up while the
    pointer is on the pane, for a moment after a pane key, or over a status
    screen, and the active pane's underline goes with it. Accepted under
    "nothing static on a playing picture": the bar's speaker and play glyphs
    say the same on the same hover, and a pane with chat on screen keeps
    both in its header. See the Keyboard section.
29. **A vertical stream in a stacked cell stays in the capped box with chat
    hidden.** The box is the one a pane with chat has, so `C` never moves
    the picture. A portrait stream is what would gain most from chat's
    space; a 4:3 stream in a cell just narrow enough to stack, and a box the
    divider was dragged smaller, give up a little width too. See "Where
    controls live".
30. **The pop-out is Windows-only.** It is compiled everywhere and offered
    only on Windows (`pop_out::offered`): on macOS the window gpui would
    keep up is a panel that hides whenever another app is in front, which
    is the one thing a pop-out is for, and nobody has run it on Linux. See
    "What to build next".
31. **A pop-out takes the keyboard when it opens, and has a taskbar
    entry.** Windows ignores `WindowOptions.focus`, so the new window is
    activated and the keys go with it; and it is an ordinary window, with a
    taskbar and `Alt+Tab` entry, because that is how the keyboard gets back
    to it. Making it a tool window would take both away.
32. **Panes maximized away still decode**, at the size they were last drawn
    at and with their sound, so a maximize saves no CPU: the quality rule
    puts the picture you come back to first. They redraw no window while
    the palette is closed; while it is open the main window redraws at
    their frame rate, since it reads every player to know which can offer
    `Choose quality for …`. See "A pane given the window".
33. **The pane order, the maximize and where pop-outs open last for the
    session.** Nothing of them goes into `settings.json`: a new session
    opens what it is told in the order it is told, every pane in the main
    window, and the first pop-out in the corner again.
34. **Rewind depends on a list knowing the broadcast and on Helix listing
    its archive.** The timeline needs `started_at`, which only Helix's live
    lists carry, so a pane opened by name or from the rail's
    recommendations, whose answers say no start, has none (see "What to
    build next", item 3). A channel that keeps no past broadcasts, or whose
    archive Helix has not listed yet in a broadcast's first minutes, says so
    in a toast and stays live; so does the first press after a restart,
    until Helix lists the new broadcast's archive. A broadcast split by a
    reconnect rewinds only within its last part. The start comes from
    whichever list knows the channel first, so a Popular page fetched during
    an earlier broadcast this session can draw the timeline from that one's
    start; a press before the archive began then opens the archive at its
    start. The way back leans on the same lists. On a channel you follow,
    `LIVE` goes at the first follows poll after the stream ends. On one only
    a snapshot list carries (Popular, a category, a search), or one a
    rewind came from that no list carries any more, it stays until Twitch
    ends the archive's playlist, so for that while after the stream ends a
    press opens a pane that says the channel is off. On a recording opened
    from a channel's page or the history of a channel no list carries —
    opened by name, or from the recommendations — it never shows. And the
    first part of a broadcast a reconnect split has a finished playlist, so
    it never shows `LIVE` either.

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
- Do not derive the video's controls' visibility from `group_hover` or from
  `on_hover`'s value; see the hover trap. A card's reveal of its own pills can
  ride `group_hover`, since all a drag elsewhere costs it is a moment hidden.
- Do not hide a control with opacity alone. At zero it is still there to be
  clicked: the watch page's old back pill went on navigating from a corner
  that looked empty, and a tap on a card's corner — a touchscreen's, with no
  hover first — could forget a recording through a pill nobody saw.
  `motion::Fade` ends hidden as `invisible()`, which gpui does not paint and
  registers no listeners for, and the cards' pills are `invisible()` until
  their card is hovered.
- Do not name a display in `WindowOptions` on macOS, and do not leave it out on
  Windows. The two platforms read saved window bounds opposite ways; see "The
  window remembers where it was".
- Do not call `Window::activate_window` on Windows. It presses Alt through
  `SendInput`; `instance::bring_forward` comes forward on a right the launching
  process handed over. See "Coming forward without a keystroke".
- Do not move `instance::claim` below anything that touches what a running
  perch owns — the log first of all — and do not answer a handover before its
  arguments are queued for a window that still exists. See "One perch".
- Do not key animated-image element ids on position.
- Do not draw on a picture without a wash measured for what goes on it.
  `video_chrome()` carries the bar and a pane's header over the picture,
  every tier; `overlay()` carries badges and a poster's dim, with `text()`
  on it and nothing else, since `text_muted`, `text_dim` and the accent all
  fail there over a white frame
  (`every_tier_drawn_on_a_picture_reads_over_a_white_frame`). A new thing on
  a picture is measured on `theme::over_white` of its wash, never on black.
- Do not write a twitch.tv URL with a `format!` of its own. `target::link` is
  the one builder, and the tested inverse of `target::parse`
  (`a_link_reads_back_as_what_it_names`), so a link Perch hands out opens the
  same thing, at the same moment, when it is pasted back.
- Do not use `--stream-url` to skip streamlink's pipeline for a *live* stream.
  A recording is resolved that way on purpose, and reads its playlist itself;
  see the Recordings trap.
- Do not hand mpv a recording's playlist URL and `seek` it. It works on a
  transport-stream recording and never on a fragmented-MP4 one, and the second
  kind is most channels. Reposition by rewriting the playlist, as `vod.rs` does.
- Do not rename or rewrite a playlist the player is reading. Write a new file
  for a reposition; append for growth. And never let two players write under
  one name: each player's label is its own (`vod::playlist_label`).
- Do not resume a recording paused for a while by unpausing it, or wait on
  ffmpeg to notice a connection that died under it. A silently dropped one
  hangs the demuxer for good; reopen where it is. See the Recordings trap.
- Do not close a pane with anything but `RootView::retire_slots`. The place a
  recording was left is on the slot, and has to be written down first.
- Do not add tokio; use a thread plus an mpsc pump.
- Do not pin a bottom-aligned `list` to hold chat still; hold the rows back
  instead. See the Chat section for why the pin does not survive a layout.
- Do not put a repeating animation on a state that can persist.
- Do not assume an overlay blocks input because it covers something; use
  `occlude` / `block_mouse_except_scroll`, and put it on the smallest thing that
  is actually opaque. And only while it is drawn: an `invisible()` occluder
  still blocks the pointer.
- Do not make a menu row act on a click. The press closes the menu, and the
  click would come from a frame with no row in it; `menu::menu_row` acts on
  the press, and `menu_rows_act_on_the_press` holds `menu.rs` to it.
- Do not give the picture the whole cell when a stacked pane hides its chat.
  The box stays the one a pane with chat has, so `C` never moves the picture,
  and with the divider where it is derived a landscape picture would gain
  only letterbox
  (`a_stacked_cell_never_has_room_to_widen_a_landscape_picture`); a dragged
  divider is the user's box, and keeps it. Nor bring
  back the strip a pane with chat hidden kept above its picture beside the
  video; the header rides the band over the picture instead (see "Where
  controls live").
- Do not give a control a fixed id when its tooltip's words follow a state it
  changes. The tooltip keeps the words it came up with; key the id on the
  state, as the bar's glyphs, Mute all, the rail's pin and the gear do (see
  the GPUI traps).
- Do not take `.occlude()` off the title bar, give a caption button a handler,
  or wrap the bar's controls in its drag area. Each one hands the platform's
  press to gpui, which reports it handled, and the window stops dragging,
  maximising, resizing from the top or closing — silently. See the title-bar
  trap.
- Do not put a modal, a toast or anything else that floats outside the root's
  content wrapper, and do not give that wrapper an id. Under the bar is what
  keeps them off the drag strip and the caption buttons, and an id there would
  re-namespace every element id beneath it.
- Do not let Mute all or the mini player write a volume. The hush lives in
  `Loudness` beside the level the user chose, and only a chosen level is
  reported to be remembered; a hush saved as `Some(0)` opens that channel
  silent next time, which is the bug muting never becoming the default exists
  to prevent.
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
- Do not give the settings sheet the whole of `Settings`, or take more than
  `SheetFields` back from it. A whole copy was as old as the sheet, so
  whatever the app wrote since — the channel a handed-over launch opened —
  went back with it; `adopt_sheet` takes the fields the sheet owns. See "The
  settings sheet saves only what it owns".
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
- Do not size a tooltip with `max_w`, or with a width on the tooltip itself.
  Same rule as above, one layer further out: a tooltip is laid out against
  `AvailableSpace::min_size()`, so there is no definite width to wrap against
  and a long title renders as one line most of the way across the display. A
  width on `gpui_component::tooltip::Tooltip` does not help either — it lands on
  the library's own flex row, and the text inside is still measured at max
  content. The width has to go on the container the text is a child of; see
  `controls::full_text`, where all three were built and only the third wrapped.
- Do not put a `flex_wrap` row of `min_w_0` children directly inside a flex
  column. gpui sizes it from its own content, and a line that can shrink to
  nothing measures one character wide — so it wraps one letter per line and
  paints over whatever is beneath. Wrap it in a `flex_row` first; see
  `render_event`.
- Do not assume `gpui-component` draws its own icons. It asks the *host* for
  `icons/<name>.svg` and ships none, so with no `AssetSource` every chevron,
  eye and clear button renders as nothing — and silently, since a missing asset
  is not an error anywhere in that path. The clickable ones are still there and
  still clickable, which is worse than absent. Perch's own controls draw from
  the same table now, by `assets::Icon` rather than by path, and an `svg`
  still needs its own `text_color` to draw at all.
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
- Do not ask a pane's questions through a browse list's request. A stopped
  pane's past broadcasts are `Request::Broadcasts`, not `Request::Videos`:
  that one lands on a channel page's shelf, ends the page's wait, and says its
  failure on the page, none of which a pane's answer should do.
- Do not give an element between the watch page and a pane's cell an id —
  the grid, its rows, the cell — nor key anything in a cell by its position
  or by what is maximized. gpui finds an element's state by the ids of every
  ancestor that has one, so maximizing a pane or showing them all again
  would remount every pane and replay its fades; a cell's ids name its pane
  alone (`watch::pane_id`, and the doc on `watch::page`).
- Do not give a slot a new key in place. Everything keyed on a pane — its
  element ids, its player's subscription, the active pane — would go on
  naming the old one; `replace_with_video` makes a new slot and swaps it in.
- Do not paint a backdrop, or a word, in a `VideoView` that has no frame yet.
  Whoever holds the player says what is happening under it, and the first
  frame fades in over that; a player of its own black hid the poster the
  moment it existed, seconds before there was a picture.
- Do not let two windows draw one `VideoView`, let the main window read one
  that is popped out while drawing (anything its render reaches), or move
  one by anything but `VideoView::set_place`. The second window would freeze
  on its first tile, the two probes would fight over the render size, and a
  window that reads a player while drawing is redrawn at its frame rate.
  What the main window draws reaches a player through
  `RootView::video_in_main` and a pop-out through `pop_out_video`, both
  answered by the one `Stage`; see the pop-out trap. Root code outside a
  draw still reads a popped player through `Slot::video` — `sync_quality`
  does, and that is how a pop-out's quality follows its own window — so do
  not swap those reads for `video_in_main`.
- Do not open or close a window inside a root update, or call a root method
  from a pop-out, except through `cx.defer` and `pop_out::to_root`.
  `open_window` draws before it returns, a pop-out's render reads the root,
  and a root method handed a pop-out's `Window` measures the wrong body and
  binds a stream's pump to a window about to close.
- Do not kill a pane's supervisor while its player is live — not to start
  a new rendition, not to tidy up a swap. The old mpv is reading that
  relay, hits end of file without it, and `stream_stopped` ends the pane and
  drops the new start with it. The old supervisor goes in `on_swapped`,
  once its player has stopped; see the swap trap under Video.
