# Handoff

For whoever picks this up next. `README.md` covers *using* it; this covers
*working on* it — the architecture, the traps, and the things that cost real
time to discover and would cost the same again.

Over 70,000 lines across seven crates, tests included. The checks under
"Checks" in `CLAUDE.md` are all expected to pass; if one does not, that is the
change you are looking at, not the baseline.

---

## What it is

A native Twitch client in one window: browse who you follow and what is on,
and watch up to four streams at once, each with its own chat; README's opening
says the rest. Rust + [GPUI](https://github.com/zed-industries/zed) (Zed's UI
framework), with streamlink as the byte source and libmpv doing decode and A/V
sync.

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

## Product decisions

Made with the user, most of them during the October 2026 UI overhaul, and
still in force. Each carries its reason, so a change that cuts across one is a
conversation with the user, not a cleanup. The overhaul's build order, live
checks and review catches are in git history; what it left open is under
"What to build next" and "Known limits".

**Who it is for.** The user is the primary user: Windows 11, a 4K horizontal
monitor at 100% scale with a vertical one beside it, and the mouse first —
`Ctrl+K` and the shortcuts are rarely touched. So:

- **Every feature needs a way in by the pointer; a key is an extra.** The
  omnibox that was to take the title bar's search box, arrow keys in menus, a
  quality key and rebindable keys were all deprioritised, and nothing new
  should need a new key. The omnibox came in as the search box's
  suggestions, a list to click under the box as you type, for the mouse
  rather than the keyboard. Adding a stream without leaving the watch page is
  the pointer's too, not the palette's: the rail's `+`, a go-live toast's
  `+ Add` and `Add beside` on a channel's right-click menu open it beside the
  panes (`Opening::Beside`; README's Browsing and "Right-click a channel").
- **The vertical monitor is a real setup, not an edge case.** Two streams
  there stacked with each chat beside a squeezed picture until somebody looked
  ("Known limits", item 8), so a layout change is looked at in a tall window
  (about 1080x1900) as well as a wide one.
- **Picture quality over CPU.** A CPU saving paid for in picture quality is
  not taken; find the part of it that costs no quality instead (see the
  Performance trap and "Things not to redo"). It is why a pop-out's quality
  follows its window upwards only, why panes maximized away keep their
  rendition, and why the render cap is 2160p rather than 1440p
  (`video::MAX_RENDER_HEIGHT`): the user chose a 4K stream drawn at its size
  on their 4K monitor over the CPU a 1440p render saved (14 October 2026).
- **No UI scale setting, and no list for hiding channels.** Both were offered
  in October 2026 and declined: the app looks right at 100% to the person who
  uses it, and nobody asked to hide anyone.

**Multiview.**

- **Sound stays separate per pane.** Each pane has its own level, remembered
  per channel; Mute all and `Only this one` are a session hush beside that
  level, never saved (see "Things not to redo").
- **Chat stays per pane, so there is no focus layout** — no one large pane
  with the rest as thumbnails sharing one chat. It is also why a maximized
  pane draws the others as nothing rather than as a strip or thumbnails (see
  "A pane given the window").
- **A pop-out that stays on top of other apps is in**, on Windows only: on
  macOS the window gpui would keep up hides whenever another app is in front
  ("Known limits", item 30).
- **Browsing stops what plays.** The browse page is a page of its own with
  the whole window, and leaving the watch page for it stops every pane at
  home; a pane in a window of its own plays on (see "The browse page" under
  "Where controls live"). The user's words, 22 October 2026: if they are
  looking for another stream, they want the current one to end. For a week
  the browse page docked beside the panes, which played on, and in use the
  user found it weird: one press of Browse shrank the stream to about a third
  of its area and moved its chat (see "Things not to redo"). The docked page,
  the mini player in the browse page's corner and its switch (the user
  usually closed it, and ran it switched off) and the guide over the watch
  page stay retired. A click on a card, a rail row or a go-live toast plays
  in place of the pane you are on (`Opening::InPlace`) rather than closing
  every pane.
- **A pane's details sit at the top of its chat.** On lines of their own,
  the picture centred, with the pane's own controls — the pin, `Past
  broadcasts`, out and back — behind a ⋯ that opens the pane's menu. The
  user chose it on 22 October 2026 over the details under the picture, whose
  viewing page they did not like (see "Pane header" under "Where controls
  live", and "Things not to redo").

**The rail and Following.** Popular and Categories left the rail and stay
browse tabs; Recommended came in, read from Twitch's unofficial `SideNav`
query, a risk taken knowingly ("Known limits", item 18). Popular, Categories
and Library stay tabs beside Following rather than links at its foot, so
each is one click away. Following (Home until the redesign, which also
renamed History to Library) groups the channels as the rail does, Pinned,
Live now, Recommended and Offline, with Continue watching after who is live,
so the two never disagree; its pins came with the redesign the user
approved.

**Voice.** The user left it to the developer: every word on a control is
sentence case, notices, toasts and tooltips read as sentences, and a pane
header's passive tags stay lowercase (`muted`, `paused`, `buffering`,
`replay`, `ad break`). The rule and its open corners are under "Casing".

**Features whose standing changed, and why.**

- **Sending chat** was ruled out early and reversed on 2 October 2026, as "a
  big feature missing". It goes through Helix with a scope of its own, so
  reading stays anonymous; an emote picker, moderation commands and whispers
  stay out (see "A live chat can send").
- **`Alt` + wheel for a pane's volume.** The modifier was asked for by name,
  so a wheel turned on the way past a video never changes its sound (see
  "Things not to redo"). Middle-click toggles mute.
- **Chat badges** were kept out of the comfort pack as a feature of their own,
  then built in the chat pack, in live chat and replays, signed in.
- **Desktop notifications** were first left out, since they touch packaging,
  then wanted on the condition that they can be turned off or kept to pinned
  channels: `Off`, `Pinned` (the default) or `All` ("Desktop notifications"
  under Windows packaging).
- **Clips** were not picked. Perch treats a clip as not a recording
  (`target::HOSTS`), and showing them would reverse that: the user's call.
- **Not recommended, for reasons outside perch:** polls and predictions need
  the streamer's permissions, following and unfollowing were taken out of
  Twitch's API, and chatter counts have no endpoint left (see "Known to be out
  of reach").
- **The chat round of October 2026** — emote kinds with their effects,
  cheermotes and a viewer card, picked on 14 October 2026 — is built (see "An
  emote can be drawn over the one before it", "Effects are drawn as BetterTTV
  draws them", "A cheer is drawn as Twitch draws it" and "A name opens its
  person's card", under Chat). The user's calls in it: effects are drawn,
  rather than their words hidden or kept as text, and only an effect nobody
  knows yet keeps the fallback they were asked about — its word hidden, the
  emote drawn plain, the tooltip saying `not drawn`. A cheer in a recording's
  chat replay stays the word typed: Twitch's copy does not say which lines
  spent Bits, and a guess would draw anyone who typed `Cheer100` without
  cheering. Nothing on the viewer card acts on the person — no hide or
  ignore, as no list for hiding anyone was wanted (above), and no moderation
  (see "Moderation and whispers" under "Known to be out of reach"). Asked
  whether its one link should go to twitch.tv or to Perch's own channel
  page, the user asked for both, with good UX for it: the card must never
  silently end what they are watching, so `Channel page` asks for a second
  press where leaving would stop streams, in the accent, as the user asked
  for where the app has no warning colour. `Channel page` is offered only
  signed in (`CardFacts::channel_page`), the user's call of 14 October 2026,
  since its lists need the token; `Watch` and `twitch.tv` work either way.

---

## Layout

```
crates/
  mpv-frames    libmpv loaded at runtime, software render to BGRA
  streamlink    supervises streamlink as a headless Twitch byte source: the
                relay, its word that it is filtering an ad (`ads`), the
                version check, and a recording's playlist
  twitch-chat   chat read over anonymous IRC, the history backfill, and
                the replay of a recording's chat
  twitch-api    device-code sign-in and the Helix the app reads and sends
                through, plus Twitch's unofficial GraphQL, asked anonymously
                (`recommend`)
  emotes        Twitch/FFZ/BTTV/7TV resolution, cheers, stacks, effects and
                the disk image cache
  settings      persisted user settings, and what has been watched, and
                the one thread the app writes both on (`writer`)
  perch         the app
```

Every crate except the last is free of UI types, deliberately — they are
testable without a window, and the video pipeline in particular was built
standalone before any GPUI existed.

App modules, one line each; each file's own `//!` says the rest:

| file | role |
|---|---|
| `main.rs` | the process: first perch or a launch to hand over, the window, where stderr goes |
| `instance/` | one perch per settings file: the claim, and a later launch handing its arguments to the running one |
| `launch.rs` | what a launch's arguments ask for, read one way at startup and on a handover (pure, tested) |
| `notifications/` | the desktop notification when a followed channel goes live with the window in the background, and its Windows toast; see "Desktop notifications" under Windows packaging |
| `trail.rs` | back and forward: the places behind and ahead (pure, tested) |
| `stage.rs` | where each pane is drawn — the watch grid, a window of its own, or nowhere while another pane is maximized — the one owner of that answer (pure, tested) |
| `os_window.rs` | what Perch asks of a window that gpui does not: its platform handle, and keeping a pop-out on top of other apps |
| `root/` | the app: `RootView` and its state in `mod.rs`, then one `impl RootView` block per concern, in the sibling named for it; `mod.rs`'s own table says what each owns. |
| `target.rs` | what a typed or pasted thing means, and `link`, the one place a twitch.tv URL is written (pure, tested) |
| `browse.rs` | the browse page: its tabs and search, which of them is on screen, and the stream card |
| `channel_page.rs` | one channel's page of past broadcasts, the recording card, and `archive_of`, which finds the recording of a broadcast (tested) |
| `history_page.rs` | the Library tab, the watch history, and the one translation between a video and a history entry |
| `following.rs` | Following, the tab the app opens on, grouped as the rail groups it |
| `veil.rs` | what the overlays on the watch page cover, so the panes' measured hover leaves the pointer alone there (tested) |
| `watch.rs` | the grid of panes and `Slot`, with its pending and warm starts, and `PaneAction`, everything a pane asks of the root |
| `watch/header.rs` | a pane's header: what it says, where it goes (`Placement`) and when it says `buffering` (`says_buffering`) (tested) |
| `watch/chat_menu.rs` | a pane's chat options menu (tested) |
| `watch/status.rs` | a pane with no picture: `Showing`, the one reading of its state, and the screen drawn from it (tested) |
| `layout.rs` | the grid's shape from the window's (`Grid::of`), the one grid every reader shares, and where a pop-out opens (pure, tested) |
| `video_view.rs` | the player element: its sound, its hover, the picture and the gestures on it, moved between places only by `set_place` |
| `video_view/swap.rs` | a rendition swapped in place, held to the stream on screen by `align`; `route`, the one rule a start's events reach their stream by (tested) |
| `video_view/bar.rs` | the control bar over a playing picture: the seek row or the live timeline, the icons, the quality pill and what fits (tested) |
| `video_view/menu.rs` | the bar's menus — the quality, More, a recording's speeds — and `set_menu`, the one write of the open menu (tested) |
| `loudness.rs` | one pane's level and the Mute all hush over it (pure, tested) |
| `channel_menu.rs` | the one right-click menu wherever a channel is named (pure, tested) |
| `compressor.rs` | the audio compressor, every pane's at once: the mpv `af` chain and the flag every render thread reads (pure, tested) |
| `wheel.rs` | `Alt` and the wheel over a picture, turned into volume steps (pure, tested) |
| `ad_break.rs` | a pane's ad-break notice: what it says and when it is over (`FrameClock`) (pure, tested) |
| `seek_bar.rs` | the seek bar on a recording and a live pane's timeline, and the arithmetic behind them (tested) |
| `catch_up.rs` | a live picture getting back to the edge: catching up after a stall, and the jump on unpause (`resume`) (pure, tested) |
| `speed.rs` | how fast a recording plays: the speeds More offers (pure, tested) |
| `rewind.rs` | rewinding a live pane, and the way back to live, worked out (pure, tested) |
| `video.rs` | the render thread, which owns the mpv `Player`, and the pane's `PositionHandle`, which every player of the pane and its chat replay share |
| `vod.rs` | positions a recording by rewriting its playlist; the keeper for one still growing |
| `chat/mod.rs` | the chat pane: rows, scrollback, what a row is painted with, and a message's line |
| `chat/emote.rs` | an emote in a line: its picture, or the one its effects make, the overlays stacked on it, its tooltip and its right press |
| `chat/room.rs` | what the room says: `ROOMSTATE`'s modes, and a timeout or ban greying its person's rows |
| `chat/badges.rs` | the chat pane's badges |
| `chat/cheers.rs` | the chat pane's cheers |
| `chat/reply.rs` | a reply's line above it (pure, tested) |
| `chat/copy_menu.rs` | a chat row's right-click menu (tested) |
| `chat/viewer_card.rs` | a person's card, opened by a press on a name or an `@mention` (tested) |
| `chat/composer.rs` | the box at the foot of a live chat that sends a message (tested) |
| `chat_effects.rs` | the pictures emote effects make, as a pane keeps them (pure, tested) |
| `chat_display.rs` | how every chat is drawn: its text size and what follows from it (`Metrics::of`) (pure, tested) |
| `chat_badges.rs` | chat badges worked out (pure, tested) |
| `chat_cheers.rs` | cheers worked out (pure, tested) |
| `chat_profiles.rs` | who a chatter is, worked out, for a viewer card (pure, tested) |
| `chat_words.rs` | what a chat says about itself in words (pure, tested) |
| `chat_tint.rs` | which wash a chat row wears (pure, tested) |
| `chat_text.rs` | what a word in a message is — link, mention or plain (pure, tested) |
| `settings_view.rs` | the settings sheet |
| `twitch.rs` | the worker: the one thread that owns the Twitch session, and every ask that rides it |
| `keys.rs` | the keymap: actions, bindings, contexts, the listing, and the keys a tooltip may name (`Hint`) |
| `theme.rs` | **all** colour, spacing, type and motion tokens |
| `sidebar.rs` | the follows rail down the left, beside both pages, and its groups (pure, tested) |
| `rail_preview.rs` | a live rail row's preview card (tested) |
| `last_live.rs` | when each offline follow was last live, worked out (pure, tested) |
| `schedule.rs` | when an offline channel says it is on next, worked out (pure, tested) |
| `shared_chat.rs` | Shared Chat labels worked out (pure, tested) |
| `recommended.rs` | the rail's Recommended group worked out (pure, tested) |
| `palette.rs` | the command palette, and the search box's suggestions (tested) |
| `controls.rs` | the one button and its variants, the tags, the live badge, the icon button, the headings and the tooltip builders |
| `widget_theme.rs` | hands `theme.rs` to `gpui-component`'s own palette |
| `assets.rs` | the icons, typed as `assets::Icon` |
| `motion.rs` | the two shapes of motion gpui allows, arrivals (`arrive`, `waiting`) and two-way changes (`Fade`, the one piece of state they need) |
| `diagnostics.rs` | where stderr goes when there is no console |
| `clock.rs` | the system's short time format, for chat stamps |
| `cpu_log.rs` | what the CPU was doing, sampled every five seconds |

**Threading model, used consistently:** anything blocking runs on a plain
`std::thread` and reports through a `futures::channel::mpsc`, which the UI drains
in a `cx.spawn_in` pump that calls `cx.notify()`. There is no async runtime. If
you add a network feature, follow that shape rather than introducing tokio.

