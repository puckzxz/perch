# Perch

[![CI](https://github.com/puckzxz/perch/actions/workflows/ci.yml/badge.svg)](https://github.com/puckzxz/perch/actions/workflows/ci.yml)

Twitch in one native window: the stream, its chat, and the channels you follow.
No Electron, no second window for the player unless you pop one out, and no
third one for chat.

Up to **four channels at once**, side by side in a grid derived from the shape
of your window, each with its own chat and its own volume. Chat is mostly for
reading — this is somewhere to watch from — but each live chat has a box at its
foot to say something in once you are signed in.
Drag the panes into another order, give one the whole window while the others
play on, or, on Windows, pop one out into a small window that stays on top of
whatever else you are doing.

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
run.cmd                    open on Home                   (Windows)
run.cmd forsen             open a channel
run.cmd forsen xqc         open two, side by side
run.cmd forsen --volume 30
```

```
./run.sh                   open on Home                   (macOS, Linux)
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
the quality, the guide and More have no key. The rest — the rail button,
Home, back and forward, search and settings — sit in the bar across the top
of the window. Neither is any use when you are not holding the mouse.

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
| `Shift+←` | Move this pane one place earlier, swapping it with the one before |
| `Shift+→` | Move this pane one place later, swapping it with the one after |
| `Z` | Give this pane the whole window, chat and all, or show every pane again |
| `Ctrl+W` | Close this pane |
| `P` | Pop this pane out into a window of its own, on top of other apps, or bring it back; on the browse page, every pane in the mini player (Windows) |
| `Esc` | Back to browsing, or back to watching — after showing every pane again, if one has the window |
| `Alt+←` | Back to where you were before — the mouse's back button too |
| `Alt+→` | Forward again — the mouse's forward button too |
| `Ctrl+F` | Search, from either page |
| `Ctrl+R` | Refresh whichever list is on screen |
| `Ctrl+,` | Settings |
| `Ctrl+K` | Command palette |
| `Ctrl+0` | Reset the pane sizes |
| `F / F11` | Fullscreen, and back |

On macOS every `Ctrl` on this page is `⌘`, every `Alt` is `⌥` and every
`Shift` is `⇧`. The `Ctrl` bindings are declared on gpui's `secondary`
modifier, which is cmd there and ctrl everywhere else, so the two never
drift apart. The settings sheet draws
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
and a half after `1` to `4` or `Tab` has chosen it, `C` has hidden its chat,
or `Shift+←`, `Shift+→` or a drag of its header has moved it.
Pause, mute and volume never bring a header up over the picture. The keys
stand aside while the cursor is in a text box — all but `Ctrl+K`, `Ctrl+,` and
`Ctrl+R`, which type nothing, so the palette is one keystroke away straight
after a search. A search typed on the watch page leaves it for the results, the
way `Esc` does. The same list is in the settings sheet. Double-clicking the
video is fullscreen too. Holding `Alt` while turning the mouse wheel over a
picture changes that stream's volume, a step of 5% per click of the wheel, as
`↑` and `↓` do — the wheel alone never touches the sound, so scrolling past a
video cannot change it — and a middle click on a picture mutes or unmutes it,
as `M` does. Both work in a pop-out too. `Esc` from the watch page goes back to
whichever tab, category or channel you left the browse page on — and first lets
go of a pane being dragged, then closes a pane's open menu, if there is one,
then the guide, if it is up, and then shows every pane again, if one has the
window. A playing pane's quality menu is in the palette as well:
type `quality` and some of the pane's name, and `Choose quality for …` opens
it. So are the two things under the bar's **More**, as `Copy link to …` and
`Open … on twitch.tv`, and the maximize, as `Maximize …` and `Show all
panes`.

The bar over a playing pane has play, the speaker — crossed out whenever the
pane is silent, Mute all included — and the volume at the left, and at the
right the quality, with two panes or more the maximize, then chat, the guide
(see below), fullscreen and **More**, which has **Only this one** with two
panes or more, **Playback speed** on a recording, **Pop out** on Windows,
**Open on twitch.tv** and **Copy link**. **Only this one** silences every other pane the way Mute all does,
without touching anyone's volume; until you change a silenced pane's volume,
every pane's More then says **Hear all again**, which brings them all back. A
narrow pane drops the volume's figure first, then its slider, then folds the
quality into More, and the maximize after it. A past broadcast with no chat to
replay shows the chat icon crossed out, with nothing to press.

Back and forward work the way they do in a browser. `Alt+←` and `Alt+→`, the
arrows in the title bar or the mouse's side buttons walk back through the
tabs, categories, searches, channel pages and the watch page you have been
on, and forward again. The house before the arrows goes straight to Home from
anywhere, the watch page included, which it leaves the way `Esc` does; back
returns from it. `Esc` and the `← Back` beside a search or a channel's
name still step out of whatever has taken the page over, and back takes that
back too. The watch page drops out of the way once nothing is playing on it.
A list you go back to is asked for again if it has been replaced since, and
opens at the top. On Windows the side buttons do nothing over the title bar's
empty strip or its window buttons, which belong to Windows rather than to
the app.

Every pane has an × in its header, a lone one included, and like the bar's
icons it names its key, `Ctrl+W`; closing the last pane goes back to the
browse page. On Windows a playing pane's header also has the icon that pops
it out into a window of its own, and brings it back, `P` (see below). A
header sitting on chat also has the chat options (see [Chat](#chat)).

A pane's header says `muted` or `paused` when either is true, so a channel
that opens silent says so without the pointer having to be on the video. With
chat hidden the header is over the picture, so they show there while the
pointer is on it — where the bar's speaker and play icon say the same.

While Twitch plays an ad, streamlink leaves it out and the picture holds
still until the stream comes back. The header says `ad break · 0:25`
meanwhile, counting down when streamlink says how long it is, or just `ad
break` when it does not, until the picture moves again. With chat hidden,
the header over the picture comes up for a moment when the break begins, and
after that shows while the pointer is on the pane. If streamlink says
nothing, nothing is shown.

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
A live pane has a red `LIVE` badge after its name, so a glance down the page
says which panes are live; a recording says `replay` there instead.

While a pane starts it shows the channel's latest picture — the one its card on
the browse page shows, when a list there has the channel — dimmed under its
name, and the stream fades in over it when it arrives. A channel that is off
says so and offers what there is instead: its last broadcast, as a card where
there is room, which plays right there in place of the live pane; **Start when
they go live**, a switch that is on unless you turn it off, for a channel you
follow; and **Try again**. Under those it says when the channel is on next, if
it has a schedule on Twitch — *Next stream: Tue 7:00 PM · its title · its
game*, in your clock's format — or that it is *On a break until 12 Oct*. When
a stream ends the pane says so — it does not simply stop on its last frame,
which is indistinguishable from a pause — and offers **Watch from the start**,
which plays that broadcast's recording from the top in the same pane once
Twitch lists it, **Try again** and **Close**. The last broadcast, the
recording and the schedule need a sign-in to be found. Chat stays connected
either way, which is where the goodnights are, until a recording takes the
pane's place and brings its own chat replay.

### The guide: more to watch, without leaving

The icon beside chat on a pane's bar — a frame with a panel across its lower
half — raises the guide over the lower part of the watch page, while every
pane goes on playing above and behind it, with its sound. A pane with no
picture, one whose stream is off or has ended, has no bar, so its header
offers the same icon instead. It is never more
than half the page tall, and on a tall window only as tall as two rows of
cards. Across its top are four tabs: **Following**, who you follow that is
live; **Recommended**, the rail's Recommended group as cards, each saying
which channel led to it; **Popular**; and **Categories**, where picking one
shows its streams inside the guide, with **← Categories** to go back. Popular
and Categories are the same lists as the browse page's tabs, with the same
**Load more**, so whatever either has fetched the other already has. A
recommended channel shows a picture only once one of those lists has it.

Rest the pointer on a card for its two offers. **Watch**, or a click anywhere
on the card, plays that channel in place of the pane you opened the guide
from, keeping its place in the grid, however the pointer crosses the other
panes on its way to the card; a channel already open in another pane is
chosen there instead. Another pane's guide icon, pressed while the guide is
up, makes that pane the one Watch replaces. **+ Add** opens it beside the
others, and at four panes says `Already watching 4 streams` and leaves the
guide up. Otherwise the guide goes away once something plays; so does the ×
at its top-right, the guide icon of the pane it was opened from, `Esc`, a
click anywhere outside it, or leaving the watch page. It opens again on the
tab, and inside the category, you left it on; pressing **Categories** again
goes back from a category to the list. A narrow window that cannot fit every
tab scrolls them sideways under the mouse wheel.

While it is up the guide takes the pointer over its own area and nowhere
else: the panes above it are pointed at and clicked as ever, and the ones
under it stay as if the pointer were not there — their bars stay down, their
chat keeps moving, and passing over them does not make them the pane the
keys talk to. Opening it closes any menu open on a pane. Recommended fills in
with the rail folded away too, while that tab is showing.

### Going back in a live stream

Above the bar on a live pane is a timeline of the broadcast so far, from
when it started up to now, with the thumb at the live edge: how long the
stream has been going on its left and a red `LIVE` badge on its right, the
same badge as beside the pane's name. Red `LIVE` only ever means the pane is
watching live, now. Point at the timeline and it says the time under the
pointer, as a recording's seek bar does. Click or drag back along it and
let go, and the pane switches to the broadcast's
recording at that moment — the one Twitch is still making — with a seek bar
of its own and the chat replayed beside it, the way **Watch from the start**
plays a stream that has ended. A press within half a minute of the edge does
nothing, and the stream plays on.

To come back, press `LIVE` at the right-hand end of the recording's seek
bar — the badge's shape, outlined rather than red, since the pane is not
live until you press it: the pane opens the channel again at the live edge,
with its live chat, and the history keeps where you were in the
recording. Pressed in a pane's own window, it brings the pane back into
the main window first, as **Bring back** does. `LIVE` is there on any recording of a broadcast that
is still going on, however you opened it — from a rewind, a channel's page
or the history — while Perch has the channel live and Twitch is still
adding to the recording. Once the stream ends it goes at the next refresh
of your follows for a channel you follow, and otherwise once Twitch marks
the recording finished. Watching the recording right up to the end of what
Twitch has written so far simply waits there for more: `LIVE` is the way
back.

Rewinding needs a sign-in, and a channel that keeps its past broadcasts:
one that does not, or whose recording Twitch has not listed yet, says
`No past broadcast to rewind into` and stays live. The timeline is there
once the stream has been going for more than half a minute, when a list
Perch has fetched says when it started — your follows, popular, a category
or a search — and on a pane wide enough to aim along it.

### Panes in another order

With two panes or more on the page, drag a pane by its header onto another
pane and the two swap places. Nothing follows the pointer; the pane it is
over is outlined, and letting go anywhere else, or pressing `Esc`, leaves
everything where it was. `Shift+←` and `Shift+→` do the same from the
keyboard, swapping the pane you are on with the one before or after it.
Before and after are the order `1` to `4` count, row by row, rather than
directions on the screen: in a grid of two rows, the first pane of the
second row moved earlier goes to the end of the first. The pane you moved
stays the one the keys talk to, nothing restarts, and each pane keeps its
picture, its sound and its chat. `1` to `4`, `Tab`, the mini player and the
palette follow the new order, which lasts until Perch closes: it is not
saved. While one pane has the whole window there is nothing to rearrange.

### One pane, the whole window

With two panes or more, `Z` or the maximize on a pane's bar gives that pane
the whole of the watch page, its chat included, as if it were the only one.
The others are not drawn at all — no strip of names, no thumbnails, which
would take room from the pane you chose — and go on playing as they were,
each with its own sound. `1` to `4` and `Tab` move the maximize to the pane
they choose, so the keys always act on the pane you can see, and so does
picking a pane's quality from the palette, bringing one back from its own
window or opening one that is already open. `Z` or the same control again
shows every pane, back where it was, and so does `Esc`, before a second
`Esc` leaves the page; the palette has `Maximize …` and `Show all panes`
too.

Adding another pane, closing or popping out the one maximized, being left
with a single pane, or a pane in its own window coming back because its
stream stopped shows them all again. Leaving the watch page keeps the
maximize for when you come back, and the mini player shows every pane
meanwhile. The pane given the window has its quality chosen for its new
size, moving up to a sharper rendition with no black where one fits; the
others keep theirs, and the grid coming back lowers none.

### A pane in a window of its own

On Windows a pane can be popped out of the window into a small one of its
own, which stays on top of every other app — a game, a browser, a document —
while the rest of Perch goes behind them. Only its picture goes: its place,
its number and its chat stay in the main window, whose cell says `Playing in
its own window` and offers **Bring back**.

A pane goes out from the pop-out icon in its header, **Pop out** under the
bar's **More**, `P`, the palette's `Pop out …`, or the icon on its tile in the
mini player. The mini player's own **Pop out**, or `P` on the browse page,
pops out every pane it shows, each into a window of its own, stacked up from
the screen's bottom-right corner so that none covers another; `P` there again
brings them all back. A pane comes back from **Bring back** on its pop-out's
bar or on its cell, the same icon in its header, `P` in either window, or the
palette's `Bring … back`.

The pop-out is dragged by its picture and resized from its edges. Its bar
has play, the volume, a recording's seek row or a live stream's timeline,
**Bring back** and **Close**, and it answers `Space`, `M`, the arrows, `P` and
`Ctrl+W`. Closing it the way Windows closes any window — `Alt+F4`, or from
the taskbar — brings the pane back; its own **Close** closes the pane.
Closing Perch closes every pop-out with it. A popped pane whose stream goes
off or ends comes back to its cell, where what it offers next is.

Its quality follows its own window, the way a pane's follows its cell: make
the window larger and it moves to a sharper rendition once you stop, with no
black; make it smaller and it keeps what it has. Bringing it back into a
larger cell moves it up again.

It is not offered elsewhere yet. On macOS the window gpui would use hides
whenever another app is in front, which is the one thing a pop-out is for.

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

Read over anonymous IRC — no account, no token. Messages carry their
emotes (Twitch, FFZ, BTTV and 7TV), links are clickable, `@mentions` are drawn
in the colour of whoever is being addressed, and subs, gifts, raids and
announcements appear as their own rows rather than being dropped. A past
broadcast gets the same pane, replaying what was said as the picture reaches
it — see [Past broadcasts](#past-broadcasts).

The time is shown once a minute, as a break between messages, rather than once
per row — in a channel where fifteen messages share a minute, a column of
identical timestamps is not a ruler. It is written the way your system writes a
time, twelve-hour or twenty-four, in your language.

The icon with the two Ts in a pane's header, beside its pop-out and its ×,
opens the chat options: the text size — **Small**, **Default**, **Large** or
**Larger** — and **Time on every message**, which starts every message with
its time in place of the once-a-minute breaks. They apply to every chat at
once and are remembered. The channel's name in the pane header opens
it on twitch.tv — and so do **Open on twitch.tv** under the bar's **More** and
the palette's `Open … on twitch.tv`. On a live pane the site plays the stream
as well, with its own sound.

### Sending a message

A live channel's chat ends in a box that says **Send a message**. Click it,
type, and press `Enter`: the message goes, the box empties, and the message
shows in chat when Twitch echoes it back, the way everybody else's does. It
takes one line of up to 500 characters (a pasted line break becomes a space,
and the box stops at the limit). While the cursor is in it, the shortcuts
stand aside, so `M` and `Space` type (all but `Ctrl+K`, `Ctrl+,` and `Ctrl+R`,
which type nothing). A past broadcast's chat has no box.

Sending uses Twitch's own Send Chat Message API with your sign-in; reading
stays anonymous. If Twitch takes the message but does not post it — a
sub-only room, a message held for the moderators — its reason appears as a
line in that chat, and so does a short note when the message could not be
sent at all (`Message not sent: you are sending messages too quickly`).

When you cannot send, the box gives way to a line saying why, with a button
where there is something to do: **Sign in to chat** when nobody is signed in,
**Sign in again to chat** when the sign-in is from before Perch could send
(press **Sign in again** and enter the new code at `twitch.tv/activate`), and
**Only followers can chat here** in a followers-only room of a channel you do
not follow. Sub-only, emote-only and followers-only rooms you might be allowed
to talk in leave the box open and say so in it, as in **Send a message
(sub-only chat)**. There is no emote picker, no replying to a message, no
moderation commands and no whispers.

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
`deleted`, rather than vanishing from under your eye. When someone is timed
out or banned, chat says so — `ronni was timed out for 10 minutes`, `ronni was
banned` — and every message of theirs on screen, a resub's note included, is
greyed the same way.

Right-click a message to copy it: **Copy message** copies the text as it was
sent, emote names and all; **Copy name** the name; **Copy link**, when the
message has one, the link you clicked on or else its first; and **Copy emote
name**, when you clicked on an emote. A toast says what was copied. A click
anywhere else or `Esc` closes the menu, and the chat stays paused while it is
open, so the message does not scroll away from under it.

While you are signed in, names wear their Twitch badges in front of them —
broadcaster, moderator, VIP, the channel's own subscriber and bits badges —
in Twitch's order, and the pointer on one says what it is: `Subscriber, 14
months`. A past broadcast's chat replay wears them too. Signed out there are
no badges and no gap where they would be.

A reply has a dim line above it, `Replying to @name: what they said`, cut
short to one line, and its own text no longer starts with the `@name`.
Someone's first message in the channel has a faint blue tint and a
`First message` tag; a message sent with Highlight My Message has a warm wash;
an announcement is washed in the colour its sender picked. When the channel
has slow mode, followers-only, sub-only, emote-only or unique chat on, a quiet
line at the foot of the chat says so (`Slow mode 30s · Sub-only`), and a
moderator switching one after you joined is a notice in the chat.

When channels share their chat — Twitch's **Shared Chat**, during a
collaboration — what is said in a partner's chat is copied into this one, and
so are its subs, gifts and raids. Those lines start with a small tag naming
the channel they came from, and the pointer on it says `Said in <name>'s
chat`. The name is looked up once per
partner while you are signed in; until it is known, or if the lookup fails, the
line shows without a tag. A past broadcast's chat replay has no such tags.

Popular and the categories arrive a hundred at a time, which is Twitch's cap
per request rather than a choice. **Load more** at the end of the list fetches
the next hundred — a page you asked for, rather than a list that grows while you
scroll past it.

## Home

The browse page opens on **Home**, which answers "what should I watch": who
you follow that is live, as cards; then *Continue watching*, the recordings you
opened and did not finish, most recently watched first; then everyone else you
follow, as names. Continue watching is one row of the **History** tab's own
cards, as many as fit the window, and **Show all** beside its heading goes to
that tab when there are more. It needs no sign-in, since the history is the
app's own, and shows above the sign-in prompt when nobody is signed in. The
offline names lead with the channels you watched most recently, live or
recorded; the ones you never watched follow by name. Each says when that
channel was last live beside its name — "Live 3 hours ago", "Live yesterday",
"Live 2 weeks ago" — and a name opens the channel's page: its past
broadcasts, and a control for its chat, which connects whether or not anyone
is streaming. Each heading carries how many are under it, after the filter:
*Live now · 23*, *Continue watching · 5*, *Offline · 103*.

When each channel was last live comes from the same unpublished Twitch query
the recommendations use (see "The rail"), asked without your sign-in, a
hundred channels to a request: for everyone offline when the follows list
first arrives, then only for a channel whose stream has just ended or one you
have just followed, and for everyone again every fifteen minutes at most —
never at every minute's refresh. Twitch says when a channel's last stream
started, not when it ended, so Perch also notes the last time each refresh
saw a channel live and counts from whichever is later: a stream that ended
while Perch was open reads right to within a minute, and one that ended
before you opened it is counted from when it started. If the request fails,
the names simply go without it; a line in the log says why, and nothing else
changes. If Twitch refuses the query outright, Perch stops asking for the
rest of the session.

The list refreshes itself every minute. `Ctrl+R`, or Refresh beside the tabs,
asks again now — for whichever list is on screen, not just follows.

Who is live is sorted by viewers, and holds still while the pointer is on it,
the way chat does: a refresh then updates the numbers where they stand, drops
whoever ended and adds whoever started at the end, and the list is put back in
order once the pointer leaves — so a card is never swapped for its neighbour
between aiming at it and clicking. The rail does the same. So do the offline
names, on Home and in the rail: somebody whose stream ends joins the end of
them until the pointer leaves, rather than landing in the middle and pushing
every name after them down.

When somebody you follow goes live, a notice says so in the corner; click it
to watch them, or `+ Add` beside it to open them next to what is playing.
Notices wait while the pointer is on them, and stay at least a couple of
seconds after it moves off, so one you are reading or reaching for does not
go. A pane left on a channel that was off, or whose broadcast ended, starts by
itself when a later poll finds them on again — open a channel before they
start and it begins without you. That is the pane's **Start when they go
live** switch, on unless you turn it off, and it is there only for channels
you follow, since the poll that notices is the follows poll.

The box at the top of Home filters all three as you type, live, unfinished and
offline, by the same few-letters-of-a-name match the palette uses; a recording
answers to its channel's name or to a word of its title. It is the opposite of
the search box in the title bar: that one asks Twitch, this one asks the app,
and nothing typed here leaves it — until nothing on the page matches, when it
offers to ask Twitch instead.

Cards are as wide as the window allows: the grid takes the room it has and
divides it, rather than leaving whatever a fixed width could not use as a gutter
down one side. Viewer count and uptime sit on the thumbnail; the name, title and
game are underneath, each on one line and cut with an ellipsis where it does not
fit. Resting the pointer on one shows the whole of it.

Whatever is playing while you browse keeps playing, with its sound, in a small
player in the bottom-right corner of the page, clear of the list's scrollbar:
one picture, or up to four two to a row. A stream popped out into a window of
its own stays there instead, since it is on screen already.
Click a picture to go back to watching it; the `×` that appears on a picture
under the pointer closes just that stream, and on Windows the pop-out icon
beside it moves that stream into a window of its own (see "A pane in a window
of its own"). The bar under the pictures says what is playing and has controls
for all of it — Mute all (or Unmute all), on Windows **Pop out**, which does
that for every stream there, Back to watching and Stop all. Mute all silences the streams without touching
anyone's volume: it is never saved, it lasts until you change a stream's volume
yourself, and Unmute all leaves a stream you had muted muted. Every list leaves
room at its foot, so nothing is stuck under the player. The **Mini player**
switch in Settings turns it off, in which case leaving the watch page stops
the streams instead — all but those popped out, which play on in their own
windows — which is the cheaper answer if you go to Home to pick the next
thing rather than to glance at the list.

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
pictures too, asked for once each time Perch starts, and under each name the
same "Live 3 hours ago" Home shows, where a live row says what is on.

Rest the pointer on a live row for a moment and its card comes up beside the
rail: the stream's picture with how many are watching and for how long, its
title and what they are playing — for a recommendation, the title, the game
and the reason, since Twitch sends no picture with those. It goes when the
pointer leaves the row, and once one is up, the next row's comes up straight
away as you move down the list. A window too small to hold the card beside
the rail shows none, rather than covering the rail or the title bar.

Recommendations start from the channels open in your panes and the ones you
watched most recently, six at most. For each, Twitch says who else its viewers
are watching — the suggestions its own website shows in its sidebar, from a
query Twitch has never published, which Perch asks without sending your
sign-in. Anyone you follow or are already watching is left out, though one you
have just opened from the list keeps its place, marked as watching, until the
pointer leaves the rail. They are asked
for when you open a channel they have not been asked about yet, and otherwise
every five minutes at most, only while you are signed in and the rail is open
or the guide is showing them.
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
typed something, and open the channel's page, as their names do on
Home; a live channel's past broadcasts are a row of their own. A
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
Home, the **Past broadcasts** control that appears on a live card,
or the palette's row for it — or search for it by name, since a search lists
the channels that answer to it and are not on, as names under the live ones.
The recordings are cards like the streams are, with the length where a
stream's card has its viewer count, and how long ago it was underneath. One
still being recorded carries the live dot and says how long it has been going
so far; it can be watched from the start while the stream is on. While the
channel is off, the bar at the top says when it is on next, beside **Open
chat**, if it has a schedule on Twitch, or that it is on a break.

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

Where Twitch muted a recording's sound for music it matched, the seek bar
marks the stretch with a dull amber band around the bar, and pointing there
says `muted` after the time. **Playback speed** under **More** plays a
recording at 0.75x up to 2x, in quarter steps, with voices kept at their own
pitch; while it is not 1x the speed is written after the length on the seek
bar, and clicking it opens the speeds again. It belongs to the pane: a
quality change keeps it, nothing saves it, and whatever the pane plays next
starts at 1x. The chat replay keeps pace.

Every recording you open is kept on the **History** tab, newest first: the ones
you are part-way through under *Continue watching*, each saying where you left
it, then the ones you finished. The first row of the unfinished ones is on
Home too. Opening a recording again — from there, from its
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
from Twitch's own copy of it, with the same emotes, links, colours and badges
as a live pane and the time breaks reading the broadcast's own clock. Jump
somewhere else and the pane swaps to the conversation around the new moment,
starting a little before it, so you land mid-chat rather than in a blank. `C`
hides it, and that is remembered per channel the way it is for a live pane. A
broadcast still being recorded has its replay too, running about thirty seconds
behind live.

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
| **Client ID** | An application you register at [dev.twitch.tv](https://dev.twitch.tv/console) | Lists the channels you follow, and sends chat messages |
| **auth-token** | The `auth-token` **cookie** from twitch.tv | Prime/Turbo ad suppression and sub-only qualities |

Neither can do the other's job.

**Sign out**, at the foot of the settings sheet beside who is signed in,
forgets the sign-in at once and leaves the app as it is with nobody signed
in: the follows go, and the settings sheet, Home, the browse lists and every
chat offer **Sign in**, which shows a new code to enter at
`twitch.tv/activate`. Everything else in settings stays
as it was. The next launch starts signing in again on its own, as a first
launch with a Client ID does.

To create the Client ID: register an application, set **OAuth Redirect URL** to
`http://localhost` (required by the form, unused by this app) and **Client Type**
to **Public**. No client secret — sign-in uses the device code flow, so nothing
secret is ever stored in the binary. Paste the Client ID into settings and the
title bar will show a code to enter at `twitch.tv/activate`, on either page, until
the sign-in lands; Home shows it too.

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
or closing, the rail folding, the window resizing or going fullscreen, a pane
given the whole window or every pane shown again, a popped-out pane's own
window resizing, or the pane coming back from it — and only ever upwards: a
pane that has grown moves to a sharper rendition, and a pane that has shrunk
keeps what it has. A quality picked from a pane's own
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
carry on without a jump. Pause and volume carry over. On a live pane, opening
the quality menu gets streamlink ready for the renditions next to the one
playing (up to four) while you choose, which takes it about two seconds, so a
pick of one of them that is ready only has the new picture to wait for. Those
waiting streamlinks fetch nothing until a pick, and stop a few seconds after
the menu closes. Meanwhile the pane's quality pill names the rendition you
picked and breathes, its controls stay up, and its menu marks it, until it
takes over; if it cannot, a note says so and the pill goes back to what plays.
For those moments the pane is fetching and decoding two streams. A quality
change in the settings still starts every pane over, each saying it is
starting until its new picture arrives.

## Layout

```
crates/
  mpv-frames    libmpv loaded at runtime, software render to BGRA
  streamlink    supervises streamlink as a headless byte source
  twitch-chat   chat read over anonymous IRC; a recording's chat replayed
  twitch-api    device-code sign-in, follows, browsing, search, chat badges
                and sending a chat message
  emotes        Twitch/FFZ/BTTV/7TV resolution, disk image cache
  settings      persisted user settings, and what has been watched
  perch         the app
```

Every crate except the last is free of UI types, so the pieces are testable
without a window.

## License

MIT — see [LICENSE](LICENSE).
