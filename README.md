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

**Windows and macOS.** Both are built and tested on every push and released
together; the Mac download is a universal `perch.app` covering Apple Silicon and
Intel. Linux is deliberately not claimed — the platform branches are there and
they compile, but nobody has watched it start up, and a platform claimed on
paper is worse than one left out.

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
run.cmd                    open on the follows page       (Windows)
run.cmd forsen             open a channel
run.cmd forsen xqc         open two, side by side
run.cmd forsen --volume 30
```

```
./run.sh                   open on the follows page       (macOS, Linux)
./run.sh forsen            open a channel
./run.sh forsen xqc        open two, side by side
./run.sh forsen --volume 30
```

Name up to four channels to open them together. A twitch.tv link works in
place of a name — to a channel, or to a recording, which opens where the link
points when it carries a `?t=`. `--volume` applies to this run only: it does
not overwrite the level each channel remembers, and it wins over one for as
long as the app is open.

Only one perch runs at a time. Launching it again while it is open brings the
open window to the front — out of the taskbar, if it was minimised — and
whatever you named opens there, beside what is playing. Two copies used to be
possible, and each spent the sign-in the other was relying on.

Or directly, once built:

```
cargo run --release -p perch -- forsen
```

Use `--release`. The video path does per-frame format conversion, and a debug
build is several times slower at it.

### Keyboard

The player's controls are on a bar that comes up over the video while the
pointer is on it, and those a key also works name the key in their tooltips;
the quality and More have no key. The rest — the rail button, back and
forward, search and settings — sit in the bar across the top of the window.
Neither is any use when you are not holding the mouse.

| | |
|---|---|
| `Space` | Pause or resume |
| `M` | Mute or unmute |
| `C` | Show or hide this pane's chat |
| `B` | Show or hide the follows rail |
| `↑ / ↓` | Volume |
| `← / →` | Skip ten seconds in a past broadcast |
| `1 – 4` | Talk to that pane |
| `Tab` | The next pane |
| `Ctrl+W` | Close this pane |
| `P` | Pop this pane out into a window of its own, on top of other apps, or bring it back (Windows) |
| `Esc` | Back to browsing, or back to watching |
| `Alt+←` | Back to where you were before — the mouse's back button too |
| `Alt+→` | Forward again — the mouse's forward button too |
| `Ctrl+F` | Search, from either page |
| `Ctrl+R` | Refresh whichever list is on screen |
| `Ctrl+,` | Settings |
| `Ctrl+K` | Command palette |
| `Ctrl+0` | Reset the pane sizes |
| `F / F11` | Fullscreen, and back |

On macOS every `Ctrl` on this page is `⌘` and every `Alt` is `⌥`. The `Ctrl`
bindings are declared on gpui's `secondary` modifier, which is cmd there and
ctrl everywhere else, so the two never drift apart. The settings sheet draws
whichever one this machine actually binds, and so does every tooltip that
names a key — a pane's × says `Close (⌘W)` on a Mac — and a test holds them
to that, and holds this table to the sheet, so a key the app lists is a key
listed here.

Player keys act on the pane you last pointed at, or last clicked — clicking
anywhere in a pane, video or chat, makes it the one the keyboard is talking to,
and with more than one pane open its header is underlined to say so. `1` to
`4` name a pane by its place in the grid and `Tab` steps along them, for when
the mouse is nowhere near. A pane with its chat hidden has its header over the
picture, so its underline shows while the pointer is on it, and for a second
and a half after `1` to `4` or `Tab` has chosen it or `C` has hidden its chat.
Pause, mute and volume never bring a header up over the picture. The keys
stand aside while the cursor is in a text box — all but `Ctrl+K`, `Ctrl+,` and
`Ctrl+R`, which type nothing, so the palette is one keystroke away straight
after a search. A search typed on the watch page leaves it for the results, the
way `Esc` does. The same list is in the settings sheet. Double-clicking the
video is fullscreen too. `Esc` from the watch page goes back to whichever tab, category
or channel you left the browse page on — and first closes a pane's open menu,
if there is one. A playing pane's quality menu is in the palette as well:
type `quality` and some of the pane's name, and `Choose quality for …` opens
it. So are the two things under the bar's **More**, as `Copy link to …` and
`Open … on twitch.tv`.

The bar over a playing pane has play, the speaker — crossed out whenever
the pane is silent, Mute all included — and the volume at the left, and at
the right the quality, chat, fullscreen and **More**, which has **Open on
twitch.tv** and **Copy link**. A narrow pane drops the volume's figure
first, then its slider, then folds the quality into More. A past broadcast
with no chat to replay shows the chat icon crossed out, with nothing to
press.

Back and forward work the way they do in a browser. `Alt+←` and `Alt+→`, the
arrows in the title bar or the mouse's side buttons walk back through the
tabs, categories, searches, channel pages and the watch page you have been
on, and forward again. `Esc` and the `← Back` beside a search or a channel's
name still step out of whatever has taken the page over, and back takes that
back too. The watch page drops out of the way once nothing is playing on it.
A list you go back to is asked for again if it has been replaced since, and
opens at the top. On Windows the side buttons do nothing over the title bar's
empty strip or its window buttons, which belong to Windows rather than to
the app.

Every pane has an × in its header, a lone one included, and like the bar's
icons it names its key, `Ctrl+W`; closing the last pane goes back to the
browse page.

A pane's header says `muted` or `paused` when either is true, so a channel
that opens silent says so without the pointer having to be on the video. With
chat hidden the header is over the picture, so they show there while the
pointer is on it — where the bar's speaker and play icon say the same.

The window opens where it was last closed — on the same monitor, at the size
it was — as long as that monitor is still there; the first time, it is sized to
fit the screen.

The seam between video and chat can be dragged, in either arrangement, and the
size is remembered. `Ctrl+0` puts both back to what the layout would have
derived.

Hiding chat is remembered per channel, the way volume is — a channel you watch
for the game stays that way without saying anything about the next one. Beside
the video, hiding chat gives the video its column, all of it: there is no
strip left above the picture. The header — the name, the numbers and the × —
comes over the top of the picture while the pointer is on it, on the same dark
band as the controls at the bottom, and stays up over a pane with no picture
to cover, one starting, offline or ended, so that pane still says whose it is
and can be closed; with no picture there is no bar either, so that header has
the chat icon that brings chat back. Under the video, the pane keeps its shape
when chat is hidden: the picture stays in its box, where every neighbour's is,
and does not move when `C` brings chat back. The space chat had says
`Chat hidden · press C`. A past broadcast with no chat to replay lays out the
same way.

Each pane's header says who is on, how many are watching, how long they have
been going, and what they are doing — the stream's title and its game, on a line
of their own. Both are cut to fit; rest the pointer on them for the whole thing.

While a pane starts it shows the channel's latest picture — the one its card on
the browse page shows, when a list there has the channel — dimmed under its
name, and the stream fades in over it when it arrives. A channel that is off
says so and offers what there is instead: its last broadcast, as a card where
there is room, which plays right there in place of the live pane; **Start when
they go live**, a switch that is on unless you turn it off, for a channel you
follow; and **Try again**. When a stream ends the pane says so — it does not
simply stop on its last frame, which is indistinguishable from a pause — and
offers **Watch from the start**, which plays that broadcast's recording from
the top in the same pane once Twitch lists it, **Try again** and **Close**. The
last broadcast and the recording need a sign-in to be found. Chat stays
connected either way, which is where the goodnights are, until a recording
takes the pane's place and brings its own chat replay.

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
announcements appear as their own rows rather than being dropped. A past
broadcast gets the same pane, replaying what was said as the picture reaches
it — see [Past broadcasts](#past-broadcasts).

The time is shown once a minute, as a break between messages, rather than once
per row — in a channel where fifteen messages share a minute, a column of
identical timestamps is not a ruler. It is written the way your system writes a
time, twelve-hour or twenty-four, in your language. The channel's name in the pane header opens
it on twitch.tv, which is the way out of a chat you cannot type in — and so
do **Open on twitch.tv** under the bar's **More** and the palette's `Open …
on twitch.tv`. On a live pane the site plays the stream as well, with its
own sound.

A pane also opens with the last hundred messages from *before* you joined, so
four panes do not open blank. Twitch publishes no scrollback of its own, so
those come from the same community service
[Chatterino](https://chatterino.com/) uses — which means the request tells
someone other than Twitch which channels you watch. Settings has the switch,
including **Off**.

Chat holds still while the pointer is over it. A link that moves as you reach
for it is not much of a link, so while a pane is pointed at and following live
its new messages wait — it says `Chat paused` — and land the moment the pointer
leaves. A pane you have scrolled back in is left alone; its position is already
yours. A message a moderator deletes stays where it was, greyed and marked
`deleted`, rather than vanishing from under your eye.

Popular and the categories arrive a hundred at a time, which is Twitch's cap
per request rather than a choice. **Load more** at the end of the list fetches
the next hundred — a page you asked for, rather than a list that grows while you
scroll past it.

## Follows

Live channels first, as cards; everyone else you follow below as names. A name
opens the channel's page — its past broadcasts, and a control for its chat,
which connects whether or not anyone is streaming.

The list refreshes itself every minute. `Ctrl+R`, or Refresh beside the tabs,
asks again now — for whichever list is on screen, not just follows.

Who is live is sorted by viewers, and holds still while the pointer is on it,
the way chat does: a refresh then updates the numbers where they stand, drops
whoever ended and adds whoever started at the end, and the list is put back in
order once the pointer leaves — so a card is never swapped for its neighbour
between aiming at it and clicking. The rail does the same. So do the offline
names, on the tab and in the rail: somebody whose stream ends joins the end of
them until the pointer leaves, rather than landing in the middle and pushing
every name after them down.

When somebody you follow goes live, a notice says so in the corner; click it
to watch them, or `+ Add` beside it to open them next to what is playing. A
pane left on a channel that was off, or whose broadcast ended, starts by
itself when a later poll finds them on again — open a channel before they
start and it begins without you. That is the pane's **Start when they go
live** switch, on unless you turn it off, and it is there only for channels
you follow, since the poll that notices is the follows poll.

The box at the top of the Following tab filters both lists as you type, live
and offline, by the same few-letters-of-a-name match the palette uses. It is
the opposite of the search box in the title bar: that one asks Twitch, this one
asks the app, and nothing typed here leaves it — until nobody you follow
matches, when it offers to ask Twitch instead.

Cards are as wide as the window allows: the grid takes the room it has and
divides it, rather than leaving whatever a fixed width could not use as a gutter
down one side. Viewer count and uptime sit on the thumbnail; the name, title and
game are underneath, each on one line and cut with an ellipsis where it does not
fit. Resting the pointer on one shows the whole of it.

Whatever is playing while you browse keeps playing, with its sound, in a small
player in the bottom-right corner of the page, clear of the list's scrollbar:
one picture, or up to four two to a row.
Click a picture to go back to watching it; the `×` that appears on a picture
under the pointer closes just that stream. The bar under the pictures says what
is playing and has three controls for all of it — Mute all (or Unmute all),
Back to watching and Stop all. Mute all silences the streams without touching
anyone's volume: it is never saved, it lasts until you change a stream's volume
yourself, and Unmute all leaves a stream you had muted muted. Every list leaves
room at its foot, so nothing is stuck under the player. Settings can turn it
off, in which case leaving the watch page stops the streams instead, which is
the cheaper answer if you go to the follows page to pick the next thing rather
than to glance at the list.

### The rail

Down the left-hand edge of both pages, in four groups. First the channels you
have pinned, in the order you pinned them, live or not. Then who else is live:
avatar, name, what they are playing, and how many people are there. Then
Recommended: up to five live channels you do not follow that are like the ones
you watch, each saying which one led to it — "Like forsen" — and holding still
under the pointer as the live list does. Then everyone else you follow, folded
under Offline and a count — click the count to unfold them, and again to fold
them away; they start folded each time Perch starts. Click a live channel,
followed or recommended, to watch it, or `+` to open it beside what is already
playing; click anyone else for their channel's page. Offline names get their
pictures too, asked for once each time Perch starts.

Recommendations start from the channels open in your panes and the ones you
watched most recently, six at most. For each, Twitch says who else its viewers
are watching — the suggestions its own website shows in its sidebar, from a
query Twitch has never published, which Perch asks without sending your
sign-in. Anyone you follow or are already watching is left out, though one you
have just opened from the list keeps its place, marked as watching, until the
pointer leaves the rail. They are asked
for when you open a channel they have not been asked about yet, and otherwise
every five minutes at most, only while you are signed in and the rail is open.
If Twitch refuses that query — most likely because it has changed it — the
group goes away until Perch is next started; if a request fails any other way,
the last list stays and is asked for again at the next five-minute refresh.
Nothing about them is saved.

The pin that appears on a row under the pointer pins that channel, and the
same pin on a pinned row unpins it. A recommended row has no pin, since a pin
is for a channel you follow. Pins are kept in `settings.json`, as `pinned`, so
they can be written there by hand too; a pin for a channel you no longer
follow shows as just its login, with nothing to say whether it is live.

It folds away with the rail button at the left of the title bar, or `B`, and
stays folded — a window left on one stream for three hours should be able to
be just the stream. Fullscreen hides it along with the title bar, folded or
not, and `B` does nothing there.

On the left, opposite chat. Chat belongs to the pane it is part of and sits on
the right of it; the rail belongs to the window.

### The palette

`Ctrl+K` — a channel to open, a pane to close, a page to go to, typed rather
than aimed at. It filters what the app already knows, so it costs nothing and
runs on every keystroke; the search box in the title bar is the one that asks
Twitch. `qb` finds QuickyBaby. Offline follows are in it too, once you have
typed something, and open the channel's page, as their names do on the
Following tab; a live channel's past broadcasts are a row of their own. A
playing pane's quality menu is a row too, once you have typed something:
`Choose quality for …` brings the pane's controls up with the menu open. Every
tab is a `Go to` row, and going back to watching or stopping everything is
offered once something is playing. `Ctrl+K` does nothing while the settings
sheet is open; close the sheet first.

With nothing typed it leads with the channels you watched most recently, then
who is live. Type a name none of your follows answer to and it offers to open
that channel anyway, with its past broadcasts beside it; paste a twitch.tv link
and it opens the channel or the recording the link names, from the moment a
`?t=` points at.

The recordings you have watched are in it too: type a few letters of the
channel, or a word of the title, to carry on with one. When the last thing you
opened was a recording you had not finished, it leads the empty palette, so
`Ctrl+K` then `Enter` picks it up where you left it.

## Past broadcasts

Every channel has a page of what it broadcast before: click an offline name on
the Following tab, the **Past broadcasts** control that appears on a live card,
or the palette's row for it — or search for it by name, since a search lists
the channels that answer to it and are not on, as names under the live ones.
The recordings are cards like the streams are, with the length where a
stream's card has its viewer count, and how long ago it was underneath. One
still being recorded carries the live dot and says how long it has been going
so far; it can be watched from the start while the stream is on.

A switch at the top of the page turns it to the channel's **Highlights** or its
**Uploads**, each a list of its own. They play the way a broadcast does, but as
picture alone: a highlight is cut from pieces of a broadcast and an upload was
never one, so neither has a chat to replay.

A recording plays in an ordinary pane, with a seek bar above the controls. Point
at the bar and it says what time is under the pointer before you click; click
it or drag its thumb, and `←` and `→` skip ten seconds. A jump takes a second
or so, because the player is reopened at the new place rather than seeked —
the reason is the player's and `HANDOFF.md` has it. The header says `replay`
— or `highlight`, or `upload` — names the video, and opens it on twitch.tv;
while it opens the pane shows the recording's own picture, dimmed, and when
the recording ends the pane says so and offers to start over. Under the
bar's **More**, **Copy link** reads `Copy link at 1:02:03` and copies a link
to the moment you are at, which pasted back into Perch opens there, and
**Open on twitch.tv** opens it at that moment. A channel's
live stream and one of its recordings can be open side by side.

Every recording you open is kept on the **History** tab, newest first: the ones
you are part-way through under *Continue watching*, each saying where you left
it, then the ones you finished. Opening a recording again — from there, from its
channel's page, or from the palette — picks up where you left it, and the pane
says so while it opens; one watched to within a minute of its end starts from
the top, and a link with a `?t=` goes where the link says. Cards on a channel's
page carry the same bar along the bottom of the picture, so you can see which
ones you have seen; on the History tab a highlight or an upload says which it is
under its title. **Forget** on a card's picture takes one off; **Clear
history** takes the lot. Either says so in a toast with an **Undo**, which puts
them back where they were, places and all. The list is `history.json` beside
the settings, and deleting it forgets what was watched and nothing else.

Resuming a recording left paused for half a minute or more takes a second: it is
reopened where it was, because the connection it was using may have gone while
it waited — and a dead one used to leave the picture stuck until you jumped away
and back. One that stops moving for twenty seconds while playing is reopened the
same way.

The chat plays back beside it: what was being said at the moment on screen,
from Twitch's own copy of it, with the same emotes, links and colours as a live
pane and the time breaks reading the broadcast's own clock. Jump somewhere else
and the pane swaps to the conversation around the new moment, starting a little
before it, so you land mid-chat rather than in a blank. `C` hides it, and that
is remembered per channel the way it is for a live pane. A broadcast still being
recorded has its replay too, running about thirty seconds behind live.

Twitch's picture for a recording comes at one small size, so it is soft on a
wide card. And without the auth-token cookie Twitch caps recordings at 1080p,
as it does for anyone not signed in on the website; with it, whatever was
broadcast.

## Settings

The gear in the title bar, on either page, or `Ctrl+,`. Perch draws that bar
itself: on Windows the gear sits just left of the minimise, maximise and close
buttons, which Perch draws too, while macOS keeps its traffic lights. Resting
the pointer on the gear says who is signed in; until somebody is, the bar says
what the sign-in is waiting for beside it. The bar goes away in fullscreen with
everything else. Stored at
`%APPDATA%/perch/settings.json` on Windows and
`~/Library/Application Support/perch/settings.json` on macOS; changes apply
immediately rather than needing a restart. Saving the sheet changes only what
the sheet shows, so anything else that changed while it was up — a channel a
second launch opened, say — stays changed. What you have watched is beside it,
in `history.json`.

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
title bar will show a code to enter at `twitch.tv/activate`, on either page, until
the sign-in lands; the Following tab shows it too.

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

The choice is made again whenever a pane changes size — another pane opening
or closing, the rail folding, the window resizing or going fullscreen — and
only ever upwards: a pane that has grown moves to a sharper rendition, and a
pane that has shrunk keeps what it has. A quality picked from a pane's own
menu is left alone; that choice was about the pane, whatever its size. The
menu's first row is the settings' own choice, in the settings' words for it —
`Auto (matches the video pane)`, `Best available`, or whatever quality the
settings name — and picking it hands the pane back, changing what plays only
if that picks something else. A press anywhere else, or `Esc`, closes the
menu. With the pointer elsewhere, the palette's `Choose quality for …` opens
it.

Changing rendition keeps the picture. Streamlink cannot switch mid-stream, so
the new rendition is started beside the one playing, silently, and takes over
in place once it has a picture: at once on a live stream, and on a recording
once it has caught up with where you are, so the seek bar and the chat replay
carry on without a jump. Pause and volume carry over. For those few seconds
the pane is fetching and decoding two streams. A quality change in the
settings still starts every pane over, each saying it is starting until its
new picture arrives.

## Layout

```
crates/
  mpv-frames    libmpv loaded at runtime, software render to BGRA
  streamlink    supervises streamlink as a headless byte source
  twitch-chat   read-only chat over anonymous IRC; a recording's chat replayed
  twitch-api    device-code sign-in, follows, browsing and search
  emotes        Twitch/FFZ/BTTV/7TV resolution, disk image cache
  settings      persisted user settings, and what has been watched
  perch         the app
```

Every crate except the last is free of UI types, so the pieces are testable
without a window.

## License

MIT — see [LICENSE](LICENSE).
