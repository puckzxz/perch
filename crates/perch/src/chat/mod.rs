//! The chat pane.
//!
//! Uses a bottom-anchored `ListState`, which is what keeps new messages pinned
//! to the bottom the way every chat client does, without manual scrolling.
//!
//! Emotes are the interesting part. GPUI has no inline-image-in-text, so a
//! message cannot be one wrapped paragraph with pictures in it. Instead each
//! message is tokenised into words and emotes and laid out in a `flex_wrap`
//! row: wrapping then happens at token boundaries, which is where you want it.
//!
//! The view is split by what it handles, the way `root/` is: this file has
//! the view, its rows and how a message is drawn; `room` what the room says
//! about itself (its modes, a timeout or a ban); `badges` the chat badges;
//! `reply` a reply's context line; `copy_menu` what a right-click on a row
//! offers to copy; and `composer` the box at the foot of a live chat that
//! sends a message.

mod badges;
pub mod composer;
mod copy_menu;
mod reply;
mod room;

use std::collections::HashMap;
use std::sync::Arc;

use emotes::{apply_named_emotes, tokenize, EmoteLoader, EmoteSets, ImageCache, Token};
use gpui::{
    div, img, list, point, prelude::*, px, AnyElement, Context, Entity, EventEmitter,
    ListAlignment, ListState, MouseButton, Pixels, RetainAllImageCache, SharedString, Subscription,
    Task, Window,
};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};

use twitch_chat::{ChatClient, ChatEvent, ChatMessage, ChatNotice, Replay, RoomModes};

use crate::chat_badges::ChatBadges;
use crate::chat_display::{self, ChatDisplay, Metrics};
use crate::chat_text::{self, Kind};
use crate::chat_tint;
use crate::chat_words;
use crate::controls;
use crate::motion;
use crate::shared_chat::{self, Label};
use crate::theme;
use crate::veil;
use crate::video::PositionHandle;

use self::composer::{Composer, Step};
use self::copy_menu::{CopyMenu, Target};

/// Emote names are worth showing on hover: half of chat is emotes, and knowing
/// what one is called is the difference between reading a message and guessing.
struct EmoteTooltip {
    name: SharedString,
}

impl Render for EmoteTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(theme::CONTROL_PAD_X))
            .py(px(theme::CONTROL_PAD_Y))
            .rounded(px(theme::RADIUS))
            .bg(theme::surface_raised())
            .text_size(px(theme::TEXT_LABEL))
            .text_color(theme::text())
            .child(self.name.clone())
    }
}

/// Messages kept in memory. Old ones drop off the top: chat runs forever, and
/// a pane holding everything it ever saw is a leak with a nicer name.
///
/// Set when scrolling back was not really possible. Now that the pane opens
/// with a backlog and holds its position when you scroll, the cap is what says
/// how far back you can actually read, so it is worth more than it costs: a row
/// is a `ChatMessage` and a few `SharedString`s, and only the visible ones are
/// ever laid out.
const MAX_MESSAGES: usize = 1_000;

/// Wall-clock time for a row, formatted once on arrival.
///
/// Twitch's `tmi-sent-ts` is when the *server* saw the message, which is what
/// you want: it stays correct for a backlog and does not drift with our own
/// processing. Falls back to now for rows we generate ourselves. Written the
/// way the system writes a time - see [`crate::clock`] - so a US machine reads
/// `11:41 PM` where a German one reads `23:41`.
fn clock(sent_at: Option<u64>) -> SharedString {
    let when = sent_at
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms as i64))
        .unwrap_or_else(chrono::Utc::now);
    SharedString::from(crate::clock::stamp(when.with_timezone(&chrono::Local)))
}

#[derive(Clone)]
enum RowKind {
    Message(Box<ChatMessage>),
    /// A sub, a gift, a raid, an announcement — the moments the streamer reacts
    /// to on camera, which a chat without them makes you watch them thank
    /// somebody you never saw.
    Event(Box<ChatNotice>),
    /// Something the app itself has to say: joined, disconnected, cleared.
    Notice(SharedString),
}

impl RowKind {
    /// The wash behind this row, if it needs one: see `chat_tint`.
    ///
    /// Events are washed rather than outlined or barred, because the row has to
    /// keep its place in the ruler of timestamps down the side. Two intensities
    /// and no more, besides the colour an announcement's sender picked:
    /// enumerating Twitch's `msg-id` values is a losing game, and the sentence
    /// in the row already says which event it was. A message is washed only
    /// when Twitch marks it, as highlighted or as someone's first.
    fn wash(&self) -> Option<gpui::Hsla> {
        match self {
            RowKind::Event(notice) => Some(chat_tint::event_wash(notice).color()),
            RowKind::Message(message) => {
                chat_tint::message_wash(message).map(chat_tint::Wash::color)
            }
            RowKind::Notice(_) => None,
        }
    }
}

#[derive(Clone)]
struct Row {
    kind: RowKind,
    /// Decided when the row is appended, never from its index.
    ///
    /// The backlog drains from the front once `MAX_MESSAGES` is reached, which
    /// shifts every surviving row's index by one — so an index-derived stripe
    /// inverted the whole pane on every message past the cap. At five messages
    /// a second that is a 5 Hz flicker across the entire list, faint enough to
    /// never be diagnosed and constant enough to be felt.
    striped: bool,
    /// Formatted once, on arrival. Doing it per frame would redo it for every
    /// visible row on every repaint, and chat repaints constantly.
    stamp: SharedString,
    /// Stable for this row's whole life, unlike its index — which shifts every
    /// time the backlog drains. Element ids inside a row are built from this,
    /// so a link keeps its hover state while messages arrive above it.
    seq: u64,
    /// A moderator has since deleted it. Kept and greyed rather than removed:
    /// the replies around it still refer to it, and a row that vanishes from
    /// under the eye reads as the pane skipping.
    deleted: bool,
}

/// Where a pane's messages come from.
pub enum Feed {
    /// A channel's chat as it happens, opening with `history` lines from
    /// before — see [`twitch_chat::history`] for where they come from and
    /// what asking costs; zero opens an empty pane.
    Live { channel: String, history: usize },
    /// The chat of a recording, replayed against where the recording is.
    /// `room_id` is the channel's numeric id, for its third-party emotes.
    Replay {
        video_id: String,
        channel: String,
        room_id: String,
        position: PositionHandle,
    },
}

