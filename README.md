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

<img width="1661" height="901" alt="image" src="https://github.com/user-attachments/assets/0d776907-df7e-4700-9a4e-508e552922c9" />


## Getting it

Built binaries are on the [releases
page](https://github.com/puckzxz/perch/releases): a zip per platform, no
installer, nothing written outside your own user folder. Windows gets the
executable on its own; macOS gets a universal `perch.app` that runs on both
Apple Silicon and Intel. `RUNNING.txt` inside each covers the things it cannot
ship — streamlink, libmpv, and the Twitch Client ID you register yourself. The
app says which is missing when you hit it, but reading that file first is
quicker.

The Mac build is signed only ad-hoc, not with a paid Apple Developer
certificate, so macOS quarantines it on download and claims it is damaged. It
is not; `xattr -d com.apple.quarantine perch.app` clears it, and the Mac
`RUNNING.txt` says so in more detail.

Or build it, which is the rest of this page.

## Running it

```
run.cmd                    reopen the last channel        (Windows)
run.cmd forsen             open a channel
run.cmd forsen xqc         open two, side by side
run.cmd forsen --volume 30
```

```
./run.sh                   reopen the last channel        (macOS, Linux)
./run.sh forsen            open a channel
./run.sh forsen xqc        open two, side by side
./run.sh forsen --volume 30
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
| `C` | Show or hide this pane's chat |
| `B` | Show or hide the follows rail |
| `↑` `↓` | Volume |
| `Ctrl+W` | Close this pane |
| `Esc` | Back to follows |
| `Ctrl+F` | Search |
| `Ctrl+R` | Refresh whichever list is on screen |
| `Ctrl+,` | Settings |
| `Ctrl+K` | Command palette |
| `Ctrl+0` | Reset the pane sizes |

On macOS every `Ctrl` above is `⌘` — the bindings are declared on gpui's
`secondary` modifier, which is cmd there and ctrl everywhere else, so the two
never drift apart. The settings sheet draws whichever one this machine actually
binds, and a test holds it to that.

Player keys act on the pane you last pointed at, or last clicked — clicking
anywhere in a pane, video or chat, makes it the one the keyboard is talking to,
and with more than one pane open its header is underlined to say so. All of them
stand aside while the cursor is in a text box. The same list is in the settings
sheet.

The seam between video and chat can be dragged, in either arrangement, and the
size is remembered. `Ctrl+0` puts both back to what the layout would have
derived.

The seam between video and chat can be dragged, in either arrangement, and the
size is remembered. `Ctrl+0` puts both back to what the layout would have
derived.

Hiding chat is remembered per channel, the way volume is — a channel you watch
for the game stays that way without saying anything about the next one. The
header keeps its place above the video, so a pane without chat still has its
name and its close button.

### Requirements

- **streamlink** on `PATH`, or `STREAMLINK_PATH` pointing at it.
- **libmpv**, which is a different errand on each platform:

  On **macOS**, `brew install mpv streamlink` is both requirements at once —
  unlike Windows, there is a real libmpv package. The app looks in
  `/opt/homebrew/lib`, `/usr/local/lib` and `/opt/local/lib`, covering Homebrew
  on either architecture and MacPorts.

  On **Windows**, `libmpv-2.dll`. There is no official development package; the
  DLL ships inside player distributions such as mpv.net and Plex, and the app
  looks there, beside its own executable, and along `PATH`.

  `MPV_DLL` overrides the search on both.

  Every one of those is an explicit directory, for a reason that is the same
  shape on each platform and points the opposite way. Handing Windows a bare
  `libmpv-2.dll` would let it search the *working* directory too, which for an
  executable run out of a shared downloads folder is somebody else's choice of
  DLL. Handing macOS a bare `libmpv.2.dylib` has the reverse problem: dyld's
  fallback search is `/usr/local/lib` then `/usr/lib` and nothing else, so the
  Homebrew install that every Mac user actually has would never be found.
  `DYLD_FALLBACK_LIBRARY_PATH` is not a way round it either — SIP strips every
  `DYLD_*` variable from a protected process.

## Chat

Read-only, over anonymous IRC — no account, no token. Messages carry their
emotes (Twitch, FFZ, BTTV and 7TV), links are clickable, `@mentions` are drawn
in the colour of whoever is being addressed, and subs, gifts, raids and
announcements appear as their own rows rather than being dropped.

The time is shown once a minute, as a break between messages, rather than once
per row — in a channel where fifteen messages share a minute, a column of
identical timestamps is not a ruler. The channel's name in the pane header opens
it on twitch.tv, which is the way out of a chat you cannot type in.

A pane also opens with the last hundred messages from *before* you joined, so
four panes do not open blank. Twitch publishes no scrollback of its own, so
those come from the same community service
[Chatterino](https://chatterino.com/) uses — which means the request tells
someone other than Twitch which channels you watch. Settings has the switch,
including **Off**.

Popular and the categories arrive a hundred at a time, which is Twitch's cap
per request rather than a choice. **Load more** at the end of the list fetches
the next hundred — a page you asked for, rather than a list that grows while you
scroll past it.

## Follows

Live channels first, as cards; everyone else you follow below as names. An
offline channel still opens — the video says so, but its chat connects either
way.

The list refreshes itself every minute. `Ctrl+R`, or the pill in the header,
asks again now — for whichever list is on screen, not just follows.

Cards are as wide as the window allows: the grid takes the room it has and
divides it, rather than leaving whatever a fixed width could not use as a gutter
down one side. Viewer count and uptime sit on the thumbnail; the name, title and
game are underneath.

Whatever is playing while you browse keeps playing, muted, in a bar along the
bottom — each stream with its own close button, and one control back to
watching. Settings can turn that off, in which case leaving the watch page
stops the streams instead, which is the cheaper answer if you go to the follows
page to pick the next thing rather than to glance at the list.

### The rail

Who is live, down the left-hand edge of both pages: avatar, name, what they are
playing, and how many people are there. Click to watch, or `+` to open beside
what is already playing. It folds away with the arrow in its header and stays
folded — a window left on one stream for three hours should be able to be just
the stream.

On the left, opposite chat. Chat belongs to the pane it is part of and sits on
the right of it; the rail belongs to the window.

### The palette

`Ctrl+K` — a channel to open, a pane to close, a page to go to, typed rather
than aimed at. It filters what the app already knows, so it costs nothing and
runs on every keystroke; the search box in the header is the one that asks
Twitch. `qb` finds QuickyBaby. Settings can turn that off, in which case leaving the watch page
stops the streams instead, which is the cheaper answer if you go to the follows
page to pick the next thing rather than to glance at the list.

### The rail

Who is live, down the left-hand edge of both pages: avatar, name, what they are
playing, and how many people are there. Click to watch, or `+` to open beside
what is already playing. It folds away with the arrow in its header and stays
folded — a window left on one stream for three hours should be able to be just
the stream.

On the left, opposite chat. Chat belongs to the pane it is part of and sits on
the right of it; the rail belongs to the window.

### The palette

`Ctrl+K` — a channel to open, a pane to close, a page to go to, typed rather
than aimed at. It filters what the app already knows, so it costs nothing and
runs on every keystroke; the search box in the header is the one that asks
Twitch. `qb` finds QuickyBaby.

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