**One thread owns the Twitch session, and it has to.** Refresh tokens are
single-use, so two things refreshing at once would spend the same token twice
and lock the user out. Every Helix read therefore goes through the worker in
`twitch.rs`, not merely for tidiness, and so do the anonymous GraphQL asks the
root makes (the rail's recommendations, when the offline follows were last
live, a Shared Chat partner's name); chat's own IRC, history and replay stay
with chat (`twitch.rs`'s `//!`). The worker takes requests on a
`std::sync::mpsc` channel and waits on `recv_timeout` against the next
follows-poll deadline — the wait and the mailbox are the same thing, so
browsing never queues behind the timer — and dropping the service drops the
sender, which wakes it at once rather than after the poll interval.

**And one process owns the settings file, for the same reason.** A second
perch on the same file was a second worker spending the same tokens — and a
second writer the `settings` crate's lock knows nothing about, since that lock
is per process. `instance` makes the first copy the only one: a later launch
hands its arguments over and exits. See "One perch".

---

## Area by area

One section per area of the app. The traps are the expensive part: most are
invisible until runtime.

### Video

**`gpui::surface()` is macOS-only.** The zero-copy video path does not exist on
Windows. Frames go through `img()` + `RenderImage`, which is CPU-side BGRA. Zed
does the same thing for screen share on non-macOS; that is where the pattern
came from.

**One `ImageId` per stream, not per frame.** `RenderImage::new` takes a fresh
monotonic id from a global counter on every call, and GPUI keys its sprite atlas
on that id — so a source that mints one `RenderImage` per frame is asking it to
build and destroy a GPU texture sixty times a second: on a maximised 1440p
pane, a 14.7 MB `CreateTexture2D` every frame. `video.rs` mints one id for the
whole stream and stamps it onto every frame instead, and `video_view.rs` calls
`Window::update_image`, which overwrites the tile's pixels in place; that verb
is a vendored patch, since `get_or_insert_with` is insert-once (fix 5 in the
root `Cargo.toml`). The call is guarded on `Arc::ptr_eq`, **not** on the id:
`render` runs on every *window draw*, not on every decoded frame, and every
frame of a stream shares one id, so an id comparison would skip every real
frame and freeze the picture. The comment over the guard in
`VideoView::render` has the rest.

**GPUI still does not refcount atlas tiles against the `Arc<RenderImage>`.**
`Window::drop_image` is the only thing that calls `sprite_atlas.remove`, so the
one tile a `VideoView` owns stays resident for the life of the window unless the
view hands it back — and the view dies on every ordinary action, including each
cold restart. `Drop` cannot do it; it has no `App`. `cx.on_release` can, and the
`Subscription` it returns has to be kept in a field (`_release`) or the hook is
dropped at once. It calls `App::drop_image(frame, None)`, which visits every
window, since the player may be drawn in a pop-out by then (the comment on it
in `VideoView::from_stream` has the rest). A rendition swapped in place keeps
the view, and `render` frees the old stream's tile instead.
`ChatView` keeps `on_release_in` for its emote cache: chat is only ever drawn
in the main window.

**And it leaks a third time inside GPUI itself, where `drop_image` cannot
reach.** This is the expensive one: 43.7 GB of committed memory in five hours,
54% of the machine's entire commit charge. `DirectXAtlas` rounds a new texture
up to 1024 in each dimension separately and then packs the next chat emote
into the rows a video frame's texture has spare, which pins that texture for
good; the comment on `[patch.crates-io]` in the root `Cargo.toml` has the
mechanism and the fix (fix 1). The tell is that it depends on the *source
resolution*, which is why it looks like magic until you measure it: beside a
1098p stream, a 936p one had leaked 8,731 textures and the 1098p one none, and
anything from roughly 90p to 960p leaks while 961p and up does not.
`scripts/verify-vendor.sh` proves the vendored tree is crates.io's gpui plus
exactly the hunks tagged `PERCH PATCH`, all eight fixes, and CI runs it before
anything slower.

**Fixing that exposes a second leak underneath it, in the same file.** `remove`
never returned a tile's space to the shelf allocator, so a *shared* atlas was
write-once, and with the first leak fixed the frames moved into shared
atlases, where it showed at once: two panes went from 491 live 4 MiB atlases to
891 in four minutes. Any frame that fits inside 1024x1024 hits it - a 2x2
layout, or any pane smaller than the default atlas. The fix is upstream's own
(Zed PR #58874), vendored as fix 2, with Metal's `remove` taking the key out up
front so that dropping one image twice cannot free a texture from under a live
tile.

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
starting screen under it, so a player mpv could not open would say "Starting…"
for good. The render thread fills `stopped` with `Stopped::Failed` and wakes
the frame channel, as an end of file does, so the pane offers `Try again`; and
so does every other way its loop ends on its own — a frame that fails to
render, a buffer that does not match its size, mpv's `Shutdown` — which would
otherwise leave the last frame up as if paused. A local `reported` keeps it
from saying again what an end of file already said, whose wake may have been
read already.

**A live picture waiting on the network says `buffering`.** mpv's
`paused-for-cache` goes up when its cache runs dry; the render thread observes
it on a live player only (a recording's jumps are reopens that wait a second
each by design, and its stalls have `vod::stalled`), and once it has been up for
`video::BUFFERING_AFTER` (a second) after the first frame, not paused,
publishes `VideoStream::buffering` with a wake of its own, since a picture that
is waiting sends no frame to carry the news. The header says it beside `muted`
and `paused`, and not during an ad break, which freezes the picture too and
says why (`watch::says_buffering`). A pane in a window of its own says it, and
an ad break's tag, as a badge over its picture (`pop_out_badge` says why). A
wake that says the waiting began or ended may carry no frame, so the ad
notice's sense of when the picture last moved is counted by the render thread
from the frames mpv says are new, offstage ones included and redraws never
(`ad_break::FrameClock`, `VideoStream::resumed_at`). Do not move it back onto
wakes: counted from them, it skipped every wake that changed `buffering`, and
stood still while the pane was maximized away, so a pre-roll that ended
offstage left its notice up to the cap. The header says nothing of a popped
pane's `muted` or `paused`; the pop-out's bar shows those under the pointer.

**A live picture that fell behind catches up.** After the network stalls,
streamlink fetches the segments it missed and hands them over at once, so mpv
resumes with the backlog in its cache and plays it at normal speed, behind for
good, with chat that far ahead of the picture. `crate::catch_up` plays a live
pane at 1.05 while its cache (`demuxer-cache-duration`) holds more than 3 s,
until it is under 1.75 s, as Twitch's player and FrankerFaceZ's `allow-catchup`
do, with mpv's pitch correction keeping the speech from rising: 6.3 s behind
came back near the edge in two minutes. Not a jump, which would throw away what
was said in those seconds; not while paused; and not on the reading a pause
leaves behind (`CatchUp::speed` says why, and what is left of that gap).
Measured by suspending a scratch copy's streamlink for eight seconds on a live
channel (`NtSuspendProcess` on the launcher and its Python). A pane's own speed
(`crate::speed`) stays a recording's only.

**A live pane's jump to the edge is a seek inside mpv's cache, or a
reopen.** A percent seek never works on the relay: the relay is not seekable,
so mpv seeks only within its cache, and a live stream has no length, so `seek
100 absolute-percent` failed on every unpause, on TS and fragmented MP4 alike
(`Player::seek_to_live` says why). While paused, mpv goes on reading the relay
into its cache, so the cache's end is the live edge and an absolute seek to it
is served at once: 0.5 s behind live after a twenty-second pause, where the
percent seek had left it 20.7 s behind and seven minutes of catch-up from the
edge. Once the cache is full (150 MiB, which a 1440p60 pause filled in 130 s)
mpv stops reading and its end falls behind, so an unpause after that opens the
relay's URL again (`Player::reopen`), which streamlink's relay answers by
resolving the channel afresh: 0.2 s behind live, about two and a half seconds
after the press, the paused frame up meanwhile. `catch_up::resume` decides,
from what the render thread observes of the cache, and has the numbers. To
test the reopen without waiting, hold the cache to 6 MiB: it fills in seven
seconds.

**`mpv_render_context_render` blocks until the frame's display time**, up to
`video-timing-offset` (50 ms default). That wait is what keeps video timed to
audio, so it is wanted — but calling it on the UI thread stalls the whole GPUI
frame loop. It runs on `video.rs`'s thread for that reason. A benchmark that
reads exactly 16.66 ms is measuring this, not CPU.

**A paused stream redraws when its size changes.** A paused player sends no
frames, so a paused pane that grew — the window made larger, a pop-out resized
or brought home, a swap promoted while paused — went on stretching its last
small frame until it played or seeked. With the pause applied and the size
asked for different from the last frame's, the render loop skips the wait and
renders at once: mpv draws the frame it holds again, without blocking
(`video::redraw_now` has when, and why never before a reposition's first
frame). Not yet seen in the app: that the software renderer's redraw is the
sharp frame rather than a black or stale one. If it is not, take the redraw out
and list a paused pane as soft after growing, until it plays or seeks, under
Known limits.

**So does any stream back from offstage, playing or not.** A player nothing
draws publishes nothing, so on its return its pane showed the frame from the
moment it went away until mpv next sent one, which in an ad break or a stall it
does not. The first pass back draws the frame mpv holds, the newest, and until
mpv sends a new one a size asked for is drawn again at once too, since the pane
asks for the size it comes back at only in its window's next draw
(`redraw_now`'s `just_shown`, `video::Redraws`). The same caveat holds: if the
redraw turns out not to be the sharp frame, this goes with it.

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
toggles, and once a run of window or pop-out resizes has settled (750 ms) —
and changes only a pane whose Auto or Fixed choice now names a *sharper*
rendition; a pane that shrank keeps what it has, and a choice made from the
pane's own menu is left alone. Changing rendition means resolving the stream
again, since streamlink cannot switch mid-stream; on a pane with a picture the
new one plays beside the old one and takes over in place (the next trap), so
it costs a second stream for a few seconds rather than any black: worth it for
a picture that was soft, not for a CPU saving in a pane that hides nothing —
which is also why the re-pick waits for a resize to settle rather than running
per event. Each pane is measured on its own (`RootView::pane_height_for`): its
cell of the watch page's grid with the seams left in, a pane in a window of
its own by that window, a maximized pane by the whole body, and the panes
maximized away by the grid they come back to, so the maximize never moves
them. Those go on decoding their rendition, which is the quality rule's price,
but render small into a buffer nobody reads and publish nothing
(`VideoStream::set_offstage`): with two live 1080p panes, a minute maximized
took the process from 116 to 104 percent of a core. The docs of `sync_quality`
and `pane_height_for` have the rest.

**A rendition change keeps the picture: the new stream is swapped in under
it.** When a pane whose picture covers it changes rendition — the re-pick
above, or a pick from its menu — `RootView::change_rendition` starts
streamlink again *beside* the stream on screen (`How::Beside`, kept as
`Slot::pending`) and leaves `Slot::supervisor` running. **Never kill a pane's
supervisor while its player is live:** the old mpv is reading that relay,
and with the relay gone it hits end of file, which `stream_stopped` reads as
the stream ending — it retires the pane and drops the new start with it. Every
start is numbered from one process-wide counter (`Slot::generation`), and
`video_view::route` sends each streamlink event and frame wake to the stream on
screen, the one getting ready, or nowhere, so a superseded start's last events
cannot reach the pane. Once it resolves, the new player starts inside the same
`VideoView`, silent and publishing no position, and `swap::align` holds it to
the one on screen: a live stream takes over on its first frame, and a
recording, started `SWAP_LEAD` ahead of a playing pane, once the pane has
reached it. Taking over (`promote`) carries the pause, the level and a seek
the old player had not applied yet; only then does the root kill the old
streamlink (`on_swapped`). One whose stream stops, or that is not done in
`SWAP_CEILING` (45 s), leaves the old player playing (`SwapFailed`). swap.rs's
`//!`, `align`, `promote` and `SWAP_LEAD` have the rules and how to tune the
three estimates from the log (see "A swap says what it did in the log").

A pick is seen to be under way from the press, since with nothing changing on
screen for the seconds a resolve takes, a working switch was taken for a broken
one: the pill names the rendition picked and breathes, More's folded row says
`Quality · switching to 480p30`, and the bar stays up until
`theme::SWITCH_LINGER` after the switch ends either way (`bar::pill_face`;
`RootView::set_pending`, the only write of `Slot::pending`, tells the player).
A re-pick is silent throughout, and one that fails waits for the next growth,
with no retry loop. Still cold: opening, `Try again`, the settings sheet's
quality and credential restarts (a swap per pane there would be two players and
two Twitch sessions per pane at once), and a pane whose picture does not cover
it yet. Not yet known: whether Twitch serves two live sessions on one token
("Known limits", item 3), and what the audio does at the moment of taking
over, which cannot be captured.

**Where a switch's seconds go, and the start ahead.** A pick took 5.7 to 6.2 s
from the press to the take-over in the user's log, of which only about 0.6 s
came after streamlink said it was ready. Measured with the streamlink CLI
(8.5.0) run as perch runs it, each output line timestamped from the spawn
(gdqtv, lirik_247, ironmouse, anonymous): a pick ran *three* streamlinks one
after the other — `--version` to find the binary, 450 to 460 ms; the `--json`
probe, 1.8 to 2.0 s (3.1 s the first time); and the serving run, 1.65 to 1.73
s to `Starting server`, which is about 450 ms of interpreter and imports, 200
ms of plugin, and a second of Twitch's access token and multivariant playlist.
So the binary is found once per session (`streamlink::find_binary`), and a
start beside a live picture names its rendition from the pane's menu
(`RootView::beside_rendition`) and skips the probe (`StreamOptions::offered`):
through the crate, spawn to `Ready` went from 4205 ms (3363 ms with the binary
already found) to 1659 to 1770 ms. The serving run then fetches nothing until a
player connects (at `-l debug` it logs nothing for five seconds after `Starting
server`, and the first byte comes 165 to 260 ms after the GET), and idle holds
about 45 MB and no CPU, so opening a live pane's quality menu starts it ahead
for up to four renditions, and a pick of one finds it ready (`root/warm.rs`,
whose `//!` has the rules). The `starting … beside …` log line ends `(ready N
ms after the pick)`, the part this cuts. Not yet measured: the same in the
app, with the auth-token header, from the log.

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

**The compressor is fitted to the browser, not copied from FrankerFaceZ.**
FFZ's settings (threshold −50 dB, knee 40 dB, ratio 12, attack 0, release
0.25 s) are a `DynamicsCompressorNode`'s, and mean something else to ffmpeg:
Chrome's threshold is where a 40 dB knee starts and ffmpeg's is the middle of
one at most ±9 dB wide, so the same −50 dB compresses far harder; and Chrome
adds +12.45 dB of makeup of its own where ffmpeg adds none. Taken literally,
the numbers need about +32 dB of makeup to sound loud enough and still come
out 3 to 11 dB from FFZ: five speech levels some 40 dB apart squeezed into
5 dB where FFZ leaves 20, and the pauses between words 20 dB above FFZ's.
`crate::compressor`'s chain is fitted to Chrome's own output instead, and a
change to it is checked the same way. Render clips through a
`DynamicsCompressorNode` with FFZ's settings in an `OfflineAudioContext` (the
browser pane can, on a page served from `localhost` that `PUT`s back what it
renders); play the same clips through libmpv with `ao=pcm`
(`ao-pcm-file`, `ao-pcm-waveheader=no`, `audio-format=float`, `vid=no`) and
the chain as `af`; compare the two envelopes every 20 ms. libmpv's output
matches `ffmpeg -af` with the same graph to the sample, so the CLI does for
sweeping parameters. The test `the_chain_follows_ffz_in_the_browser` holds
the makeup to Chrome's formula and the curve to within 2 dB of Chrome's; the
lookahead and the timing only the measurement sees.

**FFZ's compressor hears the stream at the player's volume, and so must
perch's.** Twitch's volume is the media element's, which a
`MediaElementAudioSourceNode` hands on already turned down, so the browser's
compressor works on the stream as loud as it is heard — and how hard it
squeezes depends on that: under a voice, music dips about 9 dB at full volume
and about 1 dB 23 dB down. With mpv's volume only after the chain, every pane
was squeezed as FFZ squeezes at 100%, and the user heard the background drop
whenever a streamer spoke. The chain now hears each pane at mpv's own gain for
its volume (`compressor::drive`), with the volume still applied after it,
which is FFZ's arrangement exactly; `compressor::Applied` says how a new volume
reaches a playing chain without building it again, and why its words are kept
up. To measure ducking, mix the background into one channel at −60 dB beside
the full mix in the other: it never decides the gain, and turned back up it is
the background as heard.

**mpv starts a file with no sound at all when it cannot build its `af`
chain.** Its log says `Audio filter initialized failed!`, then `Audio: no
audio`, and the pane plays its picture in silence. Set while sound plays, a
chain mpv cannot build leaves the sound playing unfiltered, but the property
keeps it, so the next `load` — a recording's reposition — starts silent too
(mpv 0.37, measured). So the compressor's chain sticks to what FFmpeg 4.3
(2020) and mpv 0.29 already had (`compressor::chain`,
`Player::set_audio_filters`).

**`sidechaincompress` starts its clock at zero, so the chain carries mpv's
`fix-pts`.** ffmpeg's `sidechaincompress` stamps the sound it puts out with a
count of samples from zero, whatever it was given, and until mpv is ready it
clips away any sound stamped before the picture's start. An MPEG-TS stream
reaches mpv with a clock near zero, so nothing showed; a channel served as
fragmented MP4 (an `#EXT-X-MAP` in the media playlist: 2 of 6 live ones checked
on 2 November 2026) keeps the broadcast's clock, hours in, so with the
compressor on the sound was clipped whole, mpv never said `audio ready`, and
the picture stood still after its first three frames — every run, and never
with the compressor off. `fix-pts=yes` on the `lavfi` wrapper puts the
stream's timestamps back on what comes out (`compressor::KEEP_TIMESTAMPS`,
which has the sources; mpv 0.29 and later). To measure anything like it
without the UI: serve the channel with streamlink's relay as `streamlink::run`
does, open `http://127.0.0.1:<port>/` with `mpv-frames` the way `video.rs`
does (`audio: true`, the chain as `af`), count the frames each second, and
read mpv's `log-file`.

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
  to play one after another — and never hands it to the HLS demuxer, so
  `demuxer=lavf` forces ffmpeg's; and ffmpeg follows a local file to the
  network only when told, by `protocol_whitelist`, quoted with mpv's `%N%`
  length prefix since its value has commas in it. `Recording::mpv_options`
  builds both.
- **The demuxer keeps the playlist file open, and Windows will not rename over
  an open file.** So every reposition writes a new file and deletes the one
  before once `FileLoaded` says the new one is in, and each player writes under
  a number no other player of the run has (`vod::playlist_label` says why).
- **A broadcast still being recorded is a playlist with no end marker that grows
  by a segment every ten seconds.** ffmpeg re-reads such a playlist, a *file*
  too, when it runs out of segments, so the keeper thread *appends* what is new
  to the file the player is reading, and `#EXT-X-ENDLIST` when Twitch does.
  `live_start_index=0` keeps the demuxer from starting three segments from the
  end, and the rewritten playlist says `#EXT-X-PLAYLIST-TYPE:EVENT` while it
  grows, or ffmpeg drops the start offset and every jump lands on a ten-second
  boundary (`Playlist::render_from`).

A reposition is known to have landed by mpv's `path` property changing to the
new file, not by the first `MPV_EVENT_FILE_LOADED`, which with two quick
repositions can belong to a file already replaced (`Recording::now_playing`).
The position the bar shows is the seconds before the current file's first
segment plus `time-pos`, and the target is stored the moment a seek is asked
for, so the bar does not sit on the old position while the player reopens.
mpv's `duration` is not read at all — it is the length of the file being read;
the length comes from the playlist (`vod::Extent`), grown by the keeper.

**A player reports its position to itself, and to the pane only while it
publishes.** The pane's shared position, which the chat replay, the history
and a link to the moment read, follows a player only once it publishes: a
pane's one player does from the start, and a player started beside it for a
quality swap only as it takes over (see "A rendition change keeps the picture"
under Video), since its position moving would otherwise reset the replay to a
moment nobody is watching and note that moment in the history.
`video::Positions` has the rule. Such a player is also handed the pane's
`SizeHandle` (`StartOptions::size`), so two players of one pane follow one
probe and the second renders at the pane's size from its first frame.

The replaced file's end arrives as `MPV_EVENT_END_FILE` with reason `Stop`,
which `Stopped::from_end` already ignores — the same reason a closing pane
sees. `Eof` is the only end that finishes a recording, and the pane reads it
as "Finished" rather than "ended the stream" by looking at what it was playing
(`watch::showing`).

`mpv-frames` deliberately has no `seek` for a recording: nothing could use it,
and a command nothing can use is a trap (`Player::load` says why).
`seek_to_live` is a live stream's jump to a time inside its cache (see "A live
pane's jump to the edge" under Video).

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
broadcast's `started_at` to now, with the recording's own seek bar; a press
let go back along it finds the broadcast's archive and replaces the pane with
it at that moment (`replace_with_video`, as `Watch from the start` does), so
from then on the pane is an ordinary recording. The rules are `rewind.rs`, pure
and tested, whose `//!` has what took deciding; the root's side is
`root/rewind.rs`; the drawing is `video_view::bar::live_row`; README's "Going
back in a live stream" is what the user sees. Two traps in it:

- **Trap: after a restart, the broadcast before is still "going on".** A
  broadcast that ended at 18:00 is listed as going on until within
  `STILL_GOING_SECS` of now, and may still wear the placeholder picture, so
  minutes into a restart, before Helix lists the new archive,
  `channel_page::archive_of` alone picks the old one, and a stale
  `Slot::broadcast` names it outright: the wrong recording, opened at its end.
  So `rewind::archive_for` first drops every archive whose listed end falls
  more than `BEGUN_SLACK_SECS` before the start the press carries.
- **Both asks share one answer type, routed by order.** A rewind and a stopped
  pane both ask `Request::Broadcasts`, which the worker answers in the order
  asked, exactly once each; `on_broadcasts` gives an answer to a pane's rewind
  before its `archives`, and first lets go by as many answers as the pane
  forgot when it began to play again (`Slot::stray_answers`), or a list from
  before the new broadcast would answer the new ask.

**And `LIVE` on the recording goes back to the live edge.** A pane playing
the archive of a broadcast that is still going on — rewound into, or opened
from a channel's page or the history — has `LIVE` at the right-hand end of its
seek row, and a press replaces the pane with the channel, cold, at the live
edge with the live chat (`RootView::back_to_live`). It is the live badge's
shape outlined in the bar's on-video colours rather than filled
(`controls::back_to_live`), so red only ever says a pane is live and never
"press to go live". Whether it shows is read off the live lists the app
already keeps, never asked, weighed against what the pane knows of where it
came from (`rewind::back_to_live`), and only while the archive's playlist is
still growing.

**No return to live by itself at the end of the archive.** A rewound
recording that catches up with what Twitch has written waits there, as any
recording still being made does near its end (`vod::EDGE`), until the playlist
grows. It does not jump back to live: the pane would change under somebody who
paused to read chat, and the end of an archive still being made is a few
seconds of buffering, not an end. `LIVE` is the way back.

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

**A recording's muted stretches are marked on its seek bar.** Helix's
`muted_segments` — the stretches whose sound Twitch took out for music it
matched — ride the pane's video to its player (`VideoView::muted`), and
`seek_bar` draws each as a band behind the rail (`muted_spans`) and puts
`muted` after the time under the pointer over one (`muted_at`). Behind the
rail, not over it: no single colour drawn on top measures apart from both the
opaque teal fill and the white-washed rail at the three-to-one a part of a
control needs, so what reads is the band's edges (`theme::seek_muted`, held to
it by `the_muted_band_reads_on_the_bar`). What Helix already said is reused,
and only a recording it has said nothing about this session is asked about,
once, with the request a link is looked up by (`Request::Video`);
`root/muted.rs`'s `//!` has how, and how an answer is told from a link's
(`Muted::claim`).

**A recording can play faster or slower, per pane.** More on a recording has
`Playback speed · 1x`, which opens the speeds, `0.75x` to `2x`, in More's
place; README's Past broadcasts has the rest of what the user sees. Nothing
on a live stream, which plays as it is made. The speed is the pane's, on its
`PositionHandle`, so every player of the pane, a swap's included, plays at it
from its first pass, and a pane that plays something else is a new slot, at
`1x`; it is never saved (`speed.rs`'s `//!`). mpv's `speed` is a player
property, so it holds across every reposition, and its pitch correction keeps
voices at their own pitch (`Player::set_speed`). Two things that read the
clock had to read the speed too: the swap's lead is four seconds of the clock
(`swap::lead`), or every swap at `2x` fell behind and reseeked; and the chat
replay's seek test (`replay::is_seek`) took three seconds of a slow request at
`2x` for a jump, and reloaded the chat for nothing.

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
An early benchmark had `hwdec=auto-copy` losing — 79.4% → 92.1% of a core at
1080p — on the argument that the GPU→CPU readback costs more than the decode
saves. A later pair of 3.5-minute steady-state runs on a 936p60 stream, read
from `cpu_log`, had it *winning* by a third, 146.6% → 97.1% of a core; the
comment on `video::hwdec_requested` carries the numbers. It is quality-neutral
— the same bitstream through a fixed-function decoder is the same frames —
which is why it is worth taking, and `auto-copy` falls back to software on its
own where a codec or driver cannot do it. The lesson is the method: a short
A/B is not a measurement here; use the CSV over alternating multi-minute runs.
`PERCH_HWDEC=0` turns it off on a machine whose decoder misbehaves.

To repeat a measurement like it: `PERCH_CPU_LOG=1 run.cmd <channel>`, leave it
for a few minutes, then again with the setting under test flipped, and compare
steady-state rows of the CSV rather than a moment of each.

**Always benchmark `--release`.** Debug builds are several times slower at the
per-frame format conversion, and their UI thread sits at a whole core on its
own, so nothing measured in one says anything. `cargo build --profile
profiling -p perch` gives an optimized build with symbols in its own
directory, which a test copy can run while a user's release build is left
alone.

**Where the UI thread's time goes, measured** (14 October 2026: an optimized
build, two live 1080p panes side by side with busy chats, a throwaway timer
around each render): 289 ms a second, of which chat rows took about 20, the
video views 24, `RootView::render` 9, and gpui's own layout, prepaint, paint
and present about 234. So a cache of each row's tokens would save two percent
of a core at most and was not built; if the UI thread is ever worth cutting,
the lever is the element count gpui lays out — a chat row is an element per
word, for the reasons under Chat — not the work done to build them.

**`perch.cpu.csv` is for the CPU questions — when asked for.** With
`PERCH_CPU_LOG=1`, perch samples itself every five seconds into
`%LOCALAPPDATA%\perch\perch.cpu.csv`; `cpu_log.rs`'s module doc has the
columns and why it is opt-in. Its first run is the baseline: the follows
page idle at 0.3–2.5% of a core and **0** renders a second, since gpui does
not repaint a page that has not changed, and one stream on the watch page
about 110% and 120 renders a second — all of it the stream, which mpv
decodes at its source's resolution whatever size the pane is drawn. With a
pane popped out, `visible` and `renders_per_s` describe the main window
alone; `focused` counts a pop-out too.

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