/// What a chat tells whoever made it.
pub enum ChatViewEvent {
    /// A line arrived copied from a Shared Chat partner's room, by its
    /// numeric id, that this chat cannot name yet. Said at every such line
    /// until the name arrives (`learn_rooms`): the root decides whether it
    /// is worth asking about (`RootView::ask_channel_name`), which keeps the
    /// one-ask-per-id rule in one place for every chat.
    UnknownRoom(String),
    /// The chat's room, by its numeric id: from each `ROOMSTATE` live (on
    /// joining, and again whenever a mode changes) and once from a replay.
    /// The root answers with the badges for it (`RootView::on_chat_room`),
    /// asking the worker for any it has not got; it is told every time, and
    /// `chat_badges::Library::wants` keeps that to one ask per room.
    Room(String),
    /// A row of the copy menu was pressed (`copy_menu`): put `text` on the
    /// clipboard and say so with `toast`. The root does both
    /// (`RootView::copy`), which keeps the clipboard written in one place,
    /// and takes the rest of the press's run.
    Copy { text: String, toast: &'static str },
    /// The composer's `Enter`, with a message worth sending (`composer`):
    /// send `message` to the room whose numeric id is `room_id`, as the
    /// signed-in user. The root asks the worker (`RootView::send_chat`) and
    /// hands back what became of it (`ChatView::sent`).
    Send { room_id: String, message: String },
    /// The button beside a closed composer's reason was pressed: sign in,
    /// sign in again, or open the settings sheet. The root does each
    /// (`RootView::on_composer_step`); opening the sign-in page with the
    /// code is the chat's own.
    Step(Step),
}

impl EventEmitter<ChatViewEvent> for ChatView {}

/// The thread behind a feed, held only so that dropping the view stops it.
enum Link {
    Live { _client: ChatClient },
    Replay { _replay: Replay },
}

pub struct ChatView {
    rows: Vec<Row>,
    /// What to say while the pane is still empty. Formatted once.
    waiting: SharedString,
    /// Whether this is a recording's chat rather than a channel's. The rows
    /// are the same; what the pane says about itself is not.
    replay: bool,
    /// Whether the source has said it is up — see [`ChatEvent::Connected`].
    /// Live, that puts a row in the pane, so the empty state is over; a
    /// replay says nothing, and this is how an empty pane knows to stop
    /// saying it is loading.
    loaded: bool,
    /// Login to chat colour, filled in as people talk.
    ///
    /// Lets an `@mention` be drawn in the colour of the person it refers to,
    /// which is the closest thing to a thread you get without threading. A miss
    /// renders plainly rather than guessing — the cache fills itself from the
    /// same messages you are already reading.
    colors: std::collections::HashMap<String, u32>,
    /// The stripe of the last row appended, toggled per row.
    striped: bool,
    /// Ever-increasing row id. See `Row::seq`.
    next_seq: u64,
    list: ListState,
    cache: Arc<ImageCache>,
    /// Decoded emotes, held per pane rather than per process.
    ///
    /// Without this, `img(path)` falls through to GPUI's global asset cache
    /// (`App::loading_assets`), which has no eviction of any kind: every emote
    /// this pane ever drew — and, for an animated one, every *frame* of it as
    /// its own atlas tile — stays resident for the life of the process, long
    /// after the channel is closed. A 7TV set runs to several hundred emotes
    /// and a 40-frame animated one is ~655 KB decoded plus 40 tiles, so an
    /// evening of channel-hopping is the bill for every channel at once.
    ///
    /// Scoping it here means the pane owns what it decoded and `on_release_in`
    /// gives it all back. What that bounds is emotes for *panes that are open*,
    /// which is the growth that actually compounds. It does not bound a single
    /// channel left open for hours: that set saturates on its own, and capping
    /// it would need a byte budget whose eviction can only ever be a guess
    /// about what is still on screen.
    emote_images: Entity<RetainAllImageCache>,
    /// Keeps the release hook alive; a dropped `Subscription` unsubscribes.
    _release: Subscription,
    /// Name lookups for FFZ / BTTV / 7TV emotes, filled in as they load.
    emote_sets: EmoteSets,
    emote_loader: EmoteLoader,
    /// Whether new rows are being held back because the pointer is over the
    /// pane. A list that moves under the pointer is a link you cannot click
    /// and a line you cannot finish, so while the pane is pointed at and
    /// following live, arrivals wait in `held` and land when it leaves. See
    /// [`sync_hold`](Self::sync_hold).
    hold: bool,
    held: Vec<(RowKind, Option<u64>)>,
    /// How big the text is and whether every message carries its time: the
    /// root's settings, mirrored here when the pane is made and on every
    /// change from the chat options menu, through
    /// [`set_display`](Self::set_display).
    display: ChatDisplay,
    /// The labels of the Shared Chat partners named so far, by numeric room
    /// id: the root's (`shared_chat::SourceRooms`), mirrored here when the
    /// chat is made and as each name arrives, through
    /// [`learn_rooms`](Self::learn_rooms). Empty on a replay, whose lines
    /// carry no source room.
    rooms: HashMap<String, Label>,
    /// Whether the pointer is in the window, as `render` last read it, for
    /// the rows the list lays out after it: a Shared Chat tag gives its
    /// tooltip only while it is ([`source_tag`](Self::source_tag)), because
    /// a tooltip outlives a pointer that leaves the window (HANDOFF, GPUI
    /// traps), and the left-hand pane's chat runs to the window's edge.
    window_hovered: bool,
    /// The chat badges this chat can draw, the global set and its own
    /// channel's: the root's (`chat_badges::Library`), mirrored here as its
    /// room becomes known and as each answer arrives, through
    /// [`set_badges`](Self::set_badges). Empty signed out, when no badge is
    /// drawn and none leaves a gap.
    badges: ChatBadges,
    /// The room's numeric id, once the source has said it
    /// ([`ChatEvent::RoomState`]): what the root keys this chat's badges on.
    room_id: Option<String>,
    /// The room's modes as last heard, merged from every `ROOMSTATE`
    /// (`RoomModes::merged`); `None` until the first, and for a replay,
    /// which has none. Drawn as the quiet line at the foot of the pane
    /// (`chat_words::modes_line`), and what a composer, when there is one,
    /// reads to say why it cannot send.
    modes: Option<RoomModes>,
    /// The copy menu, while a right-click has it open over a row; see
    /// `copy_menu`. For the moment only, like every menu.
    menu: Option<CopyMenu>,
    /// What a right press landed on inside the row that is about to hear
    /// it — a link or an emote — said by that piece's listener for the
    /// row's own (`copy_menu`). Taken by the row in the same press.
    pressed: Option<Target>,
    /// How many copy menus this chat has opened: what keys each one's
    /// arrival, so one opened in place of another fades in as itself.
    menus_opened: u64,
    /// The channel, by login, for a live chat; `None` for a replay. What
    /// the root works out this chat's [`composer::Access`] for.
    channel: Option<String>,
    /// The box that sends a message, at the foot of a live chat; `None` for
    /// a replay, which takes none. See `composer`.
    composer: Option<Composer>,
    _link: Link,
    _pump: Task<()>,
    _emote_pump: Task<()>,
}

impl ChatView {
    pub fn new(
        feed: Feed,
        cache: Arc<ImageCache>,
        display: ChatDisplay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (link, mut events, waiting, replay, live_channel) = match feed {
            Feed::Live { channel, history } => {
                let (client, events) = ChatClient::connect(&channel, history);
                // The words the row that replaces it uses, "Connected to
                // {channel}'s chat": not IRC's `#channel`, which is a name for
                // the room nobody reading the pane needs to know.
                let waiting = format!("connecting to {channel}'s chat…");
                (
                    Link::Live { _client: client },
                    events,
                    waiting,
                    false,
                    Some(channel),
                )
            }
            Feed::Replay {
                video_id,
                channel,
                room_id,
                position,
            } => {
                let (replay, events) =
                    Replay::start(video_id, channel, room_id, move || position.playhead());
                let waiting = "loading chat replay…".to_string();
                (
                    Link::Replay { _replay: replay },
                    events,
                    waiting,
                    true,
                    None,
                )
            }
        };
        let composer = live_channel.is_some().then(|| Composer::new(window, cx));

        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this: &mut ChatView, cx| {
                        this.apply(event, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let (emote_loader, mut emote_ready) = EmoteLoader::new();
        let emote_sets = emote_loader.sets();

        // Each provider lands separately, so repaint as they arrive rather than
        // waiting for all six requests.
        let emote_pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while emote_ready.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        // Scrolling changes whether the jump-to-live pill should be up, and
        // nothing else would repaint a quiet channel to notice.
        // `measure_all` makes the scrollbar's extent come from every row rather
        // than only the rendered ones. It is not a complete fix: the pass runs
        // once. Rows spliced in afterwards stay unmeasured until something
        // draws them — and a change in the list's *width* throws away every
        // measurement it has, because `List::prepaint` rebuilds the whole tree
        // as `Unmeasured` without re-running the pass, so resizing a pane
        // collapses the extent to whatever is on screen. In a window that is
        // one to four resizable panes that is the bigger source of drift, not
        // the appended rows. The thumb's *position* is right throughout, which
        // is the half that answers "how far back am I".
        //
        // Re-arming the pass is possible — `ListState` is `Clone` over an
        // `Rc<RefCell<_>>` and `measure_all` mutates the shared inner, so
        // `self.list.clone().measure_all()` would do it without touching the
        // scroll pin. It is deliberately not done: `layout_all_items` rebuilds
        // the whole `SumTree` every pass, so paying that on a cadence to stop a
        // thumb from drifting is a worse trade than the drift.
        let list = ListState::new(0, ListAlignment::Bottom, px(400.)).measure_all();
        let watcher = cx.entity().downgrade();
        list.set_scroll_handler(move |_event, _window, cx| {
            watcher.update(cx, |_, cx| cx.notify()).ok();
        });

        // Deliberately not a row. A backfill arrives stamped with the times
        // those messages were really sent, all of them older than now, so a
        // "connecting…" row would sit above an hour of history wearing a later
        // timestamp than everything beneath it. It is a state of the pane, not
        // an event in the log, and it belongs in the empty space it explains.
        let emote_images = RetainAllImageCache::new(cx);
        // GPUI does not free atlas tiles when the `Arc<RenderImage>` goes:
        // `clear` is what calls `App::drop_image` for each entry, and it needs a
        // `Window`, which `Drop` does not have. Same shape as `VideoView`'s
        // frame release, and for the same reason.
        let release = cx.on_release_in(window, {
            let emote_images = emote_images.clone();
            move |_this: &mut Self, window, cx| {
                emote_images.update(cx, |cache, cx| cache.clear(window, cx));
            }
        });

        Self {
            rows: Vec::new(),
            waiting: SharedString::from(waiting),
            replay,
            loaded: false,
            colors: std::collections::HashMap::new(),
            striped: false,
            next_seq: 0,
            list,
            cache,
            emote_images,
            _release: release,
            emote_sets,
            emote_loader,
            hold: false,
            held: Vec::new(),
            display,
            rooms: HashMap::new(),
            window_hovered: false,
            badges: ChatBadges::default(),
            room_id: None,
            modes: None,
            menu: None,
            pressed: None,
            menus_opened: 0,
            channel: live_channel,
            composer,
            _link: link,
            _pump: pump,
            _emote_pump: emote_pump,
        }
    }

    fn apply(&mut self, event: ChatEvent, cx: &mut Context<Self>) {
        match event {
            // "connected", not "joined #channel": the second is IRC's word for
            // it, and nobody reading a chat pane is thinking about IRC.
            //
            // Not a row on a replay. A live join is a moment in the log, and
            // the row wears the time it happened; a replay's "connected" is
            // its buffer being ready, which is not a moment in a chat that
            // was said last Tuesday, and a row stamped with today would draw
            // a break between two of Tuesday's minutes.
            ChatEvent::Connected { channel } => {
                self.loaded = true;
                if !self.replay {
                    self.append(
                        RowKind::Notice(format!("Connected to {channel}'s chat").into()),
                        None,
                    );
                }
            }
            ChatEvent::RoomState { room_id, modes } => self.room_state(room_id, modes, cx),
            ChatEvent::Message(message) => {
                let sent_at = message.sent_at;
                self.colors.insert(message.login.clone(), message.color);
                self.name_source(message.source_room.as_deref(), cx);
                self.append(RowKind::Message(message), sent_at)
            }
            ChatEvent::Notice(notice) => {
                let sent_at = notice.sent_at;
                // A resub note counts as having spoken, so an `@them` later in
                // the conversation still finds their colour.
                if let Some(body) = &notice.body {
                    self.colors.insert(body.login.clone(), body.color);
                }
                // The notice's own room, not the body's: most copied events
                // have no body.
                self.name_source(notice.source_room.as_deref(), cx);
                self.append(RowKind::Event(notice), sent_at)
            }
            ChatEvent::Cleared {
                login: Some(login),
                ban_seconds,
            } => self.clear_person(&login, ban_seconds),
            // The whole chat: said, and nothing greyed, as it always was.
            ChatEvent::Cleared { login: None, .. } => {
                self.append(RowKind::Notice("Chat was cleared".into()), None);
            }
            ChatEvent::Disconnected { reason } => {
                let text = if self.replay {
                    format!("Chat replay interrupted: {reason} — retrying")
                } else {
                    format!("Disconnected: {reason} — retrying")
                };
                self.append(RowKind::Notice(text.into()), None);
            }
            // The recording was repositioned. What is on screen was said
            // around a moment the picture has left; the backlog for the new
            // one follows on the same channel, so the pane is never both.
            // The colours stay: it is the same chat, and the people in it
            // have not changed.
            ChatEvent::Reset => {
                let count = self.rows.len();
                self.rows.clear();
                self.held.clear();
                // About a row that has gone.
                self.close_menu(cx);
                self.list.splice(0..count, 0);
                self.striped = false;
                self.loaded = false;
            }
            // Greyed where it stands — see `Row::deleted`. One still waiting
            // on the pointer was never shown, and is not shown now.
            ChatEvent::Deleted { id } => {
                let deleted = |kind: &RowKind| matches!(kind, RowKind::Message(message) if message.id.as_deref() == Some(id.as_str()));
                for row in self.rows.iter_mut().filter(|row| deleted(&row.kind)) {
                    row.deleted = true;
                }
                self.held.retain(|(kind, _)| !deleted(kind));
            }
            ChatEvent::Unavailable { reason } => self.append(RowKind::Notice(reason.into()), None),
        }
    }

    /// Ask for the name of the room a line was copied from, its
    /// `source_room`, if it was copied from one this chat cannot name yet.
    /// See [`ChatViewEvent`].
    fn name_source(&self, source_room: Option<&str>, cx: &mut Context<Self>) {
        if let Some(id) = source_room {
            if !self.rooms.contains_key(id) {
                cx.emit(ChatViewEvent::UnknownRoom(id.to_string()));
            }
        }
    }

    /// Label lines copied from these Shared Chat partners from now on, and
    /// the ones already here: names the root has just learned, or every one
    /// it knew when this chat was made (`RootView::watch_chat`).
    ///
    /// A row that gains a label is wider on its first line, so it may wrap
    /// differently, and the list goes on trusting its old measurement of it
    /// — so if any row here, held ones aside, is from one of these rooms,
    /// every row is handed back unmeasured ([`remeasure`](Self::remeasure)).
    /// A name arrives once a partner per session, so this is rare.
    pub fn learn_rooms(&mut self, learned: &[(String, Label)], cx: &mut Context<Self>) {
        self.rooms.extend(
            learned
                .iter()
                .map(|(id, label)| (id.clone(), label.clone())),
        );
        let from = |source_room: &Option<String>| {
            source_room
                .as_ref()
                .is_some_and(|room| learned.iter().any(|(id, _)| id == room))
        };
        let relabelled = self.rows.iter().any(|row| match &row.kind {
            RowKind::Message(message) => from(&message.source_room),
            RowKind::Event(notice) => from(&notice.source_room),
            RowKind::Notice(_) => false,
        });
        if relabelled {
            self.remeasure();
        }
        cx.notify();
    }

    /// Whether the pane is showing the newest message.
    ///
    /// A bottom-aligned list follows new messages until you scroll, then holds
    /// where you put it — and resumes only once you reach the bottom again. In
    /// a fast channel that can be a very long way down, with nothing on screen
    /// to say so, which is what the scrollbar and the pill are both for.
    fn at_live(&self) -> bool {
        let furthest = self.list.max_offset_for_scrollbar().height;
        let scrolled = -self.list.scroll_px_offset_for_scrollbar().y;
        // A pixel of slack: the offset is derived from measured item heights
        // and lands a hair short of the maximum more often than not.
        furthest <= Pixels::ZERO || scrolled >= furthest - px(1.)
    }

    /// Jump back to the newest message and start following again.
    ///
    /// What "following" *is*, for a `ListAlignment::Bottom` list, is
    /// `logical_scroll_top == None` — the state the wheel restores when you
    /// scroll back to the end. So the job is to clear the pin, and the whole
    /// question is which call clears it without doing anything else.
    ///
    /// `scroll_to`/`scroll_by` are out: they *set* a pin, so they land at the
    /// bottom and are then left behind by the next message. `reset` clears it,
    /// which is why this used to call it — but it also splices the entire list
    /// and re-arms the measuring pass, so with `measure_all` on, every click
    /// laid out all 1000 rows, emote images included, in one frame. It drops
    /// wheel events until the next paint too; its own doc comment says so.
    ///
    /// `set_offset_from_scrollbar` clamps the offset to `scroll_max` and, for a
    /// bottom-aligned list sitting exactly at `scroll_max`, sets
    /// `logical_scroll_top = None` and nothing else (`list.rs`'s
    /// `set_offset_from_scrollbar`). Any offset past the end clamps there, so
    /// the value is "further than the list can go" rather than a real
    /// coordinate. It is a no-op before the first layout, which is fine: an
    /// unlaid-out bottom-aligned list is already following.
    ///
    /// One thing is genuinely lost with `reset`. It re-armed `measure_all` too,
    /// so the stall it caused left every row measured and the scrollbar extent
    /// briefly correct. It no longer will: rows that arrived while you were
    /// scrolled back stay unmeasured until something draws them. That is the
    /// trade — a stall on every click, for a thumb that is a few pixels short
    /// until you scroll back through it.
    fn follow_live(&mut self, cx: &mut Context<Self>) {
        self.list
            .set_offset_from_scrollbar(point(px(0.), px(f32::MAX)));
        cx.notify();
    }

    /// Append one row, trimming the backlog, keeping `ListState` in step.
    ///
    /// `ListState` tracks item count separately from our Vec, so every mutation
    /// here needs a matching splice or the list renders the wrong indices.
    fn push(&mut self, kind: RowKind, sent_at: Option<u64>) {
        self.striped = !self.striped;
        self.next_seq += 1;
        self.rows.push(Row {
            kind,
            striped: self.striped,
            stamp: clock(sent_at),
            seq: self.next_seq,
            deleted: false,
        });
        let count = self.rows.len();
        self.list.splice(count - 1..count - 1, 1);

        if self.rows.len() > MAX_MESSAGES {
            let excess = self.rows.len() - MAX_MESSAGES;
            self.rows.drain(0..excess);
            self.list.splice(0..excess, 0);
        }
    }

    /// Add a row, or hold it back while the pointer is over the pane.
    fn append(&mut self, kind: RowKind, sent_at: Option<u64>) {
        if self.hold {
            self.held.push((kind, sent_at));
        } else {
            self.push(kind, sent_at);
        }
    }

    /// Hold new rows back while the pointer is over a pane that is following
    /// live, and let them land the moment it is not.
    ///
    /// A chat that moves under the pointer is a link that moves as you reach
    /// for it and a line that scrolls away as you read it, which is what
    /// every chat client pauses for. Holding the rows rather than pinning the
    /// list is what makes this cheap and safe: nothing about the list changes
    /// while it is held, so there is no scroll position to keep in step, and
    /// the rows arrive on release the way a burst of chat always does. A pane
    /// scrolled back is left alone — its position is already held, by the
    /// reader, and rows can land below it without moving anything.
    ///
    /// `over` is measured, not reported, like every hover in this app: the
    /// list's bounds from the last layout against the pointer now, so a
    /// pointer that left the window without a move event still releases the
    /// rows at the next repaint.
    ///
    /// The copy menu holds the rows too, for as long as it is open, wherever
    /// the pointer is — on the menu, past the chat's edge, or off the window
    /// — so the row it is about stays where it was right-clicked. Closing it
    /// hands the hold back to the pointer, measured like this, so a pointer
    /// still over the chat goes on holding it; see `copy_menu`.
    fn sync_hold(&mut self, over: bool) {
        let hold = (over || self.menu.is_some()) && self.at_live();
        if !hold && !self.held.is_empty() {
            for (kind, sent_at) in std::mem::take(&mut self.held) {
                self.push(kind, sent_at);
            }
        }
        self.hold = hold;
    }

    /// Stop holding rows back, and land the ones held: for a pane whose chat
    /// has gone off the screen — the browse page, or another pane given the
    /// watch page — told so by `RootView::restage`.
    ///
    /// The hold is let go only by [`sync_hold`](Self::sync_hold), and that
    /// runs only when the chat is drawn. Without this, a chat put away with
    /// the pointer over it would go on holding every row that arrived, past
    /// the cap that bounds the rows on screen, for as long as it was away.
    /// Nothing for a chat holding nothing back, which is most of them, so
    /// telling one on every restage costs nothing. A copy menu open over a
    /// chat that has gone off the screen goes with it, as it would hold the
    /// rows otherwise.
    pub fn let_go_hold(&mut self, cx: &mut Context<Self>) {
        self.close_menu(cx);
        if !self.hold && self.held.is_empty() {
            return;
        }
        self.sync_hold(false);
        cx.notify();
    }

    /// Draw this chat as `display` says from now on: the chat options menu
    /// changed it, for every chat at once (`RootView::set_chat_display`).
    /// Nothing for a chat already drawn that way.
    ///
    /// Every row the list has measured is the wrong height after this — a
    /// text size changes every row, and the times change which rows carry a
    /// break — and the list goes on trusting a measurement until something
    /// tells it otherwise, so rows would overlap or leave gaps. So every row
    /// is handed back to it unmeasured ([`remeasure`](Self::remeasure)).
    pub fn set_display(&mut self, display: ChatDisplay, cx: &mut Context<Self>) {
        if self.display == display {
            return;
        }
        self.display = display;
        self.remeasure();
        cx.notify();
    }

    /// Hand every row back to the list unmeasured, for a change that may
    /// have made any of them a different height: a new text size or times
    /// setting (`set_display`), or a Shared Chat label arriving for rows
    /// already drawn (`learn_rooms`).
    ///
    /// Following live, by `reset`, which also runs the measuring pass again
    /// so the scrollbar's extent is right; it costs one layout of every row,
    /// which a press in a menu can afford, and the pane is following anyway,
    /// so the pin `reset` clears is not there to lose. Scrolled back, `reset`
    /// would throw the reader to the bottom, so the rows go back in two
    /// splices either side of the row at the top: a splice keeps the scroll
    /// position on an index outside its range and puts it at the start of
    /// one inside it, which for the second splice is that same row. The
    /// reader keeps their place, at the top of that row, and the thumb's
    /// extent is short until they scroll back through — the trade
    /// [`follow_live`](Self::follow_live) explains.
    fn remeasure(&mut self) {
        let count = self.rows.len();
        if self.at_live() {
            self.list.reset(count);
        } else {
            let top = self.list.logical_scroll_top().item_ix.min(count);
            self.list.splice(0..top, top);
            self.list.splice(top..count, count - top);
        }
    }

    /// The frame every row shares.
    ///
    /// One column, not two. There used to be a fixed 34px timestamp gutter down
    /// the left holding a stamp per row, which in a busy channel meant fifteen
    /// consecutive `15:27`s standing in for a ruler — and stamping only the
    /// rows that said something new left the gutter empty for most of them,
    /// which is 11% of a 300px chat pane reserved for nothing. The time moved
    /// to [`time_break`] instead, where it is said once per minute and the rows
    /// get their width back.
    ///
    /// Its padding above and below follows the text size, so an emote's
    /// overhang always has room ([`Metrics::row_pad_y`]).
    fn row_frame(row: &Row, metrics: &Metrics) -> gpui::Div {
        let wash = row.kind.wash();
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .px(px(theme::ROW_PAD_X))
            .py(px(metrics.row_pad_y))
            // The stripe is a reading aid for a run of like rows; an event is
            // not one of those, and two washes on one row is mud.
            //
            // It used to be a stripe *and* a hairline under every row, which is
            // two separators doing one job. The stripe is the one that survives
            // a wrapped message: it fills the whole block, so three lines of one
            // message read as one thing rather than as three rows.
            .when(wash.is_none() && row.striped, |line| {
                line.bg(theme::stripe())
            })
            .when_some(wash, |line, color| line.bg(color))
    }

    /// The time, once, above the first message of each minute.
    ///
    /// A rule rather than a column: it says the same thing the gutter did — how
    /// long ago this was — using space that is empty anyway, and it reads as a
    /// break in the conversation, which a minute passing in a chat usually is.
    ///
    /// Not drawn while every message carries its own time (`row_time`), which
    /// says the same thing on every row; [`chat_display::draws_time_break`]
    /// is the rule.
    fn time_break(stamp: SharedString) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT))
            .px(px(theme::ROW_PAD_X))
            .pt(px(theme::GAP_TIGHT))
            .child(
                div()
                    .flex_none()
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_TIGHT))
                    .text_color(theme::text_dim())
                    .child(stamp),
            )
            .child(div().flex_1().h(px(1.)).bg(theme::divider()))
    }

    /// A row's own time, at its start, while every message carries one (the
    /// chat options menu's `Time on every message`): the time break's words
    /// without its rule, in the same dim meta style, written by the same
    /// clock (`clock`) so it reads `11:41 PM` or `23:41` as the breaks do.
    /// It does not scale with the text, as the breaks and notices do not:
    /// it is the row's supporting information, not what is read.
    fn row_time(stamp: SharedString) -> gpui::Div {
        div()
            .flex_none()
            .text_size(px(theme::TEXT_META))
            .text_color(theme::text_dim())
            .child(stamp)
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let metrics = Metrics::of(self.display.size);
        // The row's own time, when every row carries one.
        let time = self
            .display
            .times
            .then(|| Self::row_time(row.stamp.clone()));

        // Only when it says something the row above did not. Read from the rows
        // rather than from the list, which only knows about the ones on screen:
        // scrolling a stamp off the top must not make the row below it grow one.
        let previous = index
            .checked_sub(1)
            .map(|above| self.rows[above].stamp.as_ref());
        let stamped = chat_display::draws_time_break(previous, &row.stamp, self.display.times);

        let body = match &row.kind {
            // The time beside the words rather than in them: a notice is one
            // run of text, not a line of words to wrap with.
            RowKind::Notice(text) => Self::row_frame(row, &metrics)
                .when_some(time, |frame, time| {
                    frame.gap(px(theme::GAP_WORD)).child(time)
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(theme::TEXT_META))
                        .text_color(theme::text_dim())
                        .child(text.clone()),
                )
                .into_any_element(),

            RowKind::Message(message) => {
                let source = shared_chat::label(message.source_room.as_deref(), &self.rooms);
                let line = self.message_line(row, message, time, source, &metrics, cx);
                let frame = Self::row_frame(row, &metrics);
                match &message.reply {
                    None => frame.child(line),
                    // The line it answers above it, and the message in a
                    // row of its own beneath, as an event's note is: a
                    // wrapping line straight in a column is the trap
                    // `render_event` describes. The gap is an emote's
                    // overhang, so one on the message's first line does not
                    // paint over the line above.
                    Some(reply) => frame.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_y(px(metrics.overhang))
                            .child(Self::reply_line(reply))
                            .child(div().w_full().flex().flex_row().child(line)),
                    ),
                }
                .into_any_element()
            }

            RowKind::Event(notice) => self.render_event(row, notice, time, &metrics, cx),
        };

        // The right button anywhere on the row, its time break included,
        // opens its copy menu; see `copy_menu`.
        let menu = Self::row_menu(row.seq, cx);
        div()
            .w_full()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Right, menu)
            .when(stamped, |row_box| {
                row_box.child(Self::time_break(row.stamp.clone()))
            })
            .child(body)
            .into_any_element()
    }

    /// A Twitch event: the sentence Twitch wrote, and whatever the user
    /// attached to it.
    ///
    /// `system-msg` is finished English — "Foo subscribed with Prime.", "10
    /// raiders from Bar" — already assembled and already localised, so there is
    /// nothing to format and no `msg-id` to switch on. An announcement has no
    /// sentence at all and is nothing but body, which is why both halves are
    /// optional here.
    ///
    /// Its `time`, when every row carries one, starts its first line and
    /// nothing else: beside Twitch's sentence, as a notice's sits beside its
    /// words, or, for an announcement that is only a body, as the first item
    /// of that body's wrapping line, as a message's is. Not in a column of
    /// its own beside the whole event, where it would leave the note under
    /// the sentence narrower than the full row the piece budget is sized for
    /// (`chat_text::piece_chars`), so a long unbroken run in it could draw a
    /// piece wider than its line at the narrowest chat. The note is the same
    /// event, so it carries no time of its own.
    ///
    /// An event copied in from a Shared Chat partner's room carries its tag
    /// (`source_tag`) on that same first line, after the time: on Twitch's
    /// sentence, which is what would otherwise read as if it happened here,
    /// or on the body's line when there is no sentence. Once, and not again
    /// on the note, which is the same event.
    fn render_event(
        &self,
        row: &Row,
        notice: &ChatNotice,
        mut time: Option<gpui::Div>,
        metrics: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut column = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_y(px(theme::GAP_TIGHT));
        let mut source = shared_chat::label(notice.source_room.as_deref(), &self.rooms);

        // The time and the tag go to whichever line comes first: the
        // sentence's, or the body's when there is no sentence.
        if !notice.system.is_empty() {
            let tag = source.take().map(|label| self.source_tag(row, label));
            column = column.child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(theme::GAP_WORD))
                    .children(time.take())
                    .children(tag)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(theme::weight_label())
                            .text_color(theme::accent())
                            .child(SharedString::from(notice.system.clone())),
                    ),
            );
        }
        if let Some(body) = &notice.body {
            // Wrapped in a row rather than added straight to the column, so
            // this is laid out exactly the way an ordinary message is: a
            // wrapping line that is the only child of a flex *row*.
            //
            // Dropped into the column directly, it was a wrapping line inside a
            // flex *column*, and gpui sized it from its own content — which for
            // a line whose every word was then `min_w_0` was one character
            // wide, and is still only its widest piece (`render_word`). The
            // words then wrapped one per line and painted straight over the
            // rows beneath, so a resub with a note attached, or an announcement
            // carrying a link, came out as a vertical stack of letters. It only
            // ever showed on event rows, because they are the only place a
            // message line is not already a row's child.
            column = column.child(div().w_full().flex().flex_row().child(self.message_line(
                row,
                body,
                time.take(),
                source,
                metrics,
                cx,
            )));
        }

        Self::row_frame(row, metrics)
            .child(column)
            .into_any_element()
    }

    /// One word of message text, styled for whatever it turned out to be, as
    /// the pieces it is drawn in: one, for nearly every word.
    ///
    /// The punctuation around a word is rendered separately so a trailing comma
    /// is neither underlined nor sent to the browser, and it rides with the
    /// piece it touches, so it never wraps onto a line of its own.
    ///
    /// No piece shrinks. A word longer than the piece budget (sixteen
    /// characters at body size, [`chat_text::PIECE_CHARS`], and fewer at a
    /// larger text size, [`chat_text::piece_chars`]) — a long URL, in
    /// practice, or a wall of one letter — is drawn as several pieces edge to
    /// edge ([`chat_text::pieces`]), each short enough to fit any chat
    /// on a line of its own, and `flex_wrap` breaks the line between them; a
    /// link breaks just after its separators, the way it reads, and each piece
    /// opens it. Every piece after the first takes back the line's gap between
    /// words (`theme::GAP_WORD`) with a margin, so the word reads as one; where
    /// a piece starts a line, that margin is in the row's padding.
    ///
    /// It used to be one element per word that could shrink below its width
    /// (`min_w_0`), leaving gpui to wrap a link wider than the pane inside
    /// itself. gpui measured those rows wrong: its text element hands back the
    /// size from its last measurement whenever it is asked for its natural
    /// width (`elements/text.rs`), and a word that is both measured whole and
    /// measured squeezed in one layout pass got the wrong one for one of them.
    /// The row came out shorter than it drew, and painted over the next
    /// message, or taller, with a blank line in it. A word that always fits
    /// measures the same both ways.
    ///
    /// `piece_chars` is the budget at the chat's text size
    /// ([`Metrics::piece_chars`]): fewer characters as the glyphs grow, so a
    /// piece fits the narrowest chat at every size.
    fn render_word(
        &self,
        word: &str,
        seq: u64,
        position: usize,
        text_color: gpui::Hsla,
        piece_chars: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let parsed = chat_text::classify(word);
        // A plain word keeps its punctuation in its text: there is nothing to
        // keep it out of, and one element is cheaper than three.
        let (leading, body, trailing) = match parsed.kind {
            Kind::Plain => ("", word, ""),
            Kind::Link | Kind::Mention => (parsed.leading, parsed.body, parsed.trailing),
        };
        let url = (parsed.kind == Kind::Link).then(|| parsed.url());
        // Drawn in the colour of whoever is being talked to, when we have seen
        // them speak. Otherwise left alone: a wrong colour is worse than none.
        let mention_color = parsed
            .mentioned()
            .and_then(|login| self.colors.get(&login.to_ascii_lowercase()).copied())
            .map(theme::readable)
            .unwrap_or(text_color);

        let punctuation = |text: &str| {
            div()
                .flex_none()
                .text_color(text_color)
                .child(SharedString::from(text.to_string()))
        };

        let pieces = chat_text::pieces(body, piece_chars);
        let last = pieces.len() - 1;
        pieces
            .into_iter()
            .enumerate()
            .map(|(index, piece)| {
                let text = SharedString::from(piece.to_string());
                let styled = match (&url, parsed.kind) {
                    (Some(url), _) => {
                        // The right button says which link it was, for the
                        // row's copy menu.
                        let pressed = Self::press_target(Target::Link(url.clone()), cx);
                        let url = url.clone();
                        div()
                            .id(SharedString::from(format!(
                                "chat-link-{seq}-{position}-{index}"
                            )))
                            .flex_none()
                            .text_color(theme::accent())
                            .underline()
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme::text()))
                            .child(text)
                            .on_mouse_down(MouseButton::Right, pressed)
                            .on_click(cx.listener(move |_, _event, _window, cx| cx.open_url(&url)))
                            .into_any_element()
                    }
                    (None, Kind::Mention) => div()
                        .flex_none()
                        .font_weight(theme::weight_title())
                        .text_color(mention_color)
                        .child(text)
                        .into_any_element(),
                    (None, _) => div()
                        .flex_none()
                        .text_color(text_color)
                        .child(text)
                        .into_any_element(),
                };
                let before = (index == 0 && !leading.is_empty()).then_some(leading);
                let after = (index == last && !trailing.is_empty()).then_some(trailing);
                let element = if before.is_none() && after.is_none() {
                    styled
                } else {
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_baseline()
                        .children(before.map(punctuation))
                        .child(styled)
                        .children(after.map(punctuation))
                        .into_any_element()
                };
                if index == 0 {
                    element
                } else {
                    div()
                        .flex_none()
                        .ml(px(-theme::GAP_WORD))
                        .child(element)
                        .into_any_element()
                }
            })
            .collect()
    }

    /// The quiet tag naming the Shared Chat partner a row was copied from
    /// (`controls::quiet_tag`), `Said in <name>'s chat` under the pointer
    /// while the pointer is in the window (`window_hovered`). Like the time
    /// it is supporting information in the meta size, and does not grow with
    /// the text. It is one unbroken word, a channel's name of at most 25
    /// characters, which fits the narrowest chat on a line of its own as a
    /// speaker's name does.
    fn source_tag(&self, row: &Row, label: &Label) -> gpui::Stateful<gpui::Div> {
        controls::quiet_tag(label.name.clone())
            .id(SharedString::from(format!("chat-source-{}", row.seq)))
            .when(self.window_hovered, |tag| {
                tag.tooltip(controls::tip(label.tooltip.clone()))
            })
    }

    /// One message as a wrapping line of name, words and emotes — without the
    /// row frame around it.
    ///
    /// Split from the frame because a USERNOTICE renders one of these beneath
    /// Twitch's own sentence: a resub note is a message like any other and has
    /// to get the same emotes, links and mention colouring as anything else
    /// that person says.
    ///
    /// `time`, when every message carries one, is the line's first child,
    /// before the name: in the line rather than in a column beside it, so a
    /// message that wraps comes back to the row's edge and the time costs
    /// only its own width on the first line, which is what the gutter this
    /// replaced could not do (see `row_frame`). Sizes come from `metrics`,
    /// the chat's text size.
    ///
    /// `source`, for a line copied in from a Shared Chat partner's room, is
    /// that partner's label (`shared_chat::label` decides whether and what),
    /// drawn after the time and before the name as a tag
    /// ([`source_tag`](Self::source_tag)). The caller decides, because an
    /// event's note leaves it to the event's first line (`render_event`).
    ///
    /// The speaker's badges come next, before the name
    /// ([`badge_group`](Self::badge_group)). A reply's text starts with the
    /// `@parent` Twitch puts there, which the line above it already says, so
    /// that first word is not drawn (`reply::skips_leading_mention`); it is dropped here
    /// rather than from the text, because the `emotes` tag counts from the
    /// text's start. Someone's first message in the channel ends with a
    /// quiet `First message` tag, which says why the row is tinted.
    fn message_line(
        &self,
        row: &Row,
        message: &ChatMessage,
        time: Option<gpui::Div>,
        source: Option<&Label>,
        metrics: &Metrics,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // A deleted message keeps its place and loses its colour: greyed all
        // through, name included, so it reads as struck rather than as said.
        let name_color = if row.deleted {
            theme::text_dim()
        } else {
            theme::readable(message.color)
        };
        // An action is written in the speaker's colour; a normal message is
        // not, or a chat of many voices becomes a chat of many colours.
        let text_color: gpui::Hsla = if message.is_action || row.deleted {
            name_color
        } else {
            theme::text()
        };

        // Wrapping happens between children, so every word is its own child.
        let mut line = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_x(px(theme::GAP_WORD))
            .children(time)
            .children(source.map(|label| self.source_tag(row, label)))
            .children(self.badge_group(row, message, metrics));

        // An action puts the name inside the sentence, so it is not repeated.
        let name = if message.is_action {
            message.display_name.clone()
        } else {
            format!("{}:", message.display_name)
        };
        line = line.child(
            div()
                .flex_none()
                .font_weight(theme::weight_title())
                .text_color(name_color)
                .child(SharedString::from(name)),
        );

        // Twitch's own emotes come from the tag with exact positions; the rest
        // are name lookups applied to whatever text is left.
        let tokens = tokenize(&message.text, message.emotes.as_deref(), true);
        let sets = self.emote_sets.clone();
        let tokens = apply_named_emotes(tokens, &move |name| sets.lookup(name));

        // Only a message that actually has an emote in it pays for the room one
        // needs. A wrapped wall of plain text keeps its tight leading, which is
        // most of what wraps.
        let overhangs = tokens.iter().any(|token| matches!(token, Token::Emote(_)));
        line = line.when(overhangs, |line| line.gap_y(px(metrics.overhang * 2.0)));

        let mut emote_index = 0usize;
        let mut word_index = 0usize;
        // Whether the first word is the `@parent` the reply line says.
        let mut skip_first = reply::skips_leading_mention(message.reply.as_ref(), &tokens);
        for token in tokens {
            match token {
                Token::Text(text) => {
                    for word in text.split_whitespace() {
                        if std::mem::take(&mut skip_first) {
                            continue;
                        }
                        line = line.children(self.render_word(
                            word,
                            row.seq,
                            word_index,
                            text_color,
                            metrics.piece_chars,
                            cx,
                        ));
                        word_index += 1;
                    }
                }
                Token::Emote(emote) => {
                    let resolved = self.cache.get_or_request(&emote.url);
                    // The right button says which emote it was, for the
                    // row's copy menu, picture or not.
                    let pressed = Self::press_target(Target::Emote(emote.name.clone()), cx);
                    line = line.child(match resolved {
                        // Until the image lands, show the emote's name so the
                        // message still reads correctly.
                        None => div()
                            .text_color(theme::text_dim())
                            .on_mouse_down(MouseButton::Right, pressed)
                            .child(SharedString::from(emote.name))
                            .into_any_element(),
                        // The id is what makes animated emotes animate: GPUI
                        // keys per-frame state on an element's global id, and
                        // only requests an animation frame when one exists. An
                        // img without an id is pinned to frame 0 forever.
                        //
                        // The id must identify the *image*, not the slot. Keyed
                        // on position alone, a 40-frame GIF in one row and a
                        // 1-frame PNG in another share state, and GPUI indexes
                        // the PNG with the GIF's frame number and panics.
                        Some(path) => {
                            let name = SharedString::from(emote.name.clone());
                            // Emotes are taller than the text they sit in. Give
                            // the wrapper a line's worth of height and let the
                            // image overhang it, so a row with emotes is no
                            // taller than one without and the list keeps a
                            // single vertical rhythm to scan down.
                            //
                            // The right button listens on the picture, not the
                            // wrapper, for the same reason the tooltip does:
                            // the picture overhangs the wrapper above and
                            // below, and a press on the overhang is on the
                            // emote as much as one in the middle.
                            div()
                                .flex_none()
                                .h(px(metrics.line))
                                .px(px(theme::EMOTE_PAD_X))
                                .child(
                                    img(path)
                                        // Before `.id(..)`: `image_cache` is on
                                        // `Img`, and `.id(..)` yields a
                                        // `Stateful<Img>`.
                                        .image_cache(&self.emote_images)
                                        .id((SharedString::from(emote.url.clone()), emote_index))
                                        .h(px(metrics.emote))
                                        .mt(px(-metrics.overhang))
                                        .on_mouse_down(MouseButton::Right, pressed)
                                        .tooltip(move |_window, cx| {
                                            cx.new(|_| EmoteTooltip { name: name.clone() }).into()
                                        }),
                                )
                                .into_any_element()
                        }
                    });
                    emote_index += 1;
                }
            }
        }

        if message.first {
            line = line.child(controls::quiet_tag("First message"));
        }
        if row.deleted {
            line = line.child(controls::tag("deleted"));
        }

        line
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();

        // Where the pointer is, against where the list was last laid out.
        // Before `at_live` is read: releasing held rows changes it.
        // Not while the pointer is on the guide over the chat, which would
        // otherwise hold its rows for as long as the guide was being read;
        // see `crate::veil`.
        self.window_hovered = window.is_window_hovered();
        let over = veil::pointer_over(self.list.viewport_bounds(), window, cx);
        self.sync_hold(over);
        let at_live = self.at_live();
        let holding = !self.held.is_empty();
        let metrics = Metrics::of(self.display.size);
        let modes = self.modes.as_ref().and_then(chat_words::modes_line);

        // The list and everything laid over it. In a box of its own above the
        // line of modes, so the pills at its foot sit over the messages and
        // never over that line.
        let body = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                list(self.list.clone(), move |index, _window, cx| {
                    this.update(cx, |this: &mut ChatView, cx| this.render_row(index, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .size_full(),
            )
            .child(
                // Says how far back you are, which is the question a held
                // position raises and nothing else on screen answers. Kept up
                // for as long as you are held back, and out of the way while
                // the pane is following live — the default fades after a
                // moment, which is exactly when you still want to see it.
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::vertical(&self.list).scrollbar_show(if at_live {
                        ScrollbarShow::Hover
                    } else {
                        ScrollbarShow::Always
                    })),
            )
            .when(self.rows.is_empty(), |pane| {
                let caption = div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_dim());
                // A replay whose buffer is ready and empty is not loading: it
                // is a quiet stretch of the broadcast, or its start, and the
                // next message is on its way at the recording's own pace.
                // Said plainly, and still, because that can last a while.
                if self.replay && self.loaded {
                    pane.child(caption.child("nothing said here yet"))
                } else {
                    // Safe to pulse: this state always ends, either at the
                    // join or at the first disconnect notice, and both put
                    // a row in the list — or, on a replay, at the buffer
                    // being ready, which takes this branch away.
                    pane.child(caption.child(motion::waiting(
                        "chat-connecting",
                        div().child(self.waiting.clone()),
                    )))
                }
            })
            // Rows are waiting on the pointer. Said quietly, where the
            // jump-to-live pill goes — which is empty whenever this is true,
            // since rows are only held while the pane is following live.
            .when(holding, |pane| {
                pane.child(
                    div()
                        .absolute()
                        .bottom(px(theme::GAP))
                        .left_0()
                        .right_0()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .child(controls::waiting("Chat paused")),
                )
            })
            .when(!at_live, |pane| {
                pane.child(
                    div()
                        .absolute()
                        .bottom(px(theme::GAP))
                        .left_0()
                        .right_0()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .child(
                            controls::pill(
                                "chat-follow-live",
                                // Nothing about a replay is live; the
                                // bottom of its pane is what has been said
                                // so far.
                                if self.replay {
                                    "↓ Newest"
                                } else {
                                    "↓ Jump to live"
                                },
                                controls::Variant::Primary,
                            )
                            // Sits over messages, so it has to swallow the
                            // click rather than pass it to a link beneath.
                            .block_mouse_except_scroll()
                            .shadow_lg()
                            .on_click(
                                cx.listener(|this, _event, _window, cx| this.follow_live(cx)),
                            ),
                        ),
                )
            });

        div()
            .id("chat-pane")
            // Only to wake a repaint when the pointer arrives or leaves; the
            // value is not trusted, for the reasons `VideoView::hovered`
            // gives. Without it a quiet channel would not notice the pointer
            // going until the next message did.
            .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
            .size_full()
            .flex()
            .flex_col()
            // The chat's text size, which every row inherits; see
            // `chat_display`.
            .text_size(px(metrics.text))
            .line_height(px(metrics.line))
            .child(body)
            .when_some(modes, |pane, modes| pane.child(Self::modes_line(modes)))
            .children(self.composer_bar(window, cx))
            .children(self.copy_menu(cx))
    }
}

