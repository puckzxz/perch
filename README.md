# nativetwitch

Twitch in one native window: the stream, its chat, and the channels you follow.
No Electron, no second window for the player, no third one for chat.

Built on [GPUI](https://github.com/zed-industries/zed) (Zed's UI framework) with
[streamlink](https://streamlink.github.io/) as the Twitch byte source and
[libmpv](https://mpv.io/) doing decode and A/V sync.

## Running it

```
run.cmd                    reopen the last channel
run.cmd forsen             open a channel
run.cmd forsen --volume 30
```

`--volume` applies to this run only. It does not overwrite the level each
channel remembers, and it wins over one for as long as the app is open.

Or directly, once built:

```
cargo run --release -p nativetwitch -- forsen
```

Use `--release`. The video path does per-frame format conversion, and a debug
build is several times slower at it.

### Keyboard

Everything you reach for while watching is a hover-revealed overlay on the
video, which is no use when you are not holding the mouse.

| | |
|---|---|
| `Space` | Pause or resume |
| `M` | Mute or unmute |
| `↑` `↓` | Volume |
| `Ctrl+W` | Close this pane |
| `Esc` | Back to follows |
| `Ctrl+F` | Search |
| `Ctrl+R` | Refresh whichever list is on screen |
| `Ctrl+,` | Settings |

Player keys act on the pane you last pointed at. All of them stand aside while
the cursor is in a text box. The same list is in the settings sheet.

### Requirements

- **streamlink** on `PATH`, or `STREAMLINK_PATH` pointing at it.
- **libmpv** — `libmpv-2.dll` on Windows. There is no official Windows
  development package; the DLL ships inside player distributions such as
  mpv.net and Plex, and the app finds those automatically. `MPV_DLL` overrides
  the search.

## Chat

Read-only, over anonymous IRC — no account, no token. Messages carry their
emotes (Twitch, FFZ, BTTV and 7TV), links are clickable, `@mentions` are drawn
in the colour of whoever is being addressed, and subs, gifts, raids and
announcements appear as their own rows rather than being dropped.

A pane also opens with the last hundred messages from *before* you joined, so
four panes do not open blank. Twitch publishes no scrollback of its own, so
those come from the same community service
[Chatterino](https://chatterino.com/) uses — which means the request tells
someone other than Twitch which channels you watch. Settings has the switch,
including **Off**.

## Follows

Live channels first, as cards; everyone else you follow below as names. An
offline channel still opens — the video says so, but its chat connects either
way.

The list refreshes itself every minute. `Ctrl+R`, or the pill in the header,
asks again now — for whichever list is on screen, not just follows.

## Settings

The gear in the title bar. Stored at `%APPDATA%/nativetwitch/settings.json`;
changes apply immediately rather than needing a restart.

Volume is remembered per channel, because streamers are not consistent about
how loud they run. Muting one is remembered too, and deliberately never becomes
the default for a channel you have not opened before.

### The two Twitch tokens

They are unrelated credentials that do different jobs, which is worth stating
plainly because the names suggest otherwise.

| | What it is | What it does |
|---|---|---|
| **Client ID** | An application you register at [dev.twitch.tv](https://dev.twitch.tv/console) | Lists the channels you follow |
| **auth-token** | The `auth-token` **cookie** from twitch.tv | Prime/Turbo ad suppression and sub-only qualities |

Neither can do the other's job.

To create the Client ID: register an application, set **OAuth Redirect URL** to
`http://localhost` (required by the form, unused by this app) and **Client Type**
to **Public**. No client secret — sign-in uses the device code flow, so nothing
secret is ever stored in the binary. Paste the Client ID into settings and the
sidebar will show a code to enter at `twitch.tv/activate`.

The auth-token cookie is a **full account credential**. It is stored in plain
text, which is what desktop Twitch clients generally do, but it is a real
tradeoff rather than an oversight.

## Quality

`Auto` picks the stream that scales cleanly into the video pane, which is
usually cheaper than "best". Measured on a live 1080p60 stream, cost tracks the
*ratio* between source and pane more than the pixel count:

| source → pane | CPU (one core) |
|---|---|
| 1080p → 960×540 (exact half) | 35% |
| 1080p → 1920×1080 (1:1) | 79% |
| 1080p → 1280×720 (arbitrary) | **100%** |
| 720p → 1920×1080 (upscale) | 117–196% |

Note row three: an arbitrary downscale costs *more* than rendering at native
size, despite producing fewer pixels. So selection prefers 1:1, then exact
fractions, and never upscales while a larger source exists. Render size is also
clamped to the source resolution — mpv never scales up, the GPU stretches the
last bit instead, which is effectively free.

## Layout

```
crates/
  mpv-frames    libmpv loaded at runtime, software render to BGRA
  streamlink    supervises streamlink as a headless byte source
  twitch-chat   read-only chat over anonymous IRC
  twitch-api    device-code sign-in, follows, browsing and search
  emotes        Twitch/FFZ/BTTV/7TV resolution, disk image cache
  settings      persisted user settings
  nativetwitch  the app
```

Every crate except the last is free of UI types, so the pieces are testable
without a window.