**An ad is a frozen picture, and only streamlink's log says so.** While it
filters an ad streamlink sends the relay nothing, so mpv runs dry a second or
two later and holds its last frame. The twitch plugin logs the break at info,
once, and nothing when it ends; `streamlink::ads` has the lines and matches
them loosely, in the same loop that waits for `Starting server`, and sends
`StreamEvent::AdBreak { secs }`. The serving run passes `--loglevel info`,
since `Starting server`, which a start waits on to say it is ready, is logged
at info too, and the command line is read after the config file, so it wins —
over `loglevel` only. A `quiet` in the user's streamlink config silences the
log whatever the level, and a `logfile` sends it to that file; no argument
takes either back, and `--no-config` would drop their whole streamlink config,
`config.twitch` and all, which is theirs. So the wait for `Starting server` is
bounded instead (`streamlink::READY_WAIT`, a minute, against about two
seconds), and past it the pane fails with a sentence naming `quiet` and
`logfile`; a `quiet` silences `--json` too, so a start that probes fails
sooner, naming `quiet`. The end of a break has to be inferred: the header's
tag goes when its length runs out or when the picture moves again after
standing still for `ad_break::FRAME_GAP` since the notice (see "A live picture
waiting on the network says `buffering`"), and after `ad_break::NO_LENGTH_CAP`
with neither. A start resolving beside the picture ignores its own ad lines,
and a swap ends the notice.

**`--twitch-supported-codecs h264,h265,av1` is required** or Twitch's higher tiers
(1440p, 4K) are silently absent from the quality list — they ride on HEVC/AV1 and
streamlink filters to h264 by default.

**streamlink 8.0 is the oldest that runs.** 8.0.0 (11 November 2025) added
`--twitch-supported-codecs`, which every run passes, and fixed the access
token after Twitch retired the persisted query older releases asked for.
`streamlink::version` reads the `--version` that finding the binary already
runs, passes over a candidate below 8.0 for the next one (so an old copy on
`PATH` does not hide a current install), and if none will do says `streamlink
7.5.0 is too old. Update it to 8.0 or later.` A failure an update is the fix
for — `PersistedQueryNotFound`, or a client-integrity token asked for — ends
with `Twitch has likely changed under streamlink: update it.`
(`streamlink::with_advice`), read in the streamlink crate where its other
words are, so nothing in the app matches on them. README and the RUNNING files
recommend 8.6.2 or later, for the security fixes in 8.4.0, 8.6.0 and 8.6.2
(the streamlink changelog, read 14 October 2026).

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

Unrelated credentials, easy to confuse: the **Client ID**, a public app id
from dev.twitch.tv (Client Type **Public**, no secret, because sign-in uses
device code flow and nothing secret can live in a desktop binary), and
**auth-token**, the twitch.tv **cookie**, a **full account credential**
stored in plain text, which is a documented tradeoff, not an oversight.
README's "The two Twitch tokens" and `settings::Credentials` say what each
one gets.

Neither can do the other's job. Refresh tokens are **single-use** — persist the
new one immediately or the next launch is locked out. A save that fails is
not let go: `keep_session_fresh` tries it again at every poll and before
every signed-in request the worker serves until it lands, and says once
that it could not (`TwitchEvent::SignInUnsaved`, a toast) — a session from a
refresh or a sign-in at startup as well (`establish_session`'s `unsaved`).
Every save of the file is flushed to the disk before its rename
(`settings::write_atomically`), since each writes the tokens back. And a
public client's refresh token — perch's is one, since the device flow has
no secret — expires 30 days after it was issued ("Refreshing Access Tokens"
on dev.twitch.tv, read 14 October 2026). Each refresh issues a new one, and
perch refreshes whenever the access token is within five minutes of running
out (`twitch_api::needs_refresh`), so the clock starts again whenever perch
runs; one left closed for longer than that starts the device flow again,
through the "sign-in expired" path in `establish_session`.

**Nothing that can fail may sit between the token swap and the save.**
`twitch_api::refresh` takes the user's id and login as arguments rather than
asking Twitch: a second request after the swap could fail on a dropped
packet once the old refresh token was already dead, and two seconds of bad
wifi signed the user out for good (its doc has the story). Only first-time
sign-in looks the user up, where a failure costs nothing because there is no
predecessor token to lose.

**`slow_down` is an instruction, not a synonym for "pending".** RFC 8628 §3.5
requires adding five seconds to the poll interval each time the device flow
returns it (`twitch_api::Error::SlowDown`). A sign-in window also runs for
minutes, so `Error::Network` during one is retried until the deadline rather
than tearing the worker down (`establish_session`).

**Both followed endpoints paginate.** `/streams/followed` caps at 100 like
`/channels/followed` does. Fetching one page of it silently truncated the live
list, and because `on_streams` replaces `known_live` wholesale, a channel
hovering around rank 100 dropped out and returned on alternate polls — firing a
went-live toast every time it came back (`twitch_api::MAX_FOLLOW_PAGES`).

**Two threads write `settings.json`.** The sign-in worker persists OAuth tokens
as it gets them; the UI writes its copy of `Settings` back whenever a
preference changes. Saving that copy wholesale put `oauth` back to whatever
it was at startup, erasing a fresh sign-in or restoring a refresh token
Twitch had already spent. So each writer keeps the field the other owns —
the UI saves through `Settings::save_preferences`, which keeps the sign-in
it finds, and the worker through `Settings::save_sign_in` — and every write
holds `settings::FILE_LOCK` across its read and its write, since both are
read-modify-write. `save_forgetting_sign_in` is the deliberate exception, for
a new client id. `saving_preferences_keeps_a_sign_in_written_since_launch`
fails if `save_preferences` is swapped back to a plain `save`, and
`two_writers_never_lose_each_others_fields` hammers the two from two threads;
without the lock it fails within a few hundred rounds.

**The UI never writes a file itself.** Every write is flushed before its
rename, and a flush waits on the disk, so each of the UI's saves is a copy
handed to `settings::writer`, one thread that writes them in the order they
were made (`root::saving`). Two things wait for it: a worker reads the file
only once what was handed over before it started has landed
(`settings::writer::Landed`, at the top of `twitch::run`), so a Sign out or a
new client id is on disk first; and the window's close waits for every save
(`RootView::finish_saving`), the one place the UI waits on the disk. The
`settings::writer` module doc has the rest.

**The settings sheet saves only what it owns.** It is handed
`settings::SheetFields` — the client id, the auth-token cookie, quality, chat
history and desktop notifications — and hands the same type back to
`Settings::adopt_sheet`, which takes those five and nothing else, so whatever
the app writes outside the sheet while it is up, now or in a later build,
survives saving it. A field the sheet gains does not compile until both ends
handle it: the panel builds the type with a struct literal, and
`adopt_sheet` takes it apart with no `..`.
`adopting_the_sheet_keeps_everything_it_does_not_own` holds it to exactly
that, pins, volumes and placement included.

**The second scope, and signing out.** `SCOPES` asks for `user:read:follows`
and, since sending chat, `user:write:chat`. A refresh keeps the scopes a
sign-in was granted, so a sign-in from before the second scope never gains
it, and nothing may assume it has. The worker asks Twitch's token validation
at its first poll and hourly after, which Twitch requires of every app
anyway, and a send refused for want of the scope says so too
(`TwitchEvent::ChatScope`, `RootView::chat_scope`); until one has said, the
composer stays open and a send finds out, and once one has, every composer
closes with `Sign in again to chat`. Everything else keeps working on the
old token.

Sign out, Sign in and `Sign in again` are `root/account.rs`, whose module doc
has the order they rest on: the worker is dropped before the tokens are
forgotten, and it reads its `stop` flag under the file's lock
(`Settings::save_sign_in_if`), so a refresh in flight either lands first and
is cleared after, or writes nothing; read before the lock, its tokens came
back the instant after Sign out cleared them
(`a_sign_out_is_never_undone_by_a_refresh_in_flight`). A new client id saved
from the sheet stops the worker first for the same reason. `SignedOut` is
for the session: the next launch finds no tokens and starts the device
flow, as a first launch with a client id does; a flag in the file to stay
signed out was not worth a settings field. `Sign in again` is not sign out
and sign in: it goes straight to a new device code (`SignInStart::Fresh`)
and leaves the stored tokens on disk until the new session is written over
them, since they still do everything but chat (`RootView::sign_in_again`).

**A failed sign-in is answered with `Sign in`.** `SignIn::Error` is the
worker's end — the code ran out, Twitch was out of reach with the access
token spent, a refresh refused mid-session, the worker's own panic — and
Following's empty state, the sheet and every composer offer `Sign in` for
it, which starts a worker from the stored tokens and asks for a new code
only when Twitch turns them down (`RootView::start_sign_in`). The empty
state has `Open settings` beside it, for a client id Twitch will not take,
which signing in again only repeats. A send the worker never got to does not
go out after a sign-out or a restart, and one already on its way is told
`Message may not have been sent` (`RootView::stop_twitch`, `twitch::run`).

**An older build drops what it does not know.** Every save writes the fields
the running build has and nothing else: serde ignores an unknown field on load,
so it is never written back. Run a build from before `pinned` existed after
this one — `instance` stops two copies at once, not one after the other — and
its first save writes the file without the pins. Nothing in this build can
prevent that, and a field added later costs the same against this one. Keeping
fields a build does not know would take a `#[serde(flatten)]` catch-all on
`Settings`, which nothing has needed yet.

### GPUI / gpui-component

**`gpui-component 0.5.1` differs from its main-branch docs.** Read its source
in `~/.cargo/registry/src/*/gpui-component-0.5.1/` rather than the online
guide. Known differences: masking is `InputState::masked(bool)` not
`Input::content_type`; `selected_index(cx)` takes the app context.

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

Worth keeping in mind:

- **Occlude the smallest thing that is actually opaque.** Put it on a container
  and you block its whole bounding box, including empty space. Toast cards are
  occluded individually for that reason.
- **`block_mouse_except_scroll` for an overlay the wheel should pass**, so it
  still reaches what is underneath — a toast card, chat's jump-to-live pill, a
  pane's header band over its picture; plain `occlude` for a modal, where the
  page behind should not scroll either.
- **What lies over a picture, and the wheel.** `occlude` stops the hit test
  just past its own box, so an occluder still hears the wheel itself and what
  is under it does not. So `Alt` and the wheel are hung on each thing over
  the picture that blocks the pointer: the control bar hangs
  `VideoView::on_wheel` on itself (`control_bar`), and a pop-out's drag
  layer, which must keep `occlude` for its caption drag, carries the
  picture's gestures (`VideoView::picture_gestures`). The header's band over
  a pane's picture is `block_mouse_except_scroll`, so the wheel there goes on
  to the picture; that is safe only where no window control lies under it.
  Only one element hears any one wheel: `on_wheel` stops it going further
  while `Alt` is held.
- **A `WindowControlArea` under a `block_mouse_except_scroll` overlay still
  answers the window-control hit test**, as the bullet below on the root's
  content wrapper says too. So whatever must stay pressable over a drag layer
  occludes: a pop-out's bar on `block_mouse_except_scroll` made every button
  on it a caption (the comment in `control_bar`).
- **On Windows the wheel's modifiers are read when the message arrives**
  (`current_modifiers`, `GetKeyState` in gpui's windows/events.rs), so
  `ScrollWheelEvent::modifiers.alt` is true with `Alt` held, over a client
  area or a caption alike, and a wheel there always arrives as
  `ScrollDelta::Lines`, never pixels; `crate::wheel` leans on both and says
  how. `Alt` let go after the wheel does not put the window in its menu loop:
  gpui answers every `WM_SYSKEYUP` itself. A middle press on a caption
  arrives as `WM_NCMBUTTONDOWN`, which gpui dispatches as an ordinary
  `MouseDown`, and Windows drags by the left button only.

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
drawn. Occlude only while visible, as the player's control bar and
`motion::Fade::apply` do.

**An occluder hides presses from the root's `track_focus`**, which is
hover-gated like any mouse-down listener (div.rs:2025-2037), so whatever
occludes over the watch page has to hand the keys back itself, or a cursor
left in the search box keeps them and `Space` types a space. The control bar
and the player's menus do it with `capture_any_mouse_down`
(`VideoView::return_keys`), in the capture phase on purpose: gpui-component's
`Slider` thumb calls `stop_propagation` on its press
(gpui-component-0.5.1 slider.rs:464-466), so a bubble-phase handler on the bar
never hears a drag of the volume.

**A press on a pane takes the keys, except into a text box in it.** Every
pane action takes focus back for the root first (`RootView::on_pane_action`),
and a press anywhere in a pane is one (`PaneAction::Activate`, from the
cell's bubble-phase `on_mouse_down`). A chat's composer, a text box inside
the pane, hears the press first, and its `track_focus` marks it handled
(`window.prevent_default()`, div.rs:2030-2035), so an `Activate` that arrives
with `window.default_prevented()` set still makes the pane active and leaves
the keys where they went. Nothing else in a pane prevents default today (no
gpui-component button or checkbox is used); one that did would keep the keys
from the root the same way.

**A menu's rows act on the press, never on a click.** A menu dismisses itself
with `on_mouse_down_out` on its anchor, which fires in the capture phase when
the press is outside the anchor's *own* bounds (div.rs:226-236), and a menu
hanging above its button is outside them. So a press on a row closes the menu
first, and the click gpui would make at the release comes from a frame with
no row in it (div.rs:2213-2245). The press is dispatched against the frame it
landed on, so `menu::menu_row` acts in `on_mouse_down`; its doc has the rest,
and a test reads `menu.rs` and fails on an `on_click` in it. The box itself
occludes, since it rises out of the bar's own hitbox, or a press on a row
would also reach the picture's double-click.

**The second press of a double-click lands on whatever is there by then.**
The platform counts a press near the last one as the next of a run by time
and place alone (windows/window.rs:1049-1060), and hit-tests each against the
frame on screen. So whatever acts on a press and moves what is under the
pointer acts only on a run's first press, and a window-level guard stops the
rest of the run in the capture phase, which also leaves its release no click
to make: a player menu's rows mark the player's guard
(`VideoView::run_guard`, in `video_view/menu.rs`), and `Pop out`, every pane
menu row and a viewer card's controls take the root's (`take_rest_of_run`,
`RootView::run_guard`); their docs say what the second press would have
hit. A pane's press making it active guards against the same platform rule
(`watch::pane`). A control that stays put and only toggles takes no run,
like every other toggle: a double-click there is two presses of it.

**A toggle that a press outside also closes has to know that press closed
it.** A chat's viewer card closes on a press outside it (`on_mouse_down_out`,
in the capture phase, div.rs:226-236) and on its person's name pressed
again, so a press on the name closes the card first and the name's own
listener, in the bubble phase, would open it again. So the close notes whose
card it was (`chat::viewer_card::Cards::press_outside`), and the name,
finding its own login there, leaves it closed. The `chat::viewer_card`
module doc has the listener order this rests on, read from the vendored
source and held by `the_same_name_closes_its_card` as far as a test without
a window can; if the order ever changes, the failure is the card opening
again, not a lost press.

**A tooltip keeps the words it came up with.** gpui builds the tooltip's view
once, when its delay runs out, keeps it in the element's state, and sets that
same view on the window every frame the pointer stays (div.rs:1662-1664,
2279-2316); the builder a later frame hands over is not asked again. So a
control whose words follow a state it changes keys its element id on that
state too — the bar's `bar-pause`/`bar-play`, `bar-mute`/`bar-unmute`, the
chat and fullscreen glyphs, the title bar's Browse (`title-browse*`), the
rail's pin and the gear signed in or not — and the first frame drawn under
the other id drops the open tooltip with the rest of the old element's
state. `controls::tip` is the one builder for a tooltip of plain words:
`.tooltip()` takes a builder, not text, and only once per element
(div.rs:536-549).

**A tooltip outlives a pointer that leaves the window**, since it checks it is
still wanted against the last position gpui saw (div.rs:2293-2301), which a
pointer leaving never updates; fix 7 (below) does not reach that check.
Taking the builder away clears it in prepaint (div.rs:1660-1668), so the
player's bar gives its tooltips only while `window.is_window_hovered()`
(`bar::bar_icon`), and a pane's header does the same
(`watch::header::pane_header`, handed the flag by `watch::page`): the
right-hand pane's × sits `ROW_PAD_X` from the window's edge, and over the
picture an invisible band is still prepainted, so its tooltip would outlast
the band itself. The title bar sits on the window's edge as well and does
not gate its tooltips yet.

**The clipboard is the app's, not a window's.** Writing it is
`cx.write_to_clipboard(gpui::ClipboardItem::new_string(..))` (app.rs:1041),
which needs no focus and no window, and opening a link in the browser is
`cx.open_url` (app.rs:1078). Perch writes the clipboard in one place,
`RootView::copy`, followed by the toast that says it did: `Copy link` from
More or the palette, and a chat row's copy menu, whose chat asks the root
(`ChatViewEvent::Copy`) rather than writing it itself. Each platform has its
own `write_to_clipboard` under gpui's; the macOS one is compiled by CI's Mac
leg and has never been run for Perch.

**The right button arrives as itself.** Windows' `WM_RBUTTONDOWN` and
`WM_RBUTTONUP` are dispatched as `MouseDown`/`MouseUp` with
`MouseButton::Right` (gpui's windows/events.rs:82-88), so
`on_mouse_down(MouseButton::Right, ..)` hears a right press, bubbling from
the element under the pointer outward like a left one, and the root's
`track_focus` hears it too (it takes any button, div.rs:2025-2037). A click,
though, is only ever made from the left button (div.rs:2139), so a link's
`on_click` never fires on a right press. The chat's copy menu leans on the
order: a link or an emote notes that it was pressed, and its row, which
hears the same press next, opens the menu about it (`chat::copy_menu`).

**`deferred` leaves every clip behind.** A deferred element is prepainted
and painted after the whole tree, with the element offset it was given but
none of its ancestors' content masks (`Window::defer_draw`,
`prepaint_deferred_draws`), and its hitboxes go in last, so it is on top for
the pointer as well as the eye. That is what lets the rail's preview card
stand beside a row of a scrolling, clipped list (`sidebar::row`), and a
chat's copy menu and the channel menu reach past what they open from
(`chat::copy_menu`, `channel_menu`): each is `deferred(anchored()..)`, with
`anchored` keeping it inside the window. It is also why none may take part
in anything the tree is clipped for: the root's content wrapper clips the
toasts off the title bar (below), and a deferred element is not held by
that. The preview card has no listeners, so it has no hitbox at all, and
the menus open at a press below the bar, and occlude. A deferred element
keeps the text style it was deferred under, so a menu deferred from inside
a chat would take the chat's text size and leading; both menus set their
own. And it is drawn over the modals too, so opening the palette or the
settings sheet by its key (`Ctrl+K`, `Ctrl+,`) closes both menus first
(`RootView::close_copy_menus`), or one left open floats over the scrim and
takes the presses meant for the modal; a press on the gear needs nothing,
since a menu's own press-outside closes it.

**A notify from a probe asks for no frame.** A `canvas` probe's first
closure runs in prepaint (`DrawPhase::Prepaint`, window.rs:2014), and there
`cx.notify()` only adds the view to `dirty_views`: `WindowInvalidator::
invalidate_view` sets `dirty`, which is what asks the platform for a frame,
only in `DrawPhase::None` (window.rs:116-126). The frame being drawn has
already been rendered from the state before the probe spoke, so a probe
that changes what `render` would draw, and only notifies, leaves the old
drawing up until something else happens to draw: under a video that is the
next decoded frame, but on Following, on a paused pane, or anywhere nothing
moves, it is never. The rail's preview card stayed up over the page after
the pointer left its row this way. A probe that changes what is drawn asks
with `window.request_animation_frame()` instead, which queues a notify of
the current view for the next frame callback (window.rs:1654), outside
drawing, where it does schedule one (`sidebar::preview_probe`, a viewer
card's veil probe). A probe whose view is redrawn every frame anyway, a playing
player's, gets away with a notify, which is why the trap stayed hidden.

**`gpui_component::init(cx)` must run before any widget**, and `Root::new` must
wrap the window's first view or overlays have nowhere to render.

**The title bar is Perch's own, and the platform's behaviour has to be earned
back piece by piece.** `TitlebarOptions::appears_transparent` hides the
platform's bar; `root/title_bar.rs` draws the replacement, and its module doc
and `controls::caption_button`'s carry the full reasons. Every one of these
fails silently, and each was read out of gpui 0.2.2's source:

- **The bar must `.occlude()`.** The root's `track_focus` calls
  `prevent_default` on every press it hears (div.rs:2025-2037), and the
  Windows backend reports a non-client press whose default was prevented as
  handled (events.rs:976), so without the occluder a drag, a double-click to
  maximise and the top resize edge all do nothing.
- **`WindowControlArea::Drag` goes only on an empty spacer** that is a sibling
  of every control, never an ancestor: the hit test answers with the first
  window-control hitbox in paint order, parent before child
  (window.rs:1138-1141).
- **The drag and caption areas start `layout::drag_top` down.** gpui asks for
  a window control before it asks Windows about the frame, so an area reaching
  y=0 takes the top resize edge with it. Maximised it is 0: gpui skips its
  `HTTOP` band there (events.rs:918).
- **Caption buttons carry no handlers.** No `on_click`, no `stop_propagation`,
  no focus: anything that handles the press stops the platform acting on it,
  by the same events.rs:976. Nor `zoom_window`, which maximises but never
  restores, or `remove_window`, which skips `on_window_should_close` and so
  never saves the window's place.
- **A frame drawn after the pointer left the window hit-tests nothing**, on
  Windows and Linux: vendored gpui fix 7. gpui hears the leave only as the
  window's `hovered` flag, never a move (events.rs:318-327); on macOS
  `is_window_hovered` means active. The gates stay: a tooltip's prepaint
  still tests the last position, and a press on a caption button dragged off
  the window never comes back as a release (events.rs:986-997), so
  `caption_button` lights only while `window.is_window_hovered()` and keys
  its id on the flag. The flag is one bit for the window, so a pointer that
  crosses onto a caption button in one move and stops may leave it dark until
  it moves again. Read from source; not yet watched with the real cursor.
- **Nothing that takes a press of its own sits in a drag area**, the search
  box least of all: a press there is the platform's, and the moves after it
  arrive with no button held (events.rs:938-942), so drag-selecting the
  box's text would break. A sibling of the spacer can still run on under it
  in a narrow window, so the box's group is `overflow_hidden`, which clips
  hit testing as well as painting (window.rs:779). While a modal is up the
  group sits under an `occlude`d veil and the keys are kept off the box
  (`title_bar_leading`), so nothing in the bar changes the page behind a
  modal; the gear, the drag strip and the caption buttons stay live.
- **Modals and toasts live in the root's id-less content wrapper**, under the
  bar and clipped, because `block_mouse_except_scroll` does not take a drag
  area or a caption button out of the hit test (window.rs:775-796): a toast
  over the spacer would drag the window. The clip bounds hit testing as well
  as painting; tooltips and a text box's menu are drawn after the tree and
  still reach past it. The comment on the wrapper in `RootView::render` has
  the rest.
- **A drag that must survive the pointer leaving its element listens on the
  window.** An element's `on_mouse_move`/`on_mouse_up` hear only the pointer
  over its own unblocked hitbox (div.rs:173-187, 263-273), so the divider
  drag (`RootView::drag_listeners`) and the mouse's side buttons
  (`RootView::side_buttons`) are heard by a hitbox-less `canvas` that
  registers `window.on_mouse_event`.
- **An `svg` paints only with its own `text_color`** (svg.rs:110) and inherits
  none, so a glyph never given one draws nothing. `controls::icon_button`,
  `caption_button` and `fold` colour the glyph themselves and lift it with
  `group_hover`.
- **macOS drags only by AppKit's native strip.** `window_control_area` is a
  no-op there in 0.2.2 (see "Known limits"), so the spacer asks for
  `titlebar_double_click` by hand — through `on_click`, which needs an id, so
  the spacer's `.id()` is unconditional: adding one inside `.when(cfg!(..))`
  changes the element's type halfway through the chain and does not compile.

**A pop-out is a second window, and a player may be drawn by only one window
at a time.** A pane popped out (`root/pop_out.rs`) is the same `VideoView` in
another gpui window, and gpui gives every window its own sprite atlas, its own
focus and dispatch tree, and its own frame-to-frame element state. Each rule
that follows is kept with the code that keeps it: one window per player,
which the main window must not so much as read while it is popped, or it
redraws at the video's rate (`crate::stage`, `RootView::video_in_main`);
`VideoView::set_place` as the only move; a player that belongs to no window
(`VideoView::from_stream`); opening, closing and reaching the root only
through `cx.defer` and `pop_out::to_root` (`root/pop_out.rs`); the picture
as the handle it is dragged by (`controls::drag_layer`); and on top through
the platform (`os_window`), since neither `WindowKind::PopUp` nor `Floating`
is topmost on Windows. "Things not to redo" and "Known limits" hold the
rest. Read from source and not yet seen: crossing between the picture, which
is caption, and the bar, which is client, clears the window's hovered flag
until the next move (events.rs:58, 318-327), so the bar could blink for a
frame; if it does, hold it up for `MOTION_HOVER` in the pop-out alone.

Dragging a pane's header onto another pane, which swaps the two, is gpui's
own drag and drop, and the docs on `watch::header::pane_header`,
`watch::PaneDrag`, `watch::drop_layer` and `RootView::pane_move` say how it
is made to work: the drag is the app's, so it is drawn as nothing and the
root keeps which pane is carried; a drop needs an occluding layer of its
own, mounted only for the drag; and a click that ends a drag is ignored.
**Windows has no grab cursor.** gpui maps `OpenHand` and `ClosedHand` to the
arrow there (windows/util.rs:113-135), so the target's outline is all there
is, and it is enough.

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
that slider too. And because `on_hover` only fires on a `MouseMoveEvent`,
leaving the window is invisible to it: the last move it saw was inside, so the
controls stayed up.

Hover is therefore **measured, not reported**: a `canvas` probe already runs each
frame for render sizing, and it now also asks
`window.is_window_hovered() && bounds.contains(&window.mouse_position())`. The
`on_hover` listeners that remain exist only to wake a repaint — a paused stream
sends no frames, so without them nothing would ask the probe to run again. Their
*value* is ignored on purpose. Do not wire it back up. An element that
`.occlude()`s over a probed area hides the pointer from those listeners, so
it needs a wake-up of its own while it blocks, as a pane's header band has.

**A probe sees through an overlay too, so an overlay over the panes has to
say where it is.** Seeing through the bar is the point of a probe, and it
sees through everything else for the same reason: panes under an overlay
went on counting the pointer as theirs while it worked in the overlay — a
bar up behind it, a chat under it holding its rows, and the pane under the
pointer made the active one. So an overlay measures itself into a veil
every frame and lifts it on every frame it is not drawn, and every pane
probe asks `veil::pointer_over` rather than its bounds alone; the `veil.rs`
module doc has how, and which probes leave their own pane's card out. A
person's card (`chat::viewer_card`) is the one overlay now. An overlay added
later over the panes writes the veil the same way, or brings the bug back,
and wakes the probes with a hover listener of its own, as an occluder over a
probed area must.

**A view flowed into a flex item can be laid out at nothing, and stay that
way.** A pane's chat, a `size_full` block flowed into the `flex_1` box under
its header, was laid out at 0 by 6 px every frame while the box measured
1073 by 407: two panes side by side went blank as soon as both played 16:9
renditions, and stayed blank. Placed instead — the box `relative`, chat in
an `absolute().inset_0()` layer, which takes no part in measuring its parent
— it never did (the comment in `watch::pane`): blank 3 runs out of 3 before,
0 out of 3 after, alternating builds on one launch-then-widen sequence. The
cause inside taffy was not traced to a line. The likely candidate is that
taffy 0.9.0, which gpui 0.2.2 pins, keys its layout cache without the
parent's size; taffy 0.12.0's changelog adds it and calls it "necessary for
correctness". Any other view that fills a flex item by flowing a
percentage-sized block into it is exposed in the same way: if one shows up
empty with a healthy parent, place it the same way. The video box sidesteps
a related problem by making the player a flex item (see the comment on
`video_pane` in `watch::pane`).

**A line cut to fit was measured again over cut runs, and panicked on a
flag.** gpui 0.2.2's text element truncated the very runs its measure closure
captured (`TextLayout::layout`), and taffy measures a line more than once a
layout, at different widths: a later measure that kept the whole line laid
the whole text over runs cut inside it, and with a flag in a stream title
(🇨🇳, four bytes a letter) the cut fell inside a character, DirectWrite's run
slicing panicked (direct_write.rs:595) and perch closed. Vendored gpui fix 8
(`elements/text.rs`) truncates a fresh copy for each measure; upstream
stopped mutating the runs after 0.2.2.

**A row of widths cut to fill it adds up in `f32`, and a hair over wraps.**
A browse row's cards are as wide as the row divided by its columns, and
taffy breaks a wrapping row by adding its items and gaps in `f32`: seven
cards of 2412 / 7 px and six 18 px gaps came to a hair over the 2520 px they
were cut from, so every row of Following ended in an empty column. A card's
width is floored to a whole pixel (`browse::card_width`), and a test holds
every row to never overfilling.

**gpui is vendored and gpui-component is not.** `gpui 0.2.2` comes in from
`vendor/gpui` through the root `Cargo.toml`'s `[patch.crates-io]`, with the
eight fixes its comment lists; `gpui-component 0.5.1` is the plain crates.io
release, pinned by `Cargo.lock`. The gpui line numbers in these traps are the
vendored source's.

### Windows packaging

**The release build has no console**, so `eprintln!` and panic messages go to a
handle that leads nowhere. `diagnostics::capture_stderr` points the process's
stderr at `%LOCALAPPDATA%\perch\perch.log` at startup, keeping the previous
run as `.log.old`, and `diagnostics::keep_panics` appends every panic, with a
backtrace, to `perch.crash.log` beside it, since a worker thread's panic
leaves the app looking alive; the module doc has the rest. It works by
`SetStdHandle` rather than by a logging facade because Windows resolves that
handle on *every write* — so it catches the library crates and panic output
too, with no change anywhere else. That was verified with a throwaway before
the code was written; if you ever doubt it, verify it again rather than
assuming.

Debug builds keep their console on purpose (`#![cfg_attr(not(debug_assertions),
windows_subsystem = "windows")]`); a release build writes `--help` to
`perch.log`.

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

**Desktop notifications** (`notifications/`). The follows poll's go-live
news goes to a Windows toast as well as the in-app one when the main window
is not active (`Window::is_window_active`; a focused pop-out counts as away,
since the in-app toast is drawn in the main window), to whoever
`Settings::desktop_notifications` covers. `notifications::who` is the rule
and its tests are the spec: never at the worker's first poll
(`RootView::streams_seeded`, which `stop_twitch` resets — not an empty
`known_live`, which is also every poll after a night with nobody live and
would swallow the first go-live of the morning), never with the window in
front, and once a broadcast (`Seen`, which outlives `restart_twitch`). The
in-app toast goes by the same rule (`notifications::newly_live`). How a
toast is shown and clicked — an AppUserModelID registered under `HKCU`, and
a `perch://watch/<login>` link, because an unpackaged app has no COM
activator to hear the click — is the module docs of `notifications/` and
`notifications/windows.rs`, and what is written to the registry, and how to
take it out, is the README's. The legacy `Shell_NotifyIcon` balloon was the
other option: it needs a tray icon kept for the life of the process, and
Windows 10 and 11 show balloons as toasts anyway, so it was all of the cost
for none of the gain. None of this has been seen working on a desktop yet
(see "Known limits").

### Motion

**Not every cached image is immutable, and GPUI decodes one per path.** A
channel's preview lives at a *fixed* URL whose picture Twitch replaces every
few minutes, so previews go through `ImageCache::get_or_request_fresh`, which
refetches past a `max_age`, ignores files from earlier runs, and lands each
refresh at a new filename, since GPUI caches a decoded image against its
path; emotes and box art, which never change at their address, keep
`get_or_request` and stay on disk. The `emotes::cache` module doc has the
rest.

**Deleting the file is only half of a refresh.** A new path is a new
`RenderImage`, `App::loading_assets` entry and atlas tile, and nothing frees
any of the three on its own, so `RootView::render` releases what was decoded
from every superseded path before any card is built
(`browse::release_retired_previews`, whose doc has why the order is the
whole trick: backwards, the card goes permanently blank).

**Chat emotes are cached per pane, not per process.** A bare `img(path)` falls
through to `App::loading_assets`, which never evicts, so `ChatView` owns a
`RetainAllImageCache` and gives it back as it is released; that bounds
emotes for open panes, not one channel left open for hours (the field's doc
says why). The pictures emote effects make (`chat_effects::Pictures`) are
given back in the same hook and capped besides; see "Effects are drawn as
BetterTTV draws them" under Chat.

**A `list`'s scrollbar extent comes from *measured* items only.** gpui measures
rows as it draws them, so the thumb changes size as you scroll.
`ListState::measure_all()` helps only once: rows spliced in later stay
unmeasured, and a change in the list's width throws every measurement away.
The thumb's *position* is right throughout. The comment on `measure_all` in
`ChatView::new` has the full story, and why re-arming the pass is not done;
a complete fix means not using gpui's scrollbar geometry.

**Do not reach for `reset()` to jump to the bottom.** It clears the scroll pin,
but it also splices every row as unmeasured, so the next prepaint lays out all
thousand of them in one frame, and it drops the wheel until the next paint.
`follow_live` uses `set_offset_from_scrollbar(point(px(0.), px(f32::MAX)))`
instead, and its doc says why `scroll_to` and `scroll_by` are no
alternative.

**GPUI has no transitions.** `.hover()` swaps styles instantly and there is no
way to interpolate between them. Every animation in the app therefore goes
through `with_animation`, and `motion.rs` exists because that primitive only
does one thing: run forward from zero.

**Animation state is keyed on the element id**, and an id GPUI has already seen
comes back as a *finished* animation holding its last value. That is why a
two-way fade has to mint a new id on every flip (`Fade`), and why a one-shot
arrival needs no state at all — mounting the element is the whole trigger.
GPUI keeps that state only from one frame to the next, so a `Fade` not drawn
for a while replays its last flip when it comes back: keep a faded wrapper
mounted on every frame, or start it over with `Fade::hidden()` when its
element goes away (`Fade::apply`). `VideoView::set_place` does it for the
bar, and `RootView::sync_presentation` for each pane whose cell is off the
screen: its band, its chat's hold and its hover.

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
now just repaint triggers, GPUI's idea of hovered and ours must not drift, so
pane element ids are keyed on the pane's **key** — a channel's login, or a
recording's — never the index: a position-keyed survivor of a closed pane
inherits its `hover_state = true`, and its header stays hidden until the
pointer leaves the pane entirely. See `watch::pane_id`.

### What deliberately does not move

Both of these were considered and rejected, so they read as decisions rather
than as things nobody got to:

- **Browse cards.** Hover is instant there. Fading each of a hundred cards would
  need per-card state in `RootView`, and sweeping a pointer across a grid feels
  worse with fades than without — the highlight lags behind the cursor.
- **Chat rows.** Animating arrivals would pin the frame loop at 60 fps for what
  is a text list, and in a fast channel a per-message fade is a strobe.

### Design system

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
  a channel that opens silent reads as broken. That is also why the stored
  value is an `Option<u8>` — `Some(0)` (deliberately muted) and `None` (never
  opened) must not collapse into each other — and why a hush, the silence
  `Only this one` lays on the other panes, never writes a level at all
  (`loudness.rs`, tested there). `Alt` and the wheel are the volume keys'
  path, `nudge_volume`, so they do save the level, against the channel. Keys
  go through `settings::channel_key`, because the app does not agree with
  itself about case: Helix says `forsen`, the command line says whatever was
  typed.
- **Spacing** — named by role (`PAGE_PAD`, `PANEL_PAD`, `CONTROL_PAD_*`,
  `GAP_TIGHT`, `GAP`, `GAP_SECTION`, `PANE_GAP`, `ROW_PAD_*`), not by size.
- **Controls come from `controls.rs`.** There were ten hand-rolled buttons in
  six shapes — the tokens were shared the whole time and the component was not,
  which is the same drift one file down. A control that needs a shape not on
  that list is a new variant there, not one more `div`, declared in
  `variants!` so the contrast test measures it the moment it exists. Its
  module doc names everything else the file owns, and each function's doc
  says why it looks as it does: what a control says under the pointer is
  `tip`, never a `gpui_component` `Tooltip` built at the call site; one not
  on offer is `icon_waiting`, never a control faded to look disabled (see
  "Things not to redo"); `caption_button` is drawn here and pressed by the
  platform (see the title-bar trap); red is `live_badge`'s alone — this pane
  is watching live, now; and `Variant::Confirm` asks for a second press in
  the accent, since the app has no warning colour. The player's bar builds
  every icon through `video_view/bar.rs`'s helpers, which hold its tooltip
  gate and its state-keyed ids by construction (see that module doc).
- **Casing** — words on controls are sentence case: `Refresh`, `Open
  settings`, `Save`, `+ Add`, `← Back`, `Try again`, the browse tabs, the
  channel page's shelves. That covers a control's stand-in too — the
  `Loading…` a Load more row says while its page is out, chat's `Chat
  paused` where its jump-to-live pill goes, a switch's `On` and `Off`. A
  thing you read is written as a sentence: titles and headings, notices,
  field labels, tooltips, palette rows. Proper nouns keep their capitals
  either way, and a domain keeps its case (`Search Twitch for “…”`, `Open
  twitch.tv/activate`). The line a pane words itself while it starts or
  after it stops is a sentence too (`Starting…`, `Forsen is offline`,
  `Finished`), since it sits over a sentence-case pill. So is a toast (`Link
  copied`, `Couldn't switch to 480p30`, `Sign in to rewind`), a notice like
  any other. A pane header's `muted`, `paused`, `buffering`, `replay` and
  `ad break` tags stay lowercase on purpose (see "Product decisions"). Not
  swept yet, so do not take them for the rule: the other passive words that
  state a fact — chat's `deleted`, chat's notices, the palette's kind column
  — are still lowercase, and so is a failed pane's line, which is the
  failure in the words of whatever failed (streamlink, mpv, the streamlink
  crate or the player), passed through as is by `Showing::sentence`.
  `channel_page::kind_tag` is also read mid-sentence, in a palette row's
  `(replay)` and a history byline, so it cannot simply take a capital.
- **Layout reads a `layout::Body`, never the viewport.** The body is the window
  less the rail and the title bar when each is drawn — neither is in
  fullscreen (`layout::rail_shown`, `layout::title_bar_height`) — and it is a
  type made in one place, `RootView::body`, so nothing laying out a page can
  be handed the viewport by mistake; `layout::Body`'s doc has the bug that
  made it one. `a_body_is_the_viewport_less_the_rail`,
  `a_body_is_also_less_the_title_bar` and
  `the_bar_is_as_tall_as_the_room_the_page_leaves_it` hold it.
- **The watch grid is `layout::Grid::of(body, cells)`, and nothing else.**
  Its rows, columns, cell size and whether a cell stacks its chat come from
  that one call, made by `RootView::grid` for the watch page, the divider
  drag and each pane's quality (`pane_height_for`), cut from the cells
  `stage::Stage::cells` says are drawn; `layout::Grid`'s doc says why.
  `grid_shape` and the cell helpers are private to `layout.rs`, so a second
  copy of the sum cannot start. Stacking and quality are judged on a cell's
  share of the body before the seams come out, not on the cell as cut, which
  a seam can tip near `PORTRAIT_ASPECT` (`Grid::portrait`,
  `Grid::share_height`). Everything the page sends back about a pane — a
  press, its hover, a divider pulled (`watch::ResizeStart`) — names the pane
  by its key, never by its place in what was drawn.
- **Sizes the user dragged are settings, not view state.** `chat_width` and
  `video_share` live in `settings.json`, and the drag writes them on mouse *up*
  rather than on every move — a drag is hundreds of events and each save is a
  read-modify-write of the whole file. `Ctrl+0`, or `Reset pane sizes` at the
  foot of a pane's chat options menu (`RootView::reset_sizes`, the one path
  for both), puts both back to their defaults, `video_share: 0.0` meaning
  "derive it".
- **The widget library reads the same tokens**, via `widget_theme::apply` after
  `gpui_component::init`, or `init` seeds its palette from the *operating
  system's* light/dark setting and every input, dropdown and slider follows
  the OS while everything around them stays dark (`widget_theme.rs`).
- **Type** — four roles (`TEXT_TITLE/BODY/LABEL/META`). Label and meta share
  a size but differ in weight, so a thing you can click never looks like a
  thing you can only read. `weight_shout()` is for the one element that has
  to shout, `controls::live_badge`: white (`theme::live_text`) on the one red
  (`theme::live()`), since the app's off-white fails on it.
- **Contrast is measured, not judged.** `MIN_CONTRAST` is AA for the sizes this
  app uses, `theme::contrast` computes it, and `every_text_tier_is_legible`
  holds every text token to it on every surface it lands on. `readable_on()`
  lifts a username until it *measures* legible on the ground it is drawn on,
  a washed chat row's own paint included (`ChatView::row_ground`). Text on a
  picture is held to the brightest picture there is, a white frame under its
  wash (`theme::over_white`): `video_chrome()` carries every tier and is what
  the player's bar sits on, and `overlay()` carries `text()` alone, for
  badges and a starting pane's poster. `Variant::OnVideo` is measured on that
  white frame too (`controls.rs`'s `sits_on`), not on black, where light text
  never fails. `theme.rs`'s docs carry every ratio.
- **Three text tiers that step evenly, and controls quieter than what they
  sit beside** (October 2026). Text, `text_muted` and `text_dim` step 15,
  8.7 and 5.3 on the chat surface, and a control's words and glyph at rest
  are `control_rest`, quieter than `text_muted`, so chrome recedes behind
  what is read (`a_resting_control_is_quieter_than_supporting_text`). A
  passive tag sits on a neutral chip, `theme::tag_fill`, so the accent means
  something to press, something chosen or progress, and a state does not
  look like a selected control. Chat's event washes keep the accent for now.
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

The list comes from git rather than from a glob, so a new directory is never
skipped. Keep the `:(glob)` magic: in a plain pathspec `**/` needs a slash
after `src/`, so it drops every file at the top of the crate.

The first four should return nothing outside `theme.rs`; the tests that
measure text over a picture build their backdrop with `theme::over_white`
rather than a grey of their own. The fifth, literal sizes, is the ledger of
the debt that is left: what it prints should only get shorter, and a token
in `theme.rs` is how a line comes off it. A zero (`px(0.)`) is left out on
purpose: the divider seams and the seek bar's time label hang off zero-sized
anchors by design. The sixth will show genuine timings — the follows poll,
the toast lifetime and its resume floor, an mpv frame wait — but no
*animation* duration should appear outside `theme.rs`. The last should
return nothing: a control at zero opacity still takes clicks (see "Things
not to redo"), and only `motion` fades one there, on its way to
`invisible()`.

### Where controls live

**Nothing static sits on a playing picture.** Static information on a moving
picture is exactly what you end up staring past for three hours, so it lives
in the pane header on the chat panel instead — chat is already a panel, so it
costs nothing there. What does come over the picture comes because something
asked for it: the pointer on the pane, one of the bar's menus held open, or a
key that has just chosen the pane. A pane with no picture — starting,
offline, ended, a player before its first frame — is not a picture, and its
header rests over it. The split:

- **Pane header** (`watch/header.rs`, whose module doc has its two shapes):
  the pane's details at the top of the chat panel, or compact on a band over
  the picture with chat hidden or none to show (`watch::Placement::of`). Its
  ⋯ opens the pane's menu under it — the pin, `Past broadcasts`, out and
  back, and the rest (`toggle_pane_menu`) — and with two panes or more the
  header is the handle a pane is dragged by (`RootView::move_pane`). The
  band is `watch::pane`'s, faded by `Slot::header` so it outlives a player
  rebuilt by a cold restart, and up while `band_wanted` says; it occludes
  while up, so it carries its own wake-up (`band-layer`; the comment where
  it is mounted says why).
- **Not under the picture.** A pane's details never go under its picture,
  even where its column has black to give: for a week of October 2026 they
  did (`watch/strip.rs`, gone), and the user did not like the viewing page
  it made (see "A pane's details sit at the top of its chat" under "Product
  decisions"). The picture is centred beside chat; its box, render size and
  quality are the same either way.
- **Over the video**, only while something asks for it
  (`video_view::bar_wanted`): the control bar (`video_view/bar.rs`; More's
  rows are `VideoView::more_rows`, in `video_view/menu.rs`). It keeps to
  what one stream needs; what only two panes or more need is on the pane's
  right-click menu. `bar::RIGHT_BUTTONS` counts the right-hand cluster,
  which never drops a button, so one added there narrows the bar sooner.
  What is the pane's rather than the player's goes up as `VideoEvent::Pane`
  and is resolved by key in `root/pane_actions.rs`, the one place the
  clipboard is written. Nothing page-level is drawn over the panes, so the
  top edge is the pane's: navigation once floated over the first pane's ×,
  and with neither stopping propagation, one click closed the pane and
  navigated away.
- **The channel menu** (`channel_menu.rs`, `root/channel_menus.rs`): the
  right button wherever a channel is named, and the ⋯ in a pane's header;
  a pane's adds what only two panes or more need. What it offers is worked
  out as it is drawn (`channel_menu::entries`), so no row comes from a stale
  answer; under the ⋯ it hangs by its top right (`channel_menus::MenuAt`
  says why).
- **The browse page** (`RootView::go_browse`): a page of its own, and
  leaving the watch page for it stops every pane at home (`retire_homeless`,
  the one rule for it), while a pane in a window of its own plays on (see
  "Browsing stops what plays" under "Product decisions"). Whatever leaves
  asks `leaving_stops_panes` first when it wants to say so or wait. A press
  on a card, a rail row, a go-live toast or the palette's Watch plays in
  place of the pane you are on; `+ Add`, a rail row's `+`, a launch and a
  link open beside (`streams::Opening`).
- **Under the video**, the status screen (`watch/status.rs`), drawn under
  the player until its first frame has faded in (`VideoView::covers`), so
  the picture arrives over the poster rather than out of black.
- **The title bar** (`root/title_bar.rs`, whose module doc has the traps of
  drawing one): the rail button, Browse, the search box, which answers from
  what Perch knows as you type and asks Twitch after a pause
  (`root/search_box.rs`), the sign-in's words while nobody is signed in
  (they stay up on either page while you go and do it), the gear, which
  says who is signed in on its tooltip, and on Windows the caption buttons.
- **The browse page's tab strip** (`RootView::tab_strip`): back and forward,
  the four tabs, then Refresh, left-aligned to keep Refresh out from under
  the toasts.
- **The rail** (`sidebar.rs`; `sidebar::row` has a row's controls): the pin
  and `+` a row reveals under the pointer stop propagation, or the row under
  them would fire as well.
- **A pane's pop-out** (`root/pop_out.rs`, whose module doc has the ways out
  and back and the rules that hold it up), Windows only: the picture alone,
  with a bar ending in Bring back and a red Close (`bar::pop_out_cluster`);
  its chat and header stay in the main window, and its quality follows its
  window, upwards only. The window's own close brings the pane back; Close
  and `Ctrl+W` close it.
- **A pane given the window** (`stage.rs`, `root/panes.rs`): `Z`, its menu's
  `Maximize` or the palette's `Maximize …` give one pane the whole watch
  page, chat and all, for the session. The others are drawn as nothing and
  play on (see "Quality is chosen again when the grid changes" under
  Video): a strip of names would take height from the pane given the page
  and so lower its rendition, thumbnails would be the focus layout that chat
  per pane rules out, and a pane drawn small comes back soft.
  `RootView::choose`, `sync_presentation` and `active_slot` keep the keys
  on a pane that is drawn.

Everything in the pane header but the name comes from a `LiveStream` looked
up by login at render time, never copied onto the `Slot`, so there is one
source and it cannot go stale: `RootView::stream_info` walks every live list
the app holds, then `root::pane_streams`'s answers, and `live_info` and
`channel_name` add the rail's recommendations. Its count is Helix's
`viewer_count`; there is no chatter count to be had (see "Known to be out of
reach").

A pane's video and chat divide it as deliberate opposites: beside, chat has
a fixed width; stacked, the video has a box the shape of its stream and chat
the rest, since a window is tall because you want more chat, not more
letterboxing (`layout::stacked_video_height`). Stacked with chat hidden, a
pane keeps that box rather than giving the picture the whole cell, so it
stays in line with its neighbours (`watch::chat_or_why`).

### Browsing

Follows are **two lists that never merge**. `LiveStream` means *is live*, and
three things read it that way — the went-live toasts, the card's viewer count,
and the pane header's `LIVE` badge — so an offline channel sitting in that vec
would be wrong in all three at once. Offline follows are `Channel`s, and they
are drawn as names rather than cards: a card is mostly a picture, and an
offline channel has none worth showing — a thumbnail stale by hours, or a
profile picture that costs another request per refresh and says nothing.
Names also pack, so a hundred follows is five rows instead of a wall of grey
rectangles. Clicking one opens the channel's page — its past broadcasts, with
its chat one click away from that page's bar, since chat connects whether or
not anyone is streaming.

**A channel's page is three lists**, past broadcasts, highlights and uploads,
behind a switch of pills: `ChannelPage::shelves`, one `Listing` each, so a
hundred daily broadcasts do not bury the highlights. Neither highlights nor
uploads have a chat replay, so they open as picture alone, and the pane
header's tag names the kind (`channel_page::kind_tag`).

Both followed endpoints paginate (the trap under The two Twitch tokens), and
`/channels/followed` is the one whose `first` defaults to 20 rather than 100:
forget the parameter and a long follows list quietly shows a fifth of itself.

**Refresh means "this list", not "follows".** One control, beside the tabs,
whichever list is up (`RootView::refresh`). `Request::Follows` is
intercepted in `run`, since `serve` cannot see the poll timer and a poll
done by hand there would be repeated seconds later. A failed poll is
`FollowsError`, never `Error`, which the UI reads as signed out; it is a
toast only when somebody pressed the button.

Three lists on one page — Following, Popular, Categories — because they are
the same question asked three ways, so they share one grid and one card
(Following groups the follows as the rail does, and adds a row of the
history's recordings after who is live). Only categories look different, and
only because box art is 3:4 rather than 16:9. Opening a category *replaces*
the page rather than nesting inside the tab, so there is only ever one thing
to scroll. Which takeover is up is `Discovery::place`, the one copy of the
order they stack in — a channel's page over a search over a category over
the tab — read by the page, refresh, Load more, `Esc` and the trail alike.

**Back and forward walk a trail; `Esc` steps out.** `Alt+←`/`Alt+→`, the
browse page's arrows and the mouse's side buttons walk the tabs, categories,
searches, channel pages and the watch page you have been on. The trail
(`trail.rs`, driven from `root/navigation.rs`, whose module docs have the
rules) is history and nothing more: a `Route` is *read* from `page` and
`Discovery`, never kept beside them, and `RootView::record` wraps the
functions that move the view, so every way into them is a step without
knowing it. Going back replays a route through the same functions with
recording held off, so back to the list that opened a pane asks Twitch
nothing. The watch page drops off the trail once nothing plays
(`retire_slots`), so back never stops on an empty page. A fifth pane is
refused where you are, with the toast.

**The fourth tab, Library, is the app's own memory and asks Twitch nothing.**
It is `settings::history`, in `history.json` beside the settings (its module
doc says why), written by `root::history`: an entry as a recording opens,
its place every fifteen seconds while it moves, and when its pane goes. That
last is why `RootView::retire_slots` is the only way panes are closed: the
position lives on the slot, and a `slots.clear()` anywhere else takes the
last few seconds with it. The window closing waits for the last write
(`root::saving`). Opening a recording from anywhere asks `resume_point`
first; within a minute of the end (`history::finished_at`) counts as
watched.

**Following is the landing page, grouped as the rail is, and the history's
front row.** It is the browse page's first tab and the one the app opens on
(`Tab::Following`), grouped by the rail's own rule (`sidebar::groups`) so
the two never disagree; `following.rs`'s module doc has its sections and
their order. Its offline names' last-watched order is a list of its own,
`RootView::following_offline`, sorted only where the rail's is — a poll
while nothing is held, and the hold's release — never at draw time: sorted
every frame, it moved a channel whose stream had just ended from the end of
the names to near the top, under the pointer (`following::by_last_watched`).

**Each offline name says when it was last live, and Offline how many.**
"Live 3 hours ago", beside the name on Following and under it in the rail,
is `last_live.rs`'s: its module doc has when it asks Twitch's unpublished
GraphQL endpoint (`Request::LastLive`), how it goes quiet on a failure, and
why the words count from the later of Twitch's `startedAt` and the last poll
that saw the channel live.

**Following is shelves, and Offline starts folded.** The user found the page
cluttered (22 October 2026): nine cards of 260 px to a row at 2560 px, three
lines under each, a count on every heading, and a hundred offline names under
them. So each section is a shelf under its heading, Offline is folded under
its count for the session as in the rail (`following::offline_heading`), and
every grid's card is at least `browse::CARD_MIN`, 320 px, with two lines
under its picture (`browse::name_line`); the offline names kept their pills
and their "Live 3 hours ago", asked about and left as they were. A stopped
pane's last-broadcast card keeps the old least of 260 (`browse::CARD_LEAST`),
the width a title stays worth reading at.

**Forgetting can be taken back.** `History::forget` and `clear` return what
they took (`history::Forgotten`), and the toast's `Undo` hands it to
`History::restore`, which puts each entry back where it stood. An entry is
only ever added by opening (`History::progressed`), so a recording still
playing after a clear stays off.

Everything the user does there arrives as one `browse::Action` rather than one
callback per control: the page is generic over its owner, so each extra closure
would be another type parameter threaded through every helper.

Three things worth keeping:

- A category's streams are dropped if the reply arrives after the user has left
  it, and so are a search's and a channel's videos. Without that check a slow
  response repopulates the page behind them.
- **Loading and errors belong to the list that asked.** Every browse request
  names the list it fills (`Request::list_key`), and `Discovery` waits on
  lists by key, an entry per request out (`Discovery::pending`); the list on
  screen is read off `Place::first_page`, so the page and the request agree
  by construction. With one flag for everything, a reply for a list you had
  left took "Loading…" off the one you went to.
- `RootView::fetch` refuses to wait when there is nobody to answer — before
  sign-in, or after the worker has stopped — and says why on the list that
  asked; `fill_shown` asks again once sign-in lands, and the worker's
  terminal `Error` empties `pending`.

Lists are fetched once per tab and kept, and grow a page at a time on Load
more — see "Paging, and what is not paged" for the cursor's route out to the
caller.

**The live follows hold still while pointed at.** While a probe on the rail
or on Following (`RootView::holding`) has the pointer, a poll is merged into
the order on screen (`follows::keep_order`) rather than sorted again, so a
row or a card never changes between aiming and clicking; the offline names,
the pins and the recommendations hold the same way, and all go back in
order on release (`hold_live`). A list not on screen is released by the page
that is not drawing it, since an unpainted probe says nothing.

**The pins go as the groups under them do, and hold the same way.** Live
first by viewers, then off by when last live, then the rest in the order
they were pinned (`sidebar::pin_order`): the user asked for them sorted.
Unlike Following's names they are sorted at draw time
(`RootView::order_pins`, whose doc says why), which is safe because while
held the fresh order is merged into the order last drawn with `keep_order`,
so nothing moves under the pointer.

**A live rail row the pointer rests on shows its stream's card.** After
`rail_preview::PREVIEW_DELAY` (400 ms), beside the rail and never over the
row: `rail_preview.rs`'s module doc has what it shows and when, and
`sidebar::preview_probe` and `RootView::follows_rail` how a row measures the
pointer and when no card shows.

**A went-live toast is the way to the channel**, not only news of it:
`Toast::action`, watch from the text and `+ Add` from the pill beside it.
And a pane that stalled starts again by itself, while its `Start when they
go live` switch is on (`RootView::start_offered` says where it is offered),
when a list of who is live has its channel — the follows poll for a channel
you follow, `root::pane_streams` every two minutes for any other — through
`RootView::start_due`. The timestamp is the whole trick: only a broadcast
that started after the pane stopped is one it has not tried, since the list
is up to a minute behind and would otherwise turn "ended" into "offline" for
nothing. `Slot::due_to_start`'s doc has the two exceptions, each one try per
broadcast (`Slot::retried_for`).

**A toast holds still under the pointer**, every toast while the pointer is
on any of them, since the stack hangs from the top (`RootView::hold_toasts`,
`chrome::Countdown`). One probe after the cards reports for them all, which
relies on gpui prepainting a div's children in order (div.rs:1412-1416).

**A stopped live pane asks what the channel broadcast**:
`Request::Broadcasts` (`root/broadcasts.rs`), its own request rather than
`Request::Videos`, so a pane's answer and failure never land on a channel
page. Every stop asks, answer in hand or not, since a pane can sit offline
through a whole broadcast. An ended pane finds the broadcast that ended with
`channel_page::archive_of`.

**Search** is four requests behind one result (`twitch::search`):

- `/search/channels` answers with a **profile picture and no viewer count** —
  a different shape from every other list in the app. So only its logins are
  kept, and they go back through `/streams` to become ordinary stream
  records.
- The same endpoint without `live_only`, for the channels that are not on,
  kept as names and re-ranked so the channel typed comes first
  (`name_rank`). A separate, best-effort call, so offline matches cannot
  crowd the live ones out of a page, and a failure costs the names alone.
- Categories, because a name like "zomboid" is as likely to mean the game as
  a channel.

Results show **live channels first**, then offline ones, and categories are
capped at `SEARCH_CATEGORY_LIMIT`: with categories first, the channel you
searched for was below the fold. A live channel is directly watchable; a
category is another click.

The title bar's box suggests what the app knows as you type, and asks
Twitch on `Enter` or once the typing has paused on three letters or more —
never by itself on the watch page while something plays, since the results'
page stops it (`root/search_box.rs`). The **palette** asks only the app, so
it costs nothing and runs on every keystroke.

**The palette's arrows and Escape are key events, not bindings.** Its own
text field is focused while it is open, that field's context is deeper than
the root's, and the keymap deliberately stands aside for a focused input —
which is the behaviour that keeps typing working everywhere else. A binding
cannot win that argument, so `on_key_down` reads the event on the way past
instead.

The three chords on the command key — `Ctrl+K`, `Ctrl+,`, `Ctrl+R` — are the
exception, and do not stand aside for a text box (though `Ctrl+K` waits for
the settings sheet to close; see "Keyboard"). They type nothing, and no
gpui-component widget binds them, so the guard only ever cost something: a
search leaves the cursor in the title bar's box, and the palette was dead
until the page was clicked. They are still scoped to the app rather than
`None`, so a widget that ever claimed one would win it back. `keys` has a
test asking gpui's own keymap.

**Avatars are a second request.** `/streams` carries a stream's preview, not
the channel's picture, so the rail's faces come from `/users`, sent after
the live list and merged in so the rail fills in rather than blinking.
Offline follows are looked up once a session (`twitch::unpictured`).

**The rail's Recommended group reads Twitch's unofficial sidebar query.**
Helix has nothing like it. `twitch_api::recommend` asks the website's
persisted `SideNav` query, anonymously and on the website's Client-ID; its
module docs carry the hash, the forum's word on the endpoint and the
fixtures. The perch half is three files: `recommended.rs` works it out
purely, `root/recommended.rs` runs it, and `sidebar.rs` draws it, under Live
and over the offline fold. Their docs, and `Request::Recommend`'s, have the
seeds, when to ask, why only signed in and on screen, how failures stay
quiet, and why a row just opened in a pane holds its place under the pointer
(`root::recommended::hold`). The worker's terminal `Error` and a new client
id forget the ask that was out (`forget_asks`), or no ask would ever go
again. A pane opened from the group reads what it says of its channel from
`Recommended::channel`; the palette does not offer recommendations yet.

**Pins from anywhere.** A pin is made from any channel's right-click menu
(`Pick::Pin`), a pane's More (`PaneAction::Pin`), a channel page's bar
(`channel_page::Standing`), and a followed or pinned rail row's own pin; all
end in `RootView::set_pinned`. A recommended row has no pin of its own
(`sidebar::Row::offers_pin`): pinned from the row, a live channel would land
in Pinned as a bare login until the ask about it came back. Its right-click
menu pins it all the same, and the pin shows as that bare login for the
second the ask takes. A pin nobody follows rides `root::pane_streams`, whose
module doc has what that ask covers and when; its answers are kept there and
never in `follows`, which three things read as who is live.

**Card width is derived from the window**, by `browse::card_width`: the row
is filled rather than merely fitted (`browse::CARD_MIN` says why).

**What is only true right now goes over the picture**: the viewer count and
uptime sit on the thumbnail, where broadcast UIs have put them for decades.
A `LIVE` badge there said the same thing on every card, since every list of
stream cards is live-only, in the app's only saturated red, while the number
that varies sat in grey underneath. The badge is on the watch page, where it
differs from pane to pane. Don't put it back on the cards.

**An offline channel says when it is on next, from its schedule.** Helix's
Get Channel Stream Schedule, asked once a session per channel, for an offline
channel's page and a pane that found it off, never for Following's or the
rail's offline names (a request per follow); `schedule.rs` has the rules and
wording, `status::offer_room` how a pane's line yields to the last
broadcast's pill.

### Chat

The pane reads a live log for hours, so nearly every decision in it is about
scanning rather than features.

**The time is written the way the machine writes it.** `clock.rs` asks the OS
for its short time pattern once and renders every stamp through one small
formatter, with tests: a US machine reads `11:41 PM`, a German one `23:41`,
and a short time format the user edited by hand in Region settings is
honoured. Falls back to `HH:mm`, which is what chat always showed.

**The time is said once a minute, not once a row.** `ChatView::time_break`
draws the time and a rule above the first row of each minute, rather than a
stamp per row in a gutter (`ChatView::row_frame` says why), and the rows get
their width back. `Time on every message` in the chat options puts a stamp at
the start of every row instead and takes the breaks away.

**An event row's body is wrapped in a `flex_row`**, and that is load-bearing. A
`message_line` is a `flex_wrap` row; dropped straight into the event's
`flex_col`, gpui sizes it from its own content, its widest piece, so its
words wrap one per line and paint over the rows beneath. Anything else that
renders a `message_line` needs the same wrapper; `render_event` has the
story.

**Usernames go through `theme::readable_on`.** Twitch lets people pick any
colour and its own fallback palette ships pure blue, firebrick and seagreen —
all darker than the surface they land on. Lightness is lifted and hue kept,
to a measured contrast against what the name is drawn on: the row's paint in
the list (`ChatView::row_ground`), the raised surface for a viewer card's
name, the bare surface in a card's well. The tests run against
`twitch_chat::message::DEFAULT_COLORS` itself so the two cannot drift; see
"Contrast is measured, not judged" under "Design system".

**Anything per-row belongs to the row, never to its index.** The backlog drains
from the front once full, which shifts every surviving index. That inverted the
whole pane's stripe on every message past the cap, and would have cost links
their hover state too — rows carry a stable `seq` for exactly that.

**Words are classified in `chat_text`,** and the hard part is not matching URLs
but *not* matching them: chat is full of `lol.`, `1.5` and `wtf.jpg`. The
module doc has the short list of TLDs a bare host may end in, and why it
leaves out `.so`, `.is`, `.at` and `.it`.

**No word shrinks; a long one is drawn in pieces.** A word longer than the
piece budget (`chat_text::piece_chars`, 16 characters at body size) — a long
URL, in practice — is cut by `chat_text::pieces` into runs drawn edge to
edge, so `flex_wrap` breaks the line between pieces; `ChatView::render_word`
has the rules. A word left to shrink below its width (`min_w_0`) and wrap
inside itself is measured wrong: asked for its size with no width to wrap
at, gpui's text element hands back whatever its last measurement produced,
wrapped or not (`vendor/gpui/src/elements/text.rs:376`,
`wrap_width.is_none() || ...`), so a word that taffy measures both whole and
squeezed in one pass gets the wrong size for one of the two, and its row
drew over the next message or carried a blank line (reproduced on recording
2888969029 at 25,344 s, a Nightbot message with two links). Keep chat's words
`flex_none`; anything else that wraps text inside a shrinking flex item can
meet the same thing.

**Mentions take the colour of whoever is being addressed,** from a login→colour
map that fills itself as people talk. A miss renders plainly rather than
guessing. This only works *because* of the readability clamp — without it you
would be scattering unreadable blues through body text, which is worse than
leaving mentions alone.

**Emotes overhang their line rather than growing it,** so a row with emotes is
no taller than one without. `ROW_PAD_Y` is what makes that work: the 4.5px of
overhang at each end of a row has padding to sit in. At a larger text size
(below) the emote, its line and so its overhang all scale, and the padding
scales with them (`Metrics::row_pad_y`, never below `ROW_PAD_Y`) — left at
5px, a 16px chat's 5.5px overhang would reach into the row next door. A
cheer's picture overhangs the same way (`ChatView::cheer_piece`).

**An emote can be drawn over the one before it.** Some third-party emotes
are not pictures of their own but something done to the emote beside them.
Each emote carries an `emotes::Kind`, read off its provider's answer when the
set loads (`providers`, each parser tested on a trimmed copy of its global
answer of 14 October 2026, in `crates/emotes/src/fixtures/`), and
`emotes::stack` moves each into the emote it belongs to by its provider's
own rule; `Kind`, `providers` and `stack`'s module doc have the rules. Its
match over `Token` has no wildcard arm, so a new kind of token does not
compile until it says how it stacks, and `emotes::message_tokens` is the one
place the passes are put in order.

`chat::emote` draws a stack as one `flex_none` piece of the line, in the box
the emote alone has always had, so a line with a stack in it is no taller and
wraps the same. Overlays take no listener, tooltip or cursor, so gpui gives
them no hitbox and the emote under them answers the pointer for the whole
stack, and each is keyed `over ` and its URL, apart from every `id_key` (see
"Things not to redo"; the comments in `chat::emote` have why). Every tooltip
in chat — an emote's, a cheer's, a badge's, a Shared Chat tag's — is given
only while the pointer is in the window (see "A tooltip outlives a pointer
that leaves the window" under the GPUI traps): the left-hand pane's chat
runs to the window's edge.

**Effects are drawn as BetterTTV draws them.** BetterTTV draws both
providers' effects, mapping FrankerFaceZ's `ffzW`, `ffzX`, `ffzY` and
`ffzCursed` onto its own `w!`, `h!`, `v!` and `c!`, and `emotes::effects`
does what its stylesheet does, pure and tested: the module doc has every rule
at BetterTTV's 28 px emote height, which is Perch's at Default, and the
cascade that settles them (`h! v!` is upside down only). gpui's image sprite
has no transform, but nothing needs one. Wide and zero space only move the
emote (`effects::WIDE`, `ZERO_SPACE`); everything else changes the picture,
so `chat::emote` asks the pane's `chat_effects::Pictures` for it by the
emote's URL and its `Effects`, and the first ask makes it on a background
task (`chat_effects::make`: decode as gpui decodes, `effects::apply`, one
`RenderImage`). Party and shake run on a clock of their own, and the made
loop is what gpui repeats, so an animated emote's loop is run again until it
ends where their period does, within `effects::MADE_BYTES`, 4 MiB of frames
(`effects::timeline` and `loop_end` have the rules and the numbers). Until
the picture lands the emote's name stands in, as it does while the file
downloads; a picture that cannot be made is logged
(`emote effects: <url>: ...`) and the emote drawn plain.

The picture goes where `effects::Placement` says, in emote heights: the box
the line makes room for, and the picture's rectangle in it — a turn in a
square at the emote height, a shake's canvas 2 px of BetterTTV's larger on
every side, so nothing it moves is cut off. The box has to be given to the
wrapper: taffy sizes a block by the width a child sets and leaves the child's
margins out (see "Things not to redo"), so left to the margins a shaken emote
widened its line by its reach. Its id is `Emote::id_key` behind `fx `: a made
picture has frames of its own (a still emote shaken is ten), and the plain
picture drawn when one cannot be made must never share its frame state.
Overlays on an emote with effects are laid over its box plain, as BetterTTV
puts the class on the emote, not on what is laid over it. An effect
`Effect::named` does not know keeps the fallback the user was asked about:
left out of the line, the emote drawn plain, and the tooltip saying
`not drawn`.

A shake hangs past the emote's overhang by its 2 px — 1.5 px past a row's
padding at Default, and inside a wrapped message 6.5 of the 9 px between its
lines — so it can cover a pixel or two of an emote overhanging from the next
row or line, never a line's text, in four of its ten 50 ms steps; across, it
moves into the box's own `EMOTE_PAD_X`, which at Default is exactly its
reach. Kept inside its box it would have to be drawn smaller than the emotes
beside it, which BetterTTV's is not.

`Pictures` keeps at most 64 made pictures a pane and 32 MiB of their frames,
and hands back the atlas tiles of what goes (`App::drop_image`) at the start
of the pane's next `render`, before any row is drawn, and of everything in
its `on_release_in`, from every window, since a pane can be drawn in a
pop-out; the `chat_effects` module doc has the rest. FrankerFaceZ's global
answer is cut to its `default_sets`, or its supporters' effects (`ffzHyper`,
`ffzSpin`) are pictures in everyone's chat (`providers`; see "Things not to
redo").

**Chat display options are every chat's, from any pane's header.** The chat
options icon on a header that sits on a chat panel opens `watch::chat_menu`:
four text sizes, `Time on every message`, and `Reset pane sizes` ruled off at
its foot, which is `RootView::reset_sizes`, the one path `Ctrl+0` takes too
(see "Sizes the user dragged are settings"). The two options are
`Settings::chat_text_size` and `Settings::chat_message_times`, and the
settings sheet does not own them (`SheetFields`), so saving it keeps them.
The root is the one writer (`RootView::set_chat_display`); each `ChatView`
mirrors them through `set_display`, and `chat_display::Metrics::of` derives
everything a row's height is made of from the size in one place. Whatever
changes rows already drawn — a new size or times setting, or a Shared Chat
label, badges or cheermotes arriving — hands every row back to the list
unmeasured (`ChatView::remeasure`), or the list goes on laying rows out at
their old heights and they overlap. An event's time goes on its first line
only, for the reason `render_event` gives.

The menu is a layer over the pane's chat box, drawn after chat
(`watch::pane`), rather than hung from the header, which is drawn before
chat, so anything hung from it would be painted under chat. In a box shorter
than the menu it rises from the box's foot over the header and the picture
(`chat_menu::placed`), and it is not deferred, which would take it out of
the page's clip that keeps everything under the title bar off the bar's
caption areas; `chat_menu`'s module doc and `placed` have the rest. The root
also closes it when its pane stops being drawn on a chat panel or leaves
(`retire_slots`, `replace_slot`), or it would open again by itself the next
time that channel did. It occludes chat, so a press on it hands the keys back
by hand (`VideoView::return_keys`; see "An occluder hides presses from the
root's `track_focus`").

**Between two *wrapped* lines of one message there is no padding at all** — they
sit exactly `LINE_BODY` apart — so an emote on the second line paints straight
over the descenders of the first and gets painted over in turn. A message
that contains an emote or a cheer gets a `gap_y` of twice the overhang
between its wrapped lines (9 px at Default; `Metrics::overhang`); a wrapped
wall of plain text keeps its tight leading, and a single-line row stays 29px
at Default, a 19px line and 5px of padding either side. It looks like
clipping, but nothing masks to a row (`Style::overflow_mask` is `None` unless
overflow is set, and `list` masks to the whole list), and raising `ROW_PAD_Y`
changed nothing, because the emotes measured a full 28px either way: measure
the row pitch and the emote's actual height before believing a screenshot.

**Scrollback holds where you put it** and resumes only when the wheel reaches
the very bottom, which in a fast channel is a long way down. The scrollbar and
the jump-to-live pill exist because nothing on screen said so otherwise — see
the `list` trap for why the thumb's *size* is not to be trusted.

**New rows wait while the pointer is over the pane.** Not by pinning the list:
a pin at the bottom of a bottom-aligned `list` is cleared by its next layout
pass, and pinning at the first visible row means reproducing `scroll_by`'s
arithmetic. The rows are held in `ChatView::held` and pushed when the pointer
leaves, only while following live, since a pane scrolled back already holds
its own position, and the check is measured, so a pointer that leaves the
window without a move event still releases them at the next repaint.
`ChatView::sync_hold`, `append` and `push_all` have the rest, the copy menu's
hold and the cap on what is held among it.

**A right-click on a row offers to copy it.** gpui's text cannot be
drag-selected, and a message is a row of separate words and pictures besides,
so `chat::copy_menu` copies the pieces whole — the message as sent, the name,
a link, an emote's name with whatever is stacked on it as it was written —
decided by `entries`, pure and tested; its module doc has when it opens and
closes. A right press on a name opens the channel menu instead (see "The
channel menu" under "Where controls live"). The copy menu is the chat's own
state, not the root's as the chat options menu is, because what it offers is
the chat's rows; it is deferred (see the GPUI traps), and its rows ask the
root to copy (`ChatViewEvent::Copy`; see "The clipboard is the app's"). It
occludes, but the panes' probes still see through it, as they see through
everything but a person's card (`crate::veil`), so a menu reaching over the
next pane can bring that pane's bar up under the pointer.

**A name opens its person's card.** A left press on a speaker's name in a
line, a resub note's included, or on an `@mention` that is a login
(`twitch_chat::is_login`), opens a card in that chat's list box
(`chat::viewer_card`, whose module doc has what it shows, where it goes and
when it closes; README has each part). Its lines are found by login among the
rows each time the card is drawn, never kept by index, and drawn by
`ChatView::message_words`, the chat's own line-drawing. The account behind it
is Helix's Get Users, asked by the root through the worker when a card
opens, once a login a session (`chat_profiles`); a failure is asked again
only when that person's card is opened afresh, so `RootView::tell_card`,
which runs after every worker event, cannot ask in a loop.

It is a plain absolute child of the list box rather than deferred as the copy
menu is, so the palette and the settings sheet draw over it. The box has no
clip, so `viewer_card::place` is what keeps it in; in a short chat it stands
260 tall on the box's foot, the least that keeps its head and foot whole
(`the_least_card_keeps_its_head_and_foot`), and rises over what is drawn
before the chat, never down into the next pane, which is drawn after. Risen,
it writes itself into the veil every frame (see "A probe sees through an
overlay too" under the GPUI traps); the same name pressed again closes it
(see "A toggle that a press outside also closes"); and it occludes, handing
the keys back in the capture phase (`ChatViewEvent::ReturnKeys`; see "An
occluder hides presses from the root's `track_focus`").

Its foot's pills act on the press: `Watch`, `Channel page` (signed in only,
since the page asks Helix for its lists with the user's token) and
`twitch.tv`. `Channel page` leaves the watch page, which stops every pane at
home (`go_browse`), and Back does not bring them back, so where it would stop
any (`leaving_stops_panes`, decided at the press) it asks for a second press
first — `Stop streams and open?`, in the accent — and only a second press
within `theme::CONFIRM_LINGER` goes (`ChatView::confirm_leaving`). Nothing on
the card acts on the person (see "Product decisions").

**Shared Chat copies are labelled with the channel they came from.** In a
Shared Chat session a line said in any of the channels is copied into the
others, and Twitch marks every line of the session with `source-room-id`, the
room it was said in, beside `room-id`, the room it is delivered to. Only the
copies differ, so `twitch_chat::ChatMessage`'s `source_room` is set only
where they do. A copied event carries it on the notice itself
(`ChatNotice::source_room`), because most are Twitch's sentence alone, which
is exactly the line that would read as if it happened here, and arrives as
`msg-id=sharedchatnotice` with its real kind in `source-msg-id`, which
`ChatNotice::from_irc` classifies by. A replay's comments have no such field
and are never labelled.

The line carries only a numeric id, so the root asks for the name, once per
id a session (`shared_chat::SourceRooms`), as `Request::ChannelNames`:
`users(ids:)` on the anonymous GraphQL endpoint
(`twitch_api::recommend::channel_names`, verified 2 October 2026). Helix has
the same lookup behind a token; the GraphQL one keeps a label free of the
session. The names live on the root and every chat mirrors them
(`ChatView::learn_rooms`), and until one is known a copy wears nothing rather
than a placeholder that would have to come off again; the `shared_chat`
module doc has the rest. The label is `controls::quiet_tag`, solid rather
than a translucent wash, because a wash over an event row's loud wash lifted
the background under the text below the contrast bar
(`a_quiet_tag_reads_on_every_chat_row`).

**Names wear their badges, from Helix, signed in.** A line names its badges
and nothing more (`twitch_chat::message::parse_badges`, which puts
`badge-info`'s months on the badge of the same set; `userBadges` in a
replay). The pictures and titles are Helix's (`twitch_api::badges`), asked
by the root as a chat says its room: the global book once a session and each
room's once (`root::badges`, `chat_badges::Library`). Like every ask the root
makes for the chats — the Shared Chat names, the cheermotes, a card's
account — it goes through the worker and only signed in, since the worker
reads requests only from its loop; one met before sign-in is made when
sign-in lands, a dead worker's are forgotten with the rest (`forget_asks`),
and what a failed one does next is in each module's doc. A badge is under
its line at every text size (`Metrics::badge`), so a row with badges is no
taller than one without, and signed out no chat has art, so no group is
drawn and no gap is left.

**A cheer is drawn as Twitch draws it, signed in.** A line that spent Bits
carries `bits`, the cheer words' amounts summed, and Twitch sends it on
nothing else (`twitch_chat::ChatMessage::bits`). What a cheer looks like is
Helix's Get Cheermotes (`twitch_api::cheermotes`), asked once a room a
session as the badges are (`root::cheers`, `chat_cheers`), and its answers
last the session, a sign-out included. A replay is never asked for and never
handed a book. Which word is a cheer is Chatterino's rule
(`ChatCheers::find`), and the `bits` tag is the budget
(`emotes::apply_cheers`): one that would overspend stays a word, so a
`Cheer100` typed beside a 1-Bit cheer is not drawn as a hundred. A cheer
overhangs its line as an emote does (`ChatView::cheer_piece`), so a wrapped
message with one gets the emote's `gap_y`, and its amount is lifted against
what the line is drawn on, as a name is; `chat/cheers.rs` has the rest, and
Known limit 38 where a cheer stays a word.

**A timeout or a ban greys that person's rows.** `CLEARCHAT`'s `ban-duration`
(seconds, present only for a timeout) rides `ChatEvent::Cleared` as
`ban_seconds`, and the notice says `ronni was timed out for 10 minutes` or
`ronni was banned` (`chat_words::ban_notice`). Every row of theirs is greyed
where it stands, as a `CLEARMSG` greys the one row its `id` names, and rows
of theirs held under the pointer are dropped; on an event carrying a note
they wrote, the note is greyed while Twitch's sentence above it keeps its
colour, since Twitch said that, not them (`ChatView::clear_person`).

**A reply carries its parent above it.** The `reply-parent-*` tags become
`ChatMessage::reply`, drawn as one dim line above the message
(`ChatView::reply_line`, the shape the `.truncate()` trap says works), with
the message in a `flex_row` of its own beneath it (the event-body trap
above). Twitch starts the reply's own text with `@parent`; that first word is
skipped when the words are drawn (`chat::reply::skips_leading_mention`), not
cut from the text, because the `emotes` tag counts characters from the text's
start. Live only.

**The room's modes are merged and shown quietly.** `ROOMSTATE` comes once on
joining with every mode and again with only what a moderator changed, so
`twitch_chat::room` merges each line over the room as it was
(`RoomModes::merged`); after the first line, a change is a notice row, and
while any mode is on one dim line under the list says so, outside it so it
never scrolls. The emote sets reload for a new room and on a full line
(`ModeUpdate::is_full`, which Twitch sends on every rejoin), not at every
`ROOMSTATE`: reloading at a reconnect is what retries a provider whose fetch
failed or timed out. `ChatView::room_state` has the rest; `ChatView::modes`
is what the composer reads (see "A live chat can send").

**A live chat can send.** The user once ruled sending out ("a viewer, not a
chat client") and has since reversed that: it will not be used much, but its
absence was the biggest thing missing. Reading stays anonymous IRC; sending is
Helix's Send Chat Message (`twitch_api::chat::send_message`), scope
`user:write:chat`, asked of the worker (`Request::SendChat`) like every other
Helix call, so the token has one owner and the window never waits. Not IRC's
`chat:edit`, which would mean an authenticated connection per pane beside the
anonymous one, or reading with the token too. A sent message is not drawn
when it is sent: it appears when IRC echoes it, like anybody else's, so what
the pane shows is what the room got. Twitch can answer 200 and not send
(`is_sent` false); its `drop_reason` becomes a notice row in that chat
(`composer::outcome_notice`). With a user token in a Shared Chat session the
message goes to every channel in it; Twitch allows nothing else for that
token type.

Each send carries a number of the root's (`chat_send_seq`), so the answer
finds the chat it was typed into even if the panes moved (`root::chat_send`).
A worker that stops — signing out, signing in again, a new client id, a
terminal sign-in failure — takes its unanswered sends with it, and each of
their chats is told (`drop_chat_sends`): after a terminal failure that they
were not sent, and after a stop the root asked for that they may not have
been, since one can be mid-request (see "The second scope, and signing out").

The composer (`chat::composer`, whose module doc has its states and the box's
rules) closes for a followers-only room where the follows list says the user
does not follow, and that is said only on the whole list of follows: the walk
stops at ten pages, a thousand channels (`twitch_api::Followed::complete`),
and a channel missing from a list cut short is not known either way
(`composer::follows`), so the box stays open with the hint. Out of scope for
now: an emote picker, replying to a message, moderation commands and
whispers.

**A few messages wear a wash.** `chat_tint` picks it, pure and tested: a line
addressed to the signed-in user first (`chat_tint::addressed_to`), read as
the row is drawn from the composer's `Access` (`ChatView::me`), so a sign-in
after the line arrived still marks it and a replay marks nothing; then
Highlight My Message, a first message, and an announcement's own colour,
mapped to washes of the app's own. Each new wash is as strong as its hue
allows with `text_dim` and the accent still at 4.5:1 on it
(`every_chat_wash_keeps_its_text_legible`); the loud wash predates that test,
measures about 4.4:1 for the accent and 4.0:1 for `text_dim`, and is left out
of it rather than changed under raids.

**A pane opens with what was already being said.** Twitch publishes no
scrollback — IRC gives you what arrives after your JOIN and there is no Helix
endpoint — so the backfill comes from the community service Chatterino and
DankChat both use. It answers with **raw IRC**, so the lines go through
`message::parse_line` and `event_for` exactly as if they had come off the
socket, and there is no second code path to keep in step. It is the only
place the app asks a **third party** for content, which tells that service
which channels are being watched (`Settings::chat_history` turns it off); it
runs **before** the socket, so history never lands below live lines, and
only on the first attempt, since a refetch would re-deliver what is still on
screen; and the very first request for a channel nobody watches comes back
empty (`twitch_chat::history`).

**"Connecting…" is a state of the pane, not a row in it.** A backfill arrives
stamped with the times those messages were really sent, so a row would sit
above an hour of history wearing a later timestamp than everything beneath
it. It is drawn in the empty space it explains instead, and the pulse is safe
there because the state always ends — at the join, or at the first
disconnect notice, and both put a row in the list.

**Events are washed, never outlined or barred.** A sub, a gift, a raid or an
announcement gets a tinted row and nothing that changes its geometry, because
the row still has to keep its place in the rhythm you scan down. Two
intensities and no more: `msg-id` is an open set that grows whenever Twitch
ships a feature, and the `system-msg` tag is already finished English that says
which event it was. Anything unrecognised renders in full with the quiet wash —
the cost of a missing arm is a row that is not tinted, not a dropped event.
An announcement has no sentence at all and is nothing but body, which is why
`ChatNotice` has both halves optional and why a notice with neither is dropped.
An announcement's own colour is the one exception to the two intensities; see
"A few messages wear a wash".

**A recording's chat is replayed from Twitch's own query.** There is no Helix
endpoint for it. `twitch_chat::replay` asks Twitch's GraphQL for the
persisted query `VideoCommentsByOffsetOrCursor` (`QUERY_HASH`) with the
mobile Client-ID (`CLIENT_ID`), because the website's own fails Twitch's
integrity check on every cursor page (`IntegrityCheckFailed`); the module doc
has the fallback should the mobile one fail too, and what each odd answer
means. A page is fifty-odd comments spanning a few seconds of a busy chat;
`first` is capped at 100 and does not enlarge one. Thirty requests back to
back answered in about 0.3 s each with no throttling; the replay keeps one in
flight and paces refills at two a second regardless. If the hash or the id
ever stops working, the pane shows one notice row and the picture keeps
playing; `crates/twitch-chat/src/fixtures/video-comments.json` is a page as
Twitch sent it on 9 September 2026, trimmed, and is what the parser is
tested against.

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
`ChatEvent::Message` when the position passes it, a busy second's lines
spread across it (`Schedule::release`). A seek — the position back by more
than a second, or forward by more than the time that passed, times the pane's
speed, plus two (`replay::Playhead`; see "A recording can play faster or
slower" under Recordings) — reloads the twenty seconds before the target, and
only then does the pane get `ChatEvent::Reset` and that backlog, so a seek
swaps one conversation for another rather than blanking the pane for the
round trip. Once the comments run out the thread asks again every ten
seconds, which is how the replay of a broadcast still being recorded grows.

**The position is the pane's, not the player's.** `video::PositionHandle` is
made in `open_video_at`, lives on `Source::Video`, and is handed to every
`VideoStream` the pane starts, so a quality change carries the position
across and the replay sees playback continue rather than a jump. It is made
holding the place the pane opens at (`PositionHandle::starting_at`), since
the replay follows it seconds before any player exists (see "A player
reports its position to itself" under Recordings). "Watch again" starts a
player at zero, which the replay reads as a seek back and reloads from the
start. The history reads the same handle, which is why a pane still opening,
or one whose player has stopped, is noted where it is rather than at zero.

**Two events only a replay sends, and one that means something else.**
`ChatEvent::Reset` clears the rows and keeps the colour map — same chat, same
people. `ChatEvent::Unavailable` is one notice row — "no chat replay for this
broadcast", or a video that is not on Twitch — after which the thread retires.
`Connected` is the join on a live pane and puts a row in it; for a replay it is
the buffer being ready, puts no row (a row stamped with today would draw a
time break between two of last Tuesday's minutes), and is what turns an empty
pane's pulsing "loading chat replay…" into a still "nothing said here yet". A
request that fails says "Chat replay interrupted: … — retrying" once per
streak and backs off to thirty seconds; the pill at the bottom of a scrolled
replay says "↓ Newest", since nothing about it is live.

### Keyboard

Key handling is mostly about two GPUI behaviours that fail *silently*, and
`keys`'s module docs give both, with why there is no transient feedback, the
one exception to it, and what a pop-out hears. What follows is what they
leave out.

**A key reaches nothing unless something is focused.** `RootView` holds a
`FocusHandle`, takes focus on open, and takes it back through
`cx.on_focus_lost` whenever the focused element simply *disappears*, which is
what happens when the settings sheet closes and takes its buttons with it.
That listener fires only when the path empties, not when focus moves, so it
cannot loop. Without either half, every binding is dead and nothing says so.

**A pop-out has a focus of its own, and keys of its own.** Every gpui window
has its own focus and dispatch tree, and a handle no element of a window
tracks reaches only that window's outermost node (window.rs,
`focus_node_id_in_rendered_frame`), where nothing is bound. So each
pop-out's view (`root::pop_out::PopOut`) tracks a handle of its own, takes it
back the way the root does, and reports `keys::CONTEXT_POPOUT`, whose doc
says what it hears; its player hands the keys back to that handle, not the
root's (`VideoView::set_place`).

**`Esc` on the watch page takes back one thing at a time.** A header being
dragged, an open menu, fullscreen, a pane given the whole page, and only then
the page itself, which stops what plays at home; `RootView::on_go_browse`
gives the order and why fullscreen comes before the page. While a pane has
the page, choosing another moves the maximize to it (`RootView::choose`),
and once the pane the keys talked to has closed they fall back on a cell the
grid draws (`RootView::active_slot`), so `Space`, `M` and `Ctrl+W` never act
on a pane drawn nowhere. `Z` is bound on the watch page alone: the browse
page draws no pane, and a pop-out's pane is not on the page to fill it.

**The title bar's search box outlives a change of page.** It is in the bar
over both pages, so a cursor left in it — where a search leaves it — makes
every key on the watch page stand aside for it, Space typing a space into the
search. So whatever the root's own mouse-down cannot hear takes focus back by
hand: the title bar's buttons, the browse page's arrows, a toast
(`act_on_toast`), the side buttons, and a pane opened with no press on the
page at all (`RootView::show_watch_page`, which says why only from the
search box). A press on a pane's own control bar or menu, which hide it from
the root, gives the keys back from the player (`VideoView::return_keys`,
holding the root's handle from `Start::focus`); see the GPUI traps for why it
listens in the capture phase.

**A key context and a key predicate are different grammars.** A context is
whitespace-separated identifiers (`Perch Watch`); a predicate is a boolean
expression over them (`Perch && Watch`), and interpolating the first into the
second panics `KeyBinding::new` at startup. `keys.rs` builds both from the
same identifiers, and a test pins that they agree.

Three more things worth keeping:

- **Never bind with `context: None`.** It outranks every scoped binding and
  eats typing (see `keys`). Everything here is scoped and carries
  `keys::TYPING`, whose doc says why `!X` works.
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
  it opened and does nothing over the sheet, which is drawn over the palette
  (`the_palette_key_waits_for_the_settings_sheet_to_close`).

There is deliberately **no transient feedback** for pause, mute or volume
(see `keys`). What there is is a standing one: the pane header carries `muted`
and `paused` tags, read off the `VideoView` at render time, so a channel saved
muted does not open silent with nothing on screen to say so — except with
chat hidden, where the header rides the band over the picture ("Known
limits", item 28). The quality is deliberately not in the header: it is on
the control bar. When a header runs out of room the name gives way first,
with the numbers and the ad-break tag, and ellipsises (its tooltip still
names the channel): the right-hand cluster is `flex_none` and must never be
pushed past the edge, since it carries the × (`header::name_link`,
`with_status`).

The shortcut list in the settings sheet is `keys::SHORTCUTS`, which
`every_documented_key_is_actually_bound` holds to the keymap. The README's
keyboard table is held to the same list by `the_readme_lists_every_shortcut`,
whose doc says what it looks for, why not on macOS, and that the `RUNNING`
pages are kept in step by hand.

The one exception to "no transient feedback" is **which pane the keys talk
to**: pointing at a pane is what makes it active, so the pointer can never
answer the question. A pane key, `C` hiding a chat, or a pane moved by
`Shift+←`/`Shift+→` or a header drop brings that pane's band up for
`theme::HEADER_REVEAL`, underline and all when it is marked
(`RootView::reveal_header`, whose doc lists every caller). One pane at a
time: a reveal takes it off the others. It sets `Slot::revealed`, which
`band_wanted` reads, and a detached timer clears it, numbered by
`reveal_epoch` so an earlier reveal's timer cannot take down a later one
(`reveal_is_current`). A header in the panel is always on screen and gets
none. Never on `Space`, `M` or the plain arrows, which answer for themselves.
Whether brief chrome after a key or a drop suits is a product call, so the
reveal can be dropped as a unit, from its callers.

The controls are the fourth place a key is named. A tooltip such as `Pause
(Space)` never spells a key: it names a keystroke as a `keys::Hint` and
borrows the label the sheet shows for it, and
`every_hint_names_a_listed_and_bound_key` holds every hint to a listed key
bound to the control's own action (see `Hint`).

**Fullscreen** is `f` on the watch page, `F11` on either, and a double-click on
the video. The title bar goes with it (`layout::title_bar_height`), and the
rail too, folded or not (`layout::rail_shown`, which says why), so `Ctrl+F`,
`B` and the palette's rail row do nothing there rather than act on what is not
drawn (`on_focus_search`, `toggle_sidebar`). The mouse's way out is a
double-click on the video, which brings the bar back. The double-click lives
on the pane's root element; the control bar over it is `.occlude()`d while it
is up, so a double-click on one of its controls does not also reach it.
A single click deliberately does nothing there — it is how a pane is made the
active one, and pausing on a click would turn choosing a pane into stopping it.

**`Shift+←` and `Shift+→` move the active pane** one place along the order
`1`–`4` count, swapping it with its neighbour exactly as dropping its header
there would (`RootView::move_pane`), and only where a header can be dragged,
with two panes or more on the page. `keys::MovePaneEarlier` says why along
the order rather than across the screen, and its binding why not `[` and
`]`. Clamped at either end (`stage::moved`) rather than wrapped, which would
renumber every pane at once. The pane moved keeps the keys, and every pane
counts as pointed at already, so the one the swap put under a still pointer
does not take them.

**`1`–`4` and `Tab` choose the pane the keys talk to**, through
`keys::ActivatePane`, the one action here that carries data. `Esc` on the
browse page is `StepOut`, the other direction from the watch page's `Esc`,
each scoped to its own page; `keys::StepOut` says why it is named apart from
`NavigateBack`. The watch page's `Esc` closes a pane's open menu before it
leaves: the menus are hand-rolled (`video_view/menu.rs`), so they have no key
context to catch `Esc` themselves, and `RootView::on_go_browse` asks every
pane to close its menu first (`close_menus`). A pane has one menu open at
most, and every one opens from the bar's right-hand cluster, the menus' one
anchor; `pane_cluster` in `video_view/bar.rs` says why the dismiss is on the
anchor rather than the menu, and what inside the anchor closes the menu
itself. The quality menu has no key of its own: the palette's `Choose
quality for …` opens it, and with it the bar (`RootView::choose_quality`), its
rows after every `Close` row so that `c quin` stays a close
(`an_abbreviated_close_leads_with_the_close`). A pane still waiting for its
first frame is offered no row, and `VideoView::open_menu` refuses it; its doc
says why. More's two rows are in the palette by name as well — `Copy link to
…` and `Open … on twitch.tv`, for every pane, a stopped one included — and go
down the route More's own rows take (`RootView::on_pane_action`), so the link
and the toast are the same.

**Back and forward are `Alt+←` and `Alt+→`** (`⌥` on macOS), as well as the
arrows on the browse page's tab strip (`root/pages.rs`) and the mouse's side
buttons. The keys are bound twice, on the watch and browse predicates, never
on `anywhere`, which a modal's context also satisfies, and their bindings say
why they carry `TYPING` although they are chords. The listing gives them two
rows (`keys::SHORTCUTS`), and `alt!` writes the modifier the way `secondary!`
does, held to the keystroke beside it by
`the_listing_names_the_modifier_it_binds`. The side buttons are heard by
`RootView::side_buttons`, a window-level listener like the divider drag's
(see the GPUI traps), in the capture phase and on the press; its doc says
what a press with nowhere to go does, and why. With the sheet or the palette
up they do nothing, like the keys (`RootView::modal_open`). On Windows they
are deaf over the title bar's empty strip and the caption buttons; see "Known
limits", item 21.

### The window and one perch

**The window remembers where it was.** `Settings::window` is written from
`on_window_should_close` with the platform's restore bounds, so a maximised or
fullscreen window is saved as the size it would un-maximise to. On open it is
used only if some display still holds it — a monitor unplugged since is the
common way to lose a window — and otherwise the default is centred and fitted
to the primary display (`main::initial_window_bounds`). On Windows the saved
bounds are gpui's client area, worked out from the frame with the border
offset gpui measures and turned back into a frame the same way on open, which
can let the size creep smaller (see "Known limits", item 22).

The window has a least size, `root::window_min_size`, set in `main`: as wide
as the title bar with nothing that must stay on it pushed off (`least_width`
in `root/title_bar.rs` says what that is), and as tall as the bar. Without
one, Windows let the window narrow until Maximise and Close ran off its right
edge, where nothing could press them.

**On Windows the display has to be named.** gpui keeps saved bounds only when
their centre is on the display in `WindowOptions::display_id`, and with none
that is the primary — so a window closed on a second monitor opened at a
default size on the first, every time, while the check above passed.
`main::home_display` picks the display holding the centre and hands its id
over. Not on macOS, where gpui reads the bounds *relative* to the display it
is given: the saved bounds are global, so naming one would shift the window by
that display's origin.

**One perch.** The `instance` module's docs say how a launch claims the
settings directory or hands its arguments to the perch that has it, and
`instance/windows.rs` what the pipe is named for. `instance::claim` runs first
thing in `main` — before `capture_stderr` too, which starts by renaming the
log and would move a running copy's log out from under it. A launch is
answered only once its arguments are queued for a window that still exists:
`main` closes the queue the moment the window starts to close, so a launch
arriving after that is turned away, tries again, and becomes the next perch as
soon as the old one lets go, rather than being answered by a process on its
way out. What is handed over opens beside whatever is playing, `--volume` and
`--help` included (`root/launches.rs`).

**Coming forward without a keystroke.** gpui's `Window::activate_window` on
Windows earns the foreground by pressing Alt through `SendInput`: a real
keystroke, into whatever had the keyboard. Instead the launch that hands over
passes its right to the foreground to the running copy before it says
anything, and the running copy restores itself if minimised and comes forward
on its own handle (`give_way` and `instance::bring_forward`, in
`instance/windows.rs`), which is what Chromium's process singleton does.

---

## Working on it

The checks a change has to pass, and how to run, stop and drive a test copy
beside the user's own perch, are in CLAUDE.md ("Checks" and "Running a test
copy"). What follows is what those leave out.

**A running perch locks its exe.** Windows fails the link step of a build that
would replace a running `perch.exe` with "Access is denied", and the user's
own perch may be running from `target\release`, where `run.cmd` starts it.
Never close theirs to build: a test copy is a debug build, and a release
build — always release for anything perf-related — goes into a target
directory of its own (`CARGO_TARGET_DIR`).

**One perch at a time is enforced.** A perch started on settings another perch
holds hands its arguments over and exits — `cargo run` and `run.cmd` included,
which then seem to do nothing but bring the user's window forward, with
whatever they named opening in it. A debug build says so in its console. The
two were never safe together, since they spent each other's sign-in, so a
test copy runs on settings of its own.

**`cargo build | tail` reports `tail`'s exit code, not cargo's.** Use
`set -o pipefail` and `${PIPESTATUS[0]}`, or a bare build. This produced at least
one false "build OK".

**Large multi-file edits: write a Python patch script to the scratchpad and run
it**, rather than shell heredocs, which fight you on quoting. Make the script
idempotent (treat an already-applied replacement as success) so a partial
failure can be re-run safely.

**A test build can run beside the user's own perch.** The two share no files:
settings come from `APPDATA` (`settings::default_path`), the image cache and
the log from `LOCALAPPDATA` (`root::image_cache_dir`), and the one-perch pipe
is named for a hash of the settings directory (`instance/windows.rs`, `key`),
so nothing is handed over. They do share the registry: a test copy that shows
a desktop notification takes the `perch:` scheme and the toasts' icon over,
and one whose notifications are turned Off from its sheet takes both keys out
from under the user's (`notifications/windows.rs`). Give a test copy
`"desktop_notifications": "off"` in its `settings.json` before it starts.

**The user's own perch is usually running, and is theirs.** Leave their
settings, their `perch.log` and their sign-in alone; whatever needs the
sign-in is unit-tested, or checked in their session with their consent.
`Copy link` only with their consent too, saving and putting back what the
clipboard held.

**Verify visually.** Much of this was found by capturing the running app and
reading the PNG. Capture a test copy with `PrintWindow` and
`PW_RENDERFULLCONTENT`, which draws it whatever is in front of it, never with
`CopyFromScreen`, which captures whatever is on top: the user's own windows.
The window rect includes invisible resize borders at the sides and bottom —
trim ~8px there; there is no caption to trim at the top, since the title bar
is Perch's and part of the client area.

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

**A swap says what it did in the log**, under the pane's key: from `starting …
beside …` to `swapped A->B after N ms; position a->b`, or `couldn't swap to …`
and why, or `called off the swap to …`, with a start ahead's lines before it
when one was taken. The swap trap under Video reads them, and
`video_view/swap.rs` and `root/warm.rs` write them. The log is `perch.log` in
a release build and the console in a debug one. A pane that went back to
`Starting…` for a quality change was started cold, not swapped: its picture
did not cover it yet (`restart_how`), or the change came from the settings
sheet, which always restarts cold (`restart_stream`).

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
after a row that picks a quality other than what plays on a pane whose picture
covers it, which holds the bar up, with no blink at the press, until the switch
ends and `theme::SWITCH_LINGER` after; only More over the quality menu, the
pill over More and More's quality row put one menu in place of another and keep
it. So open the menu again from the palette before each press. Rows, the pill,
More, chat and fullscreen act before the fade starts, and so do the slider and
the seek row, which act on the press (the seek row holds the bar while its
thumb is held, and seeks on the release wherever it lands; a live pane's
timeline, the same row, rewinds on the release instead, which replaces the
pane). Play and the speaker act on a click, and their press closes the menu in
the capture phase, which flips the bar's fade and so re-keys everything under
it (`motion::Fade::animation_id`): a frame drawn before the release forgets the
press, and no click comes. Post their down and up back to back, since gpui
paints only once the posted queue is empty. Never post two presses in a run
where the bar was: the first lets the bar go, which stops it blocking the
picture at once, so the second reaches the picture, where a double-click is
fullscreen. A stopped pane's band is up at rest, so its × and its screen's
controls take posted presses with no help. Tooltips and the hover lift need a
real pointer, so leave them to the user. Never post a press on anything that
opens twitch.tv — an `Open on twitch.tv` row, the pane header's name, the
palette's `Open … on twitch.tv` — which opens a browser.

**A stand-in streamlink reaches what Twitch will not do on cue.**
`STREAMLINK_PATH` pointed at a script that answers as streamlink does,
serving a local file, is how a stall (the `buffering` tag) and a streamlink
too old for perch were checked in a scratch copy, with no channel needed.

**Verify animation by measuring, not by looking.** One still cannot tell a fade
from a cut. Burst-capture with a stopwatch, reduce a small crop to a mean
brightness per frame, and read the numbers: a cut is one step between two
values, a fade has intermediates. The control bar measured
`0 → 4.73 @ 87 ms → 6.56 @ 159 ms` going in and the mirror image coming out,
which is what a 120 ms symmetric fade looks like. Keep crops small — sampling a
full window through `GetPixel` is slow enough to distort the timing you are
trying to measure. For the waiting pulse the tell is the *ratio*: trough over
peak came out at 0.45, which is `PULSE_FLOOR` exactly.

To reach a state that only exists briefly, drive the app into it rather than
racing a restart: changing quality in the settings sheet, a cold restart,
puts every pane back into `Starting` with the window already open and
stable. A pick from a pane's own menu or the palette does not, since on a
pane whose picture has faded in it swaps the new rendition in place.

**Measure the pixels, do not read the screenshot.** Two separate wrong diagnoses
of the emote overlap came from looking at a zoomed crop and believing it. What
settled it was measuring the row pitch down a column of the chat pane, then
scanning each row band for the tallest run of non-background pixels to get the
emote's real height. An emote that measures 28 in a taller row is not being
clipped, whatever it looks like at 6x.

**Verify pixel correctness by dumping PNGs.** `mpv-frames`'s `dump_frames` example
writes frames to disk with stats (per-channel means, alpha minimum, non-black
percentage). A red Superman "S" on a blue suit is how BGRA vs RGBA got confirmed.

**Clean up processes.** streamlink is a child of the app: a graceful close
takes it down, and on Windows so does a hard kill, through the job object; on
macOS a hard kill orphans it (see "Known limits", item 7). Kill an orphan by
its process id, never by name, which also takes the streams of the user's
perch.

---

## Shipping a build

`.github/workflows/release.yml` builds `--release --locked` on two runners and
zips each result beside `LICENSE` and that platform's `RUNNING` page. A `v*`
tag cuts a GitHub Release, whose assets download without an account — which
is the whole point, since the reason to build this at all is somebody who does
not want to compile it. Run by hand (`workflow_dispatch`), the same zips are
run artifacts, which need a login and expire: that is for handing a build to
somebody who is already here. The workflow's comments say why publishing is a
job of its own and why both macOS slices are built for 11.0.

**Windows** ships the bare `perch.exe`, and that is genuinely all it needs: the
widget icons are `include_bytes!` and the app icon is a linked resource, so
there is nothing beside the exe to lose.

**macOS** ships a `perch.app` around a universal binary, because on that system
the icon, the name and dock activation come from a bundle rather than from the
executable. `packaging/macos/bundle.sh` assembles it and says what it builds and
why, its ad-hoc signature included. CI runs it on the macOS leg against a debug
binary: a bundle that assembles and then will not launch is invisible to every
other check, and a tag push is the worst moment to discover one. Gatekeeper
still quarantines the download, and `RUNNING-macos.txt` opens by explaining how
to clear it. Notarising properly means a paid Apple Developer account, which is
a decision rather than a task.

What neither zip carries is libmpv — somebody else's licence to redistribute,
from an MIT app — or streamlink, which is a Python application. The `RUNNING`
pages say where to get both, and on macOS that is one `brew install` for the
pair. Neither is a silent failure: each surfaces in the pane, naming itself and
the environment variable that overrides the search.

**Before a release, run `scripts/live-check.sh`.** Everything perch rides
without a contract fails quietly by design — the Recommended group goes, a
replay says one notice row, emotes stay words, a renamed rendition never
plays — so nothing a user sees says a query was retired. The script's header
says what each crate's ignored `tests/live.rs` asks. It sends no token, and
stays out of CI: anonymous GQL from a shared runner's address invites blocks
and flaky failures. All of it answered on 14 October 2026.

## What to build next

Nothing here is agreed. Ranked by what would be noticed, roughly:

1. **A live link at a moment.** A live `Copy link` at a moment is the
   rewind's mapping (`rewind::archive_for`, `position_in`) with
   `Slot::rewind`'s archive, once found.
2. **Highlight rules** that wash the row background rather than colouring a
   word. The washes already exist for events, for Twitch's own marks and for
   lines addressed to the reader (`chat_tint`).
3. **The auth-token cookie off argv.** It is documented as a tradeoff, but a
   per-spawn `--config` file with a user-only ACL, deleted once streamlink has
   started, would take it out of the process list at the cost of one more file
   on disk. Not done here because it changes a documented decision.

Smaller, roughly smallest first:

- The room estimates, tuned against a capture of real panes:
  `theme::QUALITY_PILL_ROOM` ("Known limits", item 24) and
  `theme::STATUS_ROOM`, whose doc says what too small costs.
- `Start when they go live` lasts for the session and is per pane. Keeping
  it per channel would be a setting.
- Pin from the palette. `palette::entries` takes seven positional arguments,
  its tests call it some forty times, and it wants an inputs struct before it
  takes an eighth; keyboard-first, so it waits.
- A channel menu on what names a channel and has none yet: a recording's card
  (on a channel's page, in Library, Continue watching), the palette's rows
  and a viewer card's name.

Left out because nobody asked for them: insert-style reordering, a picture
under the pointer while dragging a header, dragging a pane between windows,
chat in a pop-out and a pop-out holding several panes.

**Not the buffered range on the seek bar.** It was here, from
`demuxer-cache-time`, but a recording's demuxer is only about a second ahead
(see "A recording paused for a while can come back to a dead connection" under
Recordings) and every jump is a reopen ("Known limits", item 16), so the range
would be a sliver that says nothing. What a viewer wondering about a stall
wanted, the `buffering` tag, is there, and the muted stretches are marked
already, from Helix (see "A recording's muted stretches are marked on its
seek bar").

**Known to be out of reach**, so nobody re-derives it:

- **Chatter counts.** The old `tmi.twitch.tv/.../chatters` endpoint was shut
  down on 3 April 2023 and returns 404. Helix `GET /chat/chatters` needs
  `moderator:read:chatters` *and* that the token's user is the broadcaster or
  one of their moderators — 403 otherwise. IRC `NAMES` via
  `twitch.tv/membership` still responds but stops listing above ~1000 users,
  so it returns nothing on exactly the channels worth asking about.
  `viewer_count` from Get Streams (no scope, app or user token) is the only
  public number. Do not go looking again.
- **Polls and predictions.** Creating or running one needs the
  broadcaster's own scopes (`channel:manage:polls`,
  `channel:manage:predictions`), so a viewer's token cannot.
- **Following and unfollowing.** Twitch took both out of Helix; a client can
  read follows but not change them.
- **Moderation and whispers.** Not out of reach so much as not asked for:
  each needs a scope of its own (`moderator:manage:*`, `user:manage:whispers`)
  and a UI.
- **Text selection across a message.** Every word is its own element and GPUI
  has no cross-element text selection, so there is no contiguous run to select.
  A row's right-click menu copies the pieces whole instead: the message, the
  name, a link, an emote's name (`chat::copy_menu`).
- **Link previews.** Chatterino ships this off by default on privacy grounds,
  which is a strong enough hint not to build it.
- **Chat scrollback from Twitch itself.** There is none. The website renders its
  own history server-side and exposes no endpoint; every client that shows it
  uses the third-party service `twitch_chat::history` talks to.
- **How long a chatter has followed the channel.** Twitch's own card says
  it; Helix answers it only to the channel's broadcaster and moderators (Get
  Channel Followers with `user_id` needs `moderator:read:followers` and a
  token whose user moderates the channel), and Get Followed Channels answers
  only for the token's own user, so a viewer's card cannot say it.

## Paging, and what is not paged

`twitch_api::Page` carries a cursor back out to the caller, and
`browse::Listing` holds one beside the items it belongs to, while the two
followed endpoints walk their pages inside the API layer until Helix stops;
both docs say why: you follow a fixed number of people, and "popular" is
every live channel on Twitch.

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
   scrolled back reading them. A row is a `ChatMessage` and a few
   `SharedString`s, and only the visible ones are ever laid out, so it can go
   further if it needs to.
3. **A quality change runs two streams for a few seconds.** The rendition
   is chosen again, upwards only, whenever a pane grows, and streamlink has
   no way to switch mid-stream, so the new rendition is resolved and played
   beside the old one until it takes over in place (the swap trap under
   Video): two streamlink sessions, two decoders and two audio outputs for
   the overlap, and a live cut that cannot be lined up, so it can step a
   little. Whether Twitch serves a second live session on one token is
   unverified: a live pick has been seen to swap in the user's own session,
   which did not settle whose token streamlink had. If it does not, live
   panes go back to cold restarts, and the way to keep them from going
   black is to copy the old view's last frame into an image of its own and
   draw it as a still under the starting screen until the new picture
   covers. The settings sheet's quality and credential changes still
   start every pane over cold, each back to its starting screen until its
   new picture arrives, and a pane that shrinks is left on the rendition it
   has. And a live pane's quality menu, while it is open and for five
   seconds after, keeps up to four more streamlinks waiting, idle (about
   45 MB each, no CPU, no network), so a pick does not wait for one to
   start (`root::warm`).
4. **Chat said while the connection was down is lost.** Backfill runs on
   the first connect only (`twitch_chat`, "Only on the first attempt"), so
   a sleep, a Wi-Fi drop or a NAT timeout leaves a gap the pane does not
   mark beyond its `Disconnected` notice. The history service's `after=`
   could fill it, but its changelog lists that parameter as unreleased,
   so it would have to be deduplicated by message id and tested against
   the live service first.
5. **A mention is matched by login alone.** A line is washed as
   addressed to the reader when it names their login as a word
   (`chat_tint::addressed_to`); a display name that is not the login in
   another case — a localised one, in another script — is not matched,
   since the composer's `Access` carries the login only. Nor are nicknames
   or any other word: there are no highlight rules ("What to build next",
   item 2).
6. **`ImageCache::new` is still on the pre-window UI thread.** It is
   bounded — `scan_and_prune` trims the permanent directory to 256MB, least
   recently drawn first, in the same pass that indexes it, as
   `emotes/src/cache.rs` explains — but it is one `read_dir` plus a stat per
   file, done synchronously in `RootView::new`. Deleting
   `%LOCALAPPDATA%/perch/images` is always safe.
7. **Orphaned streamlink on a hard crash, on macOS.** Windows ties every
   child to a job object (`streamlink::job`) that the kernel closes with the
   process, however it died; nothing equivalent is wired up on macOS.
8. **Little seen on a vertical monitor.** The layout derives portrait grids and
   stacks chat below video, and the logic is unit-tested. The user runs perch
   on a vertical monitor as well as horizontal ones, which is how two streams
   there were found stacking with each chat *beside* a squeezed picture;
   `theme::PORTRAIT_ASPECT` says how that was fixed
   (`two_streams_on_a_vertical_monitor_keep_chat_under_each`). Three and four
   streams there stack in one column with chat beside, which suits their wide
   cells; nobody has looked at them on the screen yet.
9. **The offline follows list has no cap.** Someone following several hundred
    channels gets several hundred names. The search box and the palette
    find them by a few letters of a name; the wall is still a wall. The rail
    folds it under its count and builds no rows folded, but unfolded it is the
    whole wall again — up to a thousand rows, none of them virtualised.
10. **Every settings save is a read-modify-write of the whole file, flushed
    to the disk.** Runs of changes — a volume drag — are coalesced into one
    write after they settle (`RootView::save_settings_soon`); single changes
    are still a write each. The flush is the cost of the sign-in surviving a
    power cut, and it runs on the writer's thread (`settings::writer`), not
    the UI's, so a slow disk delays the save and not the window — except as
    the window closes, which waits for the last saves to land
    (`RootView::finish_saving`). A save lands a moment after the press;
    the one reader that needs it landed is the next sign-in worker, and
    that waits for it.
11. **Chat history depends on somebody else's server.** If it is down the pane
    opens blank, which is what it did before the feature existed. Failures go to
    the log rather than the pane, on purpose.
12. **Shortcuts are not rebindable.** `keys::bindings` is a plain list built
    from constants, so the work is a UI and a settings shape, not a
    mechanism. Deprioritised, since the user is mouse-first (see "Product
    decisions").
13. **The palette scrolls with no visible scrollbar.** It is a window of eight
    rows around the selection, so arrowing scrolls it; the wheel works and
    nothing says so. The settings sheet, the browse lists and the rail draw
    gpui-component's `Scrollbar` over a tracked `ScrollHandle`.
14. **A pin nobody follows says less than a followed one.** It is asked
    about by itself (`root::pane_streams`), signed in, every two minutes
    and when it is made, so it shows live up to two minutes after it comes
    on, and stays live, count and preview card and all, up to two minutes
    after it ends; and it is not asked about before the list of everyone
    followed has come back after a sign-in. A stopped pane on such a channel
    reads the same answer, so its page's `Watch live` can outlast the
    stream as long. Off, it is its bare login, dim, with no picture and no
    "Live 3 hours ago": the worker looks up faces and last times for follows
    alone (`twitch::unpictured`). Nothing announces it coming on, toast or
    desktop notification, since both read the follows poll
    (`notifications::who`). A recommended rail row has no pin under the
    pointer, but its right-click menu pins it (`channel_menu::entries`
    always offers `Pin to the rail`), and the new pin is a bare login until
    the ask about it is answered (see "Pins from anywhere" under Browsing).
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
    requests behind it for as long as six requests take. When the offline
    follows were last live comes from the same endpoint and breaks the same
    quiet way (see "Each offline name says when it was last live" under
    Browsing).
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
    guesses until someone tunes them on a Mac. If dragging by AppKit's strip
    alone is not enough, porting Zed's `start_window_move` into the vendored
    gpui is the fix (a `PERCH PATCH`, listed in the root `Cargo.toml` and in
    `scripts/verify-vendor.sh`). `Cmd+[` and `Cmd+]` for back and forward
    were left unbound.
21. **The mouse's side buttons do nothing over the title bar's empty strip
    or its caption buttons, on Windows.** Those answer the platform's hit
    test as non-client areas, so a press there arrives as `WM_NCXBUTTONDOWN`,
    which gpui 0.2.2 does not translate (events.rs:84-91). Everywhere else in
    the window — the bar's own controls included — they go back and forward.
    Accepted rather than patched in the vendored gpui.
22. **The saved window size can creep smaller on Windows.** Worked out from
    the vendored gpui and not observed. With the platform's caption gone,
    gpui's frame is thicker at the top while maximised than while windowed
    (windows/events.rs:1509-1544), and gpui measures it again only at
    creation, on a DPI change and on a system settings change
    (windows/window.rs:471, events.rs:786 and 1116), never on a restore. So a
    change that lands while the window is maximised, such as the taskbar or a
    display moving the work area, leaves the maximised offset in use after
    the restore, and a close before the next re-measure saves the restore
    bounds about 7px shorter and 4px lower at 100% scale, more when scaled;
    each time it happens the window comes back a little smaller. The fix
    belongs in `vendor/gpui` — re-measure only while not maximised — and
    waits on a decision to patch it; see "The window remembers where it was".
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
    the channel, or the rail's recommendations do, or the ask about
    channels no list carries has answered for it (`RootView::poster`, from
    the preview template a live channel always has); one opened by name
    waits on black unless that answer, asked as it starts
    (`root::pane_streams`), comes back before its picture. The preview is a
    snapshot, minutes old at most.
26. **`Start when they go live` is signed in only, and slower for a
    channel you do not follow.** The follows poll starts a followed
    channel's pane within a minute of its stream; any other's is started by
    the ask about channels nobody follows (`root::pane_streams`), every two
    minutes, so up to two minutes late, and a pane on one offers the switch
    only once the list of everyone followed has come back after a sign-in,
    which is what says it is one.
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
    only on Windows (`pop_out::offered`, which says why): on macOS the
    window gpui would keep up is a panel that hides whenever another app is
    in front, which is the one thing a pop-out is for, and hover on it
    follows the window being active (window.rs:1727-1737); nobody has run it
    on Linux. A Mac has to show what it takes — most likely a `PERCH PATCH`
    clearing `hidesOnDeactivate`, listed in the root `Cargo.toml` and in
    `scripts/verify-vendor.sh` — before it is offered.
31. **A pop-out takes the keyboard when it opens, and has a taskbar
    entry.** Windows ignores `WindowOptions.focus`, so the new window is
    activated and the keys go with it; and it is an ordinary window, with a
    taskbar and `Alt+Tab` entry, because that is how the keyboard gets back
    to it. Making it a tool window would take both away.
32. **Panes maximized away still decode**, their rendition and their sound,
    so a maximize saves only what drawing them cost (their frames are
    rendered small and thrown away, `VideoStream::set_offstage`): the
    quality rule puts the picture you come back to first. They redraw no
    window, an open palette's included: it reads every player to know
    which can offer `Choose quality for …`, but one offstage wakes the UI
    only for a stop or a change of `buffering`. See "A pane given the
    window".
33. **The pane order, the maximize and where pop-outs open last for the
    session.** Nothing of them goes into `settings.json`: a new session
    opens what it is told in the order it is told, every pane in the main
    window, and the first pop-out in the corner again.
34. **Rewind depends on Helix knowing the broadcast and listing its
    archive.** The timeline needs `started_at`, which only Helix's live
    answers carry. A pane opened by name, from a link or from the rail's
    recommendations, which no fetched list carries, asks for its stream by
    itself as it starts (`root::pane_streams`), so its timeline comes up
    once that is answered, usually with its picture; signed out it has
    none. A channel that keeps no past broadcasts, or whose archive Helix
    has not listed yet — in a broadcast's first minutes, or at the first
    press after a restart — says so in a toast and stays live. A broadcast
    split by a reconnect rewinds only within its last part, and its first
    part never shows `LIVE`. The start comes from whichever list knows the
    channel first, so a Popular page fetched during an earlier broadcast
    this session can draw the timeline from that one's start. The way back
    leans on the same lists: on a channel only a snapshot list carries,
    `LIVE` stays until Twitch ends the archive's playlist, a press meanwhile
    opening a pane that says the channel is off, and on a recording of a
    channel no list carries it never shows unless the channel is pinned.
    See "Rewinding a live pane" under Recordings.
35. **Desktop notifications are Windows-only, and unproven on a desktop.**
    The registry AUMID, the toast and the protocol click were built from
    Microsoft's documentation and checked by compiling, not by watching one
    arrive; the first live run should confirm the toast shows under `perch`
    with its icon, and that a click from the banner and from the
    notification centre brings perch forward (the foreground right passes
    from the click to the second launch to the running window, the
    `give_way` path, which nothing here has exercised from a toast). A toast
    can come up to a poll — a minute — after the stream starts. macOS shows
    none: `notifications::show` is a no-op there, and its sheet field says
    so. A moved `perch.exe` leaves the scheme on the old path until the next
    toast.
36. **Some of the overhaul has been seen only by its tests.** As of 2
    October 2026 nobody had watched these work in the app. The user has
    used it daily since, so read them as unconfirmed rather than suspect,
    and look first here when one of them misbehaves:
    - the pop-out: dragging by the picture and resizing from an edge with a
      real mouse, the 256 × 144 floor, staying in front while another app
      is active, `M`, the arrows and `Ctrl+W` in it, whether its bar blinks
      as the pointer crosses between picture and bar, a pane paused, popped,
      played, paused again and brought back showing the second pause (the
      stale tile the pop-out trap guards), video memory over ten restarts
      while popped (see "Working on it") and the CPU log with one open
      (`perch.cpu.csv`, under Windows packaging), two or more stacking clear
      of each other, and its quality following its size, and the re-pick on
      bringing one back; its other entry points (the pane menu's row,
      More's, `P` from the browse page, which brings the watch page up with
      it);
    - phase 2's signed-in parts: an offline pane's last broadcast as a card
      or pill, `Start when they go live` starting a pane, `Watch from the
      start`, the live poster; and the failed screen when mpv cannot open;
    - the swap on a recording, and on a live pane a second live session on
      one token (item 3); and after a failed switch, its message with the
      bar's linger, and More's folded row mid-switch on a narrow pane;
    - the paused redraw after a resize, a pop-out, a bring-back and a
      maximize, and the redraw of any pane back from offstage, playing,
      paused, or in an ad break or a stall, and again at the size its
      window asks for after (the video trap says what to do if it draws
      black or stale);
    - from the checks before the push of 14 October 2026: an ad notice
      ending on the frames of a pane maximized away (a pre-roll that ends
      offstage) and holding through the grid's return mid-break, the frame
      clock having moved to the render thread; and a popped pane's badge,
      `buffering` through a stall and the ad break's tag through a break;
    - maximize and reorder end to end: no remount or flash either way, the
      re-pick on the way back, `Esc`'s order, a header dragged and dropped
      or let go elsewhere, and `Shift` with an arrow (the pane menu's
      `Maximize`, `Show all panes` and `Move later` were seen with two
      panes, signed out, on 15 October 2026);
    - the comfort, player and chat packs and what came with them: chat's
      text size and time on every message, Shared Chat tags, toasts holding
      still under the pointer, `Alt` + wheel, middle-click, `Only this
      one`, the ad-break notice, badges, reply lines, the rail's preview
      cards and right-click copy.
    - the round of 14 October 2026, signed-in parts: the mention wash on
      a real chat, recommended previews on the rail's cards and under a
      starting pane, an ended pane's one try while its broadcast is still
      listed, the sign-in toast for a failed save, the 2160p render on a 4K
      stream, and a render loop's own exit offering `Try again`.
    - pins from anywhere, signed in: More's `Pin to the rail` and
      `Unpin from the rail` on a live pane and a recording, the channel
      page's pin and its tooltip after a press, a pin nobody follows
      turning live in the rail within two minutes of its stream, a stopped
      pane nobody follows offering `Start when they go live` and starting
      by itself, an ended one's `Watch from the start` finding its own
      broadcast, and the one ask's cadence (a `pane streams:` line in the
      log is a failed ask; the ask itself says nothing).
    - the chat options menu on a chat box shorter than it
      (`chat_menu::placed`): at 1080 × 1900 with two stacked panes and the
      divider dragged to 0.8, on the top pane and the bottom one, risen over
      the header and the picture with every row in sight and taking its
      press, `Reset pane sizes` among them; checked only against taffy's
      layout in a throwaway crate.
    - the details at the top of chat (`header::details`), in three places
      not yet seen: the compact header's ⋯ on the band over a picture (the
      band comes up only under the real pointer), a recording's header, and
      the tooltips.
37. **What emotes stacked together leave out stays out, and stacks and
    effects are seen only by their tests.** A wide overlay on a narrow emote
    is fitted into the emote's box rather than spilling over the words beside
    it, so it is drawn smaller than 7TV's own extension or Chatterino draw
    it. FrankerFaceZ's `margins` and `offset` on a modifier, and its global
    sets given to named users, are ignored. Effects follow BetterTTV's
    stylesheet, not FrankerFaceZ's own drawing, so `ffzX ffzY` is upside
    down only, as BetterTTV draws it. Party and shake on an animated emote
    can make a picture of many times its frames, up to `effects::MADE_BYTES`,
    and a pane keeps 32 MiB of them (`chat_effects::BYTES`), so in a channel
    heavy with `p!` and `s!` on animated emotes, ones scrolled past are made
    again when they come back into view, their names standing in meanwhile;
    a loop that meets the effect's period only past that budget jumps by
    what is left over as it comes round (see "Effects are drawn as BetterTTV
    draws them" under Chat). A turn's box is a square at the emote height
    where BetterTTV's is the picture's own width, at most 28 px: the turned
    picture is the same either way, and a narrow emote's box a little wider.
    As of 14 October 2026 none of it had been seen on screen: the overlay's
    box is reasoned from gpui's source (an absolute `img` with a percentage
    width in a block wrapper), and a made picture's place from taffy's (a
    block wrapper given the box, the `img` placed in it by its margins), so
    the first look should measure them — the row pitch still 29 at Default,
    the emote 28 tall, the overlay the emote's box plus `EMOTE_PAD_X` a side,
    a wide emote 112 across, a turned or flipped one in its own box, a shaken
    one moving 2 px either way about where it rests, and `z!` 4 px closer to
    what is before it — at Larger as well, on a wrapped message, with the
    tooltip gone once the pointer leaves the window; and watch the log for
    `emote effects:` lines.
38. **Cheers are words where Twitch does not mark them, or Perch cannot
    ask.** In a recording's chat replay, whose comments say nothing of Bits;
    in a room first met signed out, when Helix cannot be asked (a room
    answered before a sign-out keeps its book for the session, as the badges
    do); and in a Shared Chat copy that uses a partner channel's own
    cheermote, which the room's book does not hold (the global ones draw).
    Entries Helix types `display_only` are skipped, as its reference asks;
    Chatterino keeps every type, and which prefixes Helix types so has not
    been seen. A channel's custom cheermote is cached by its URL for good, so
    one re-uploaded at the same address shows the old picture. As of 14
    October 2026 no cheer had been seen on screen: the first look, signed in,
    should check that a cheer animates at the emote height, that size `2`
    decodes at 56 px (only sharpness rests on it), that the amount reads on a
    striped row and a mention row and greys on a deleted one, that a two-line
    message with a cheer does not paint over its other line, and that the
    word's prefix stands in, dim, until the picture is on disk; that
    `Cheer100` stays a word in that channel's replay, and signed out in a
    copy that has not signed in since it started (a sign-out keeps the books
    already answered); which prefixes one Get Cheermotes answer types
    `display_only` (keep the type if people type one in chat); and watch the
    log for `chat cheers:` lines.
39. **The viewer card has been seen only by its tests.** As of 14 October
    2026 perch had not been launched with it. Its layout is reasoned from
    taffy (an absolute flex column with only `max_h`, holding a block well
    that shrinks), and the same-name toggle from gpui's listener order, read
    from source. The first look should check, in a 1600x920 window with two
    panes side by side and a 1080x1900 one with two and four stacked: a name
    near the top opens below and one near the foot above; in a 2x2 grid at
    1600x920 a name in the middle of a chat opens the 260-tall card on the
    chat's foot with its head and foot whole; at the narrowest chat the card
    is 248 wide and its pills wrap; a short chat's card stands on its foot
    and rises over the header and the picture, and with the pointer resting
    on the part that rose, the pane above is not made the active one, no
    player's bar comes up behind it, and the chat holds its rows (`Chat
    paused` once one arrives); with a card open, adding a
    third pane from the palette (`Ctrl+K`) shrinks the chat and the card
    stays inside it; while the account is asked for, the unseen account line
    keeps the room the answer's takes (one line or two); the badges row and
    the foot wrap at the card's width rather than one per line; the lines
    well shrinks and scrolls rather than collapsing to nothing or spilling
    out of the card (if it collapses, see "A view flowed into a flex item");
    the same name closes it, another moves it, `Esc` and the × close it, and
    a double-click on a name leaves it open; signed out, a card for someone
    not looked up this session has no picture or age and no gap for them,
    and `Channel page` is not offered; a lookup that
    failed (the network off) asks again when the card is opened again;
    `Channel page` asking `Stop streams and open?` while something plays,
    going on the second press and putting itself back after four seconds or
    a press elsewhere (not seen since it came back on 22 October 2026); and
    the log for `viewer card:` lines. Known edges: the card
    stays where it opened rather than following its row; a line that
    arrives while it is open lands at the foot of the well without scrolling
    to it; and an `@mention` of a word that is a login nobody has
    (`@everyone`) costs one ask and shows no account.
40. **Browsing that stops what plays has been seen in part.** It was seen on
    22 October 2026, on the user's sign-in in a scratch copy at 2560 × 1392.
    Not yet seen: `Esc` leaving fullscreen before the page (a test copy in
    fullscreen takes over a monitor the user is working on), Bring back and
    `P` from the browse page bringing the watch page up, `LIVE` pressed in a
    pop-out while the browse page is up, and the viewer card's second press
    (item 39).

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
  clicked: a tap on a card's corner — a touchscreen's, with no hover first —
  could forget a recording through a pill nobody saw. `motion::Fade` ends
  hidden as `invisible()`, which gpui does not paint and registers no
  listeners for, and the cards' pills are `invisible()` until their card is
  hovered.
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
  on it and nothing else, since `control_rest`, `text_dim` and the accent
  all fail there over a white frame
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
  state, as the bar's glyphs, the rail's pin and the gear do (see the GPUI
  traps).
- Do not take `.occlude()` off the title bar, give a caption button a handler,
  or wrap the bar's controls in its drag area. Each one hands the platform's
  press to gpui, which reports it handled, and the window stops dragging,
  maximising, resizing from the top or closing — silently. See the title-bar
  trap.
- Do not put a modal, a toast or anything else that floats outside the root's
  content wrapper, and do not give that wrapper an id. Under the bar is what
  keeps them off the drag strip and the caption buttons, and an id there would
  re-namespace every element id beneath it.
- Do not let a hush — `Only this one`'s — write a volume. The hush lives in
  `Loudness` beside the level the user chose, and only a chosen level is
  reported to be remembered; a hush saved as `Some(0)` opens that channel
  silent next time, which is the bug muting never becoming the default
  exists to prevent.
- Do not let the plain wheel change the volume over a picture. `Alt` was
  asked for by name, so a wheel turned on the way past a video never changes
  what it sounds like; without it the player lets the wheel go by.
- Do not cache an image whose URL is stable but whose content is not, and do not
  refresh one in place — GPUI decodes per path.
- Do not derive anything per-row from a row's index; the backlog drains from the
  front.
- Do not merge FrankerFaceZ's global answer whole. Its `default_sets` are
  everyone's; the other sets belong to the users named under `users`, and
  taking them all put its supporters' effects in every chat.
- Do not give an emote's overlay a listener, a tooltip or a cursor. It would
  get a hitbox and take the pointer from the emote under it, which answers
  for the whole stack (see "An emote can be drawn over the one before it").
  Nor an id any tooltipped element can have: chat's ids are the pane's, not
  the row's, and an element without a tooltip builder clears the tooltip of
  every element sharing its id.
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
- Do not size a block wrapper by the margins of a child with a width of its
  own. taffy takes that width and leaves the margins out
  (`determine_content_based_container_width`, compute/block.rs), so they only
  move the child inside a box as wide as it is; give the wrapper its width,
  as a made emote picture's is (see "Effects are drawn as BetterTTV draws
  them").
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
- Do not put a `flex_wrap` row directly inside a flex column. gpui sizes it
  from its own content, so it wraps at its narrowest and paints over whatever
  is beneath. Wrap it in a `flex_row` first; see `render_event`.
- Do not let a text element shrink below its width and wrap inside a flex item
  that taffy also measures whole. gpui's text measure returns its last size
  when asked with no wrap width, so the row comes out the wrong height; draw
  long words in pieces instead (`chat_text::pieces`).
- Do not assume `gpui-component` draws its own icons. It asks the *host* for
  `icons/<name>.svg` and ships none, so with no `AssetSource` every chevron,
  eye and clear button renders as nothing — and silently, since a missing asset
  is not an error anywhere in that path. The clickable ones are still there and
  still clickable, which is worse than absent. Perch's own controls draw from
  the same table, by `assets::Icon` rather than by path, and an `svg`
  still needs its own `text_color` to draw at all.
- Do not let `gpui_component::init` have the last word on the palette. It seeds
  itself from `cx.window_appearance()`, which is the *operating system's*
  light/dark setting, and nothing else in this app asks the OS anything. Call
  `widget_theme::apply` after it.
- Do not set a margin on the element you hand to `motion::arrive`. It animates
  `mt` and overwrites whatever is there, so the offset belongs on a wrapper.
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
- Do not count a pane's frames from its view's wakes for the ad notice
  (`VideoView::resumed_at`). A player maximized away sends no wakes for its
  frames, and a wake that says `buffering` began or ended, or a stop,
  carries none: counted there, the clock stood still offstage and had to
  skip wakes to stay honest. The render thread sees every frame and counts
  them (`ad_break::FrameClock`).
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
- Do not dock the browse page beside the panes again, nor keep what plays
  at home going behind it, without asking the user. It docked on the left,
  half the window, for a week of October 2026, and in use they found it
  weird: one press shrank the stream to a third of its area and moved its
  chat under it. Going to browse is looking for another stream, and they
  want the one on to end (see "Product decisions"). A pop-out plays on.
- Do not move a pane's details under its picture again. For a week of
  October 2026 a beside pane's header went into the black under its
  picture, the picture pinned to the top of its column; the user did not
  like the viewing page it made, and chose the details at the top of chat,
  the picture centred.