#[cfg(test)]
mod tests {
    use emotes::{tokenize, Token};
    use twitch_chat::replay::parse_page;

    /// The tag a replay builds is read by the tokenizer the live pane uses,
    /// and the two have to agree on what an index counts: characters. The
    /// positions Twitch sends in the same answer count bytes, and would put
    /// `Kappa` three characters late after an emoji.
    #[test]
    fn a_replayed_emote_lands_where_the_tokenizer_looks() {
        let body = serde_json::json!([{ "data": { "video": { "id": "1", "comments": {
            "edges": [{ "cursor": "c", "node": {
                "id": "x", "contentOffsetSeconds": 7,
                "commenter": { "id": "1", "login": "fisher", "displayName": "Fisher" },
                "createdAt": "2026-09-08T14:01:56Z",
                "message": { "fragments": [
                    { "emote": null, "text": "🎣 " },
                    { "emote": { "id": "25;5;9", "emoteID": "25", "from": 5 }, "text": "Kappa" },
                    { "emote": null, "text": " nice" }
                ], "userColor": null }
            }}],
            "pageInfo": { "hasNextPage": false }
        }}}}]);
        let page = parse_page(&body).unwrap();
        let message = &page.comments[0].message;
        let tokens = tokenize(&message.text, message.emotes.as_deref(), false);
        let names: Vec<String> = tokens
            .iter()
            .map(|token| match token {
                Token::Text(text) => format!("T:{text}"),
                Token::Emote(emote) => format!("E:{}", emote.name),
            })
            .collect();
        assert_eq!(names, ["T:🎣 ", "E:Kappa", "T: nice"]);
        assert!(matches!(&tokens[1], Token::Emote(emote) if emote.url.contains("/25/")));
    }
}
