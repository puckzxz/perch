# Perch

[![CI](https://github.com/puckzxz/perch/actions/workflows/ci.yml/badge.svg)](https://github.com/puckzxz/perch/actions/workflows/ci.yml)

Twitch in one native window: the stream, its chat, and the channels you follow.
No Electron, no second window for the player, no third one for chat.

Up to **four channels at once**, side by side in a grid derived from the shape
of your window, each with its own chat and its own volume. Chat is read-only by
design — this is somewhere to watch from, not another client to talk in.

Built on [GPUI](https://github.com/zed-industries/zed) (Zed's UI framework) with
[streamlink](https://streamlink.github.io/) as the Twitch byte source and
[libmpv](https://mpv.io/) doing decode and A/V sync.

**Windows only** for now. The video path is portable, but the app crate is not:
`diagnostics.rs` redirects the process's stderr through a Win32 call, because a
`windows_subsystem = "windows"` binary has no console to print to.

A personal project, published because a working one is more interesting than a
tidy one. Not affiliated with, endorsed by, or connected to Twitch Interactive,
Inc.

<img width="1591" height="918" alt="Four Twitch streams playing side by side in one window, each with its own chat pane" src="https://github.com/user-attachments/assets/421cff6e-591f-453d-895e-46415f893517" />

## Running it

```
run.cmd                    reopen the last channel
run.cmd forsen             open a channel
run.cmd forsen xqc         open two, side by side
run.cmd forsen --volume 30
```

Name up to four channels to open them together. `--volume` applies to this run
only: it does not overwrite the level each channel remembers, and it wins over
one for as long as the app is open.

Or directly, once built:

```
cargo run --release -p perch -- forsen
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
- **libmpv** — `libmpv-2.dll`. There is no official Windows development
  package; the DLL ships inside player distributions such as mpv.net and Plex,
  and the app looks there, beside its own executable, and along `PATH`.
  `MPV_DLL` overrides the search.

  Every one of those is an explicit directory. Handing Windows a bare
  `libmpv-2.dll` would let it search the *working* directory too, which for an
  executable run out of a shared downloads folder is somebody else's choice of
  DLL.

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

The gear in the title bar. Stored at `%APPDATA%/perch/settings.json`; changes
apply immediately rather than needing a restart.

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

The auth-token cookie is a **full account credential**, and it is worth knowing
exactly where it goes before you paste one in. It is stored in plain text, which
is what desktop Twitch clients generally do, and it is passed to streamlink as a
command-line argument — where, on Windows, any process running as you can read
it, and where command-line auditing will log it if your machine has that turned
on. Reading the file needs the same access, so this is one exposure rather than
two, but a command line is the kind that leaves the machine.

Both are real tradeoffs rather than oversights, and the feature is entirely
optional: everything except ad suppression and subscriber-only qualities works
without it.

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
  perch         the app
```

Every crate except the last is free of UI types, so the pieces are testable
without a window.

## License

MIT — see [LICENSE](LICENSE).
