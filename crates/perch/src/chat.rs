//! The chat pane.
//!
//! Uses a bottom-anchored `ListState`, which is what keeps new messages pinned
//! to the bottom the way every chat client does, without manual scrolling.
//!
//! Emotes are the interesting part. GPUI has no inline-image-in-text, so a
//! message cannot be one wrapped paragraph with pictures in it. Instead each
//! message is tokenised into words and emotes and laid out in a `flex_wrap`
//! row: wrapping then happens at token boundaries, which is where you want it.

use std::sync::Arc;

use emotes::{apply_named_emotes, tokenize, EmoteLoader, EmoteSets, ImageCache, Token};
use gpui::{
    div, img, list, point, prelude::*, px, AnyElement, Context, Entity, ListAlignment, ListState,
    Pixels, RetainAllImageCache, SharedString, Subscription, Task, Window,
};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};

use twitch_chat::{ChatClient, ChatEvent, ChatMessage, ChatNotice, NoticeKind};

use crate::chat_text::{self, Kind};
use crate::controls;
use crate::motion;
use crate::theme;

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

/// Rendered emote height. Twitch's 2.0 assets are around 56px, so this halves
/// them and keeps them crisp on a HiDPI display.
///
/// An emote overhangs its line rather than growing it — see the wrapper in
/// `message_line` — so at this height it comes within about half a pixel of
/// the hairline above and below. That is deliberate: shrinking the emote to
/// buy clearance costs more than the crowding does.
const EMOTE_HEIGHT: f32 = 28.0;

/// How far an emote hangs past its line, top and bottom.
///
/// At the top and bottom of a row this is absorbed by `ROW_PAD_Y`. *Between*
/// two wrapped lines of the same message there is no padding at all — the lines
/// sit exactly `LINE_BODY` apart — so an emote on the second line paints over
/// the descenders of the first, and an emote on the first is painted over in
/// turn. This is the gap that gives the overhang somewhere to go.
const EMOTE_OVERHANG: f32 = (EMOTE_HEIGHT - theme::LINE_BODY) / 2.0;

/// Wall-clock time for a row, formatted once on arrival.
///
/// Twitch's `tmi-sent-ts` is when the *server* saw the message, which is what
/// you want: it stays correct for a backlog and does not drift with our own
/// processing. Falls back to now for rows we generate ourselves.
fn clock(sent_at: Option<u64>) -> SharedString {
    let when = sent_at
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms as i64))
        .unwrap_or_else(chrono::Utc::now);
    SharedString::from(
        when.with_timezone(&chrono::Local)
            .format("%H:%M")
            .to_string(),
    )
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
    /// The wash behind this row, if it needs one.
    ///
    /// Events are washed rather than outlined or barred, because the row has to
    /// keep its place in the ruler of timestamps down the side. Two intensities
    /// and no more: enumerating Twitch's `msg-id` values is a losing game, and
    /// the sentence in the row already says which event it was.
    fn wash(&self) -> Option<gpui::Hsla> {
        match self {
            RowKind::Event(notice) => Some(match notice.kind {
                NoticeKind::Raid | NoticeKind::Announcement => theme::event_wash_loud(),
                _ => theme::event_wash(),
            }),
            _ => None,
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
}

pub struct ChatView {
    rows: Vec<Row>,
    /// What to say while the pane is still empty. Formatted once.
    channel: SharedString,
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
    _client: ChatClient,
    _pump: Task<()>,
    _emote_pump: Task<()>,
}

impl ChatView {
    /// `history` is how many messages from before now to open with; zero joins
    /// an empty pane. See [`twitch_chat::history`] for where they come from and
    /// what asking costs.
    pub fn new(
        channel: String,
        history: usize,
        cache: Arc<ImageCache>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (client, mut events) = ChatClient::connect(&channel, history);

        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this: &mut ChatView, cx| {
                        this.apply(event);
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
            channel: SharedString::from(format!("connecting to #{channel}…")),
            colors: std::collections::HashMap::new(),
            striped: false,
            next_seq: 0,
            list,
            cache,
            emote_images,
            _release: release,
            emote_sets,
            emote_loader,
            _client: client,
            _pump: pump,
            _emote_pump: emote_pump,
        }
    }

    fn apply(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Connected { channel } => {
                self.push(RowKind::Notice(format!("joined #{channel}").into()), None)
            }
            ChatEvent::RoomState { room_id } => self.emote_loader.load_channel(room_id),
            ChatEvent::Message(message) => {
                let sent_at = message.sent_at;
                self.colors.insert(message.login.clone(), message.color);
                self.push(RowKind::Message(message), sent_at)
            }
            ChatEvent::Notice(notice) => {
                let sent_at = notice.sent_at;
                // A resub note counts as having spoken, so an `@them` later in
                // the conversation still finds their colour.
                if let Some(body) = &notice.body {
                    self.colors.insert(body.login.clone(), body.color);
                }
                self.push(RowKind::Event(notice), sent_at)
            }
            ChatEvent::Cleared { login } => {
                let text = match login {
                    Some(who) => format!("{who} was timed out or banned"),
                    None => "chat was cleared".to_string(),
                };
                self.push(RowKind::Notice(text.into()), None);
            }
            ChatEvent::Disconnected { reason } => self.push(
                RowKind::Notice(format!("disconnected: {reason} — retrying").into()),
                None,
            ),
        }
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
        });
        let count = self.rows.len();
        self.list.splice(count - 1..count - 1, 1);

        if self.rows.len() > MAX_MESSAGES {
            let excess = self.rows.len() - MAX_MESSAGES;
            self.rows.drain(0..excess);
            self.list.splice(0..excess, 0);
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
    fn row_frame(row: &Row) -> gpui::Div {
        let wash = row.kind.wash();
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .px(px(theme::ROW_PAD_X))
            .py(px(theme::ROW_PAD_Y))
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

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };

        // Only when it says something the row above did not. Read from the rows
        // rather than from the list, which only knows about the ones on screen:
        // scrolling a stamp off the top must not make the row below it grow one.
        let stamped = index == 0 || self.rows[index - 1].stamp != row.stamp;

        let body = match &row.kind {
            RowKind::Notice(text) => Self::row_frame(row)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(theme::TEXT_META))
                        .text_color(theme::text_dim())
                        .child(text.clone()),
                )
                .into_any_element(),

            RowKind::Message(message) => Self::row_frame(row)
                .child(self.message_line(row, message, cx))
                .into_any_element(),

            RowKind::Event(notice) => self.render_event(row, notice, cx),
        };

        if !stamped {
            return body;
        }
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(Self::time_break(row.stamp.clone()))
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
    fn render_event(&self, row: &Row, notice: &ChatNotice, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_y(px(theme::GAP_TIGHT));

        if !notice.system.is_empty() {
            column = column.child(
                div()
                    .font_weight(theme::weight_label())
                    .text_color(theme::accent())
                    .child(SharedString::from(notice.system.clone())),
            );
        }
        if let Some(body) = &notice.body {
            // Wrapped in a row rather than added straight to the column, so
            // this is laid out exactly the way an ordinary message is: a
            // wrapping line that is the only child of a flex *row*.
            //
            // Dropped into the column directly, it was a wrapping line inside a
            // flex *column*, and gpui sized it from its own content — which for
            // a line whose every word is `min_w_0` is one character wide. The
            // words then wrapped one per line and painted straight over the
            // rows beneath, so a resub with a note attached, or an announcement
            // carrying a link, came out as a vertical stack of letters. It only
            // ever showed on event rows, because they are the only place a
            // message line is not already a row's child.
            column = column.child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .child(self.message_line(row, body, cx)),
            );
        }

        Self::row_frame(row).child(column).into_any_element()
    }

    /// One word of message text, styled for whatever it turned out to be.
    ///
    /// The punctuation around a word is rendered separately so a trailing comma
    /// is neither underlined nor sent to the browser.
    ///
    /// Every word here can shrink below its content width, which sounds like it
    /// would break words in half and does not: `flex_wrap` moves a word to the
    /// next line long before it would have to shrink, so shrinking only ever
    /// happens to a word that is wider than the *whole* pane. That is a long
    /// URL, in practice, and the alternative is the one this replaced — a link
    /// that simply runs off the edge of the chat, unreadable and unclickable
    /// past the boundary.
    ///
    /// Breaking one is gpui's job and it is better at it than a character cap
    /// would be: `/` is not a word character, so a URL breaks at its path
    /// separators, and a run with no break opportunity at all — an opaque media
    /// id — is hard-broken at the edge rather than overflowing.
    fn render_word(
        &self,
        word: &str,
        seq: u64,
        position: usize,
        text_color: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let parsed = chat_text::classify(word);

        let styled = match parsed.kind {
            Kind::Plain => {
                // The common case, and worth keeping as one element rather than
                // three: most words have no punctuation to split off.
                return div()
                    .min_w_0()
                    .text_color(text_color)
                    .child(SharedString::from(word.to_string()))
                    .into_any_element();
            }
            Kind::Link => {
                let url = parsed.url();
                div()
                    .id(SharedString::from(format!("chat-link-{seq}-{position}")))
                    .min_w_0()
                    .text_color(theme::accent())
                    .underline()
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::text()))
                    .child(SharedString::from(parsed.body.to_string()))
                    .on_click(cx.listener(move |_, _event, _window, cx| cx.open_url(&url)))
                    .into_any_element()
            }
            Kind::Mention => {
                // Drawn in the colour of whoever is being talked to, when we
                // have seen them speak. Otherwise left alone: a wrong colour is
                // worse than none.
                let color = parsed
                    .mentioned()
                    .and_then(|login| self.colors.get(&login.to_ascii_lowercase()).copied())
                    .map(theme::readable)
                    .unwrap_or(text_color);
                div()
                    .min_w_0()
                    .font_weight(theme::weight_title())
                    .text_color(color)
                    .child(SharedString::from(parsed.body.to_string()))
                    .into_any_element()
            }
        };

        // Pinned, so a shrinking row takes it out of the word rather than out
        // of the comma after it.
        let punctuation = |text: &str| {
            div()
                .flex_none()
                .text_color(text_color)
                .child(SharedString::from(text.to_string()))
        };

        div()
            .min_w_0()
            .flex()
            .flex_row()
            .items_baseline()
            .when(!parsed.leading.is_empty(), |row| {
                row.child(punctuation(parsed.leading))
            })
            .child(styled)
            .when(!parsed.trailing.is_empty(), |row| {
                row.child(punctuation(parsed.trailing))
            })
            .into_any_element()
    }

    /// One message as a wrapping line of name, words and emotes — without the
    /// row frame around it.
    ///
    /// Split from the frame because a USERNOTICE renders one of these beneath
    /// Twitch's own sentence: a resub note is a message like any other and has
    /// to get the same emotes, links and mention colouring as anything else
    /// that person says.
    fn message_line(&self, row: &Row, message: &ChatMessage, cx: &mut Context<Self>) -> gpui::Div {
        let name_color = theme::readable(message.color);
        // An action is written in the speaker's colour; a normal message is
        // not, or a chat of many voices becomes a chat of many colours.
        let text_color: gpui::Hsla = if message.is_action {
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
            .gap_x(px(theme::GAP_WORD));

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
        line = line.when(overhangs, |line| line.gap_y(px(EMOTE_OVERHANG * 2.0)));

        let mut emote_index = 0usize;
        let mut word_index = 0usize;
        for token in tokens {
            match token {
                Token::Text(text) => {
                    for word in text.split_whitespace() {
                        line =
                            line.child(self.render_word(word, row.seq, word_index, text_color, cx));
                        word_index += 1;
                    }
                }
                Token::Emote(emote) => {
                    let resolved = self.cache.get_or_request(&emote.url);
                    line = line.child(match resolved {
                        // Until the image lands, show the emote's name so the
                        // message still reads correctly.
                        None => div()
                            .text_color(theme::text_dim())
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
                            div()
                                .flex_none()
                                .h(px(theme::LINE_BODY))
                                .px(px(theme::EMOTE_PAD_X))
                                .child(
                                    img(path)
                                        // Before `.id(..)`: `image_cache` is on
                                        // `Img`, and `.id(..)` yields a
                                        // `Stateful<Img>`.
                                        .image_cache(&self.emote_images)
                                        .id((SharedString::from(emote.url.clone()), emote_index))
                                        .h(px(EMOTE_HEIGHT))
                                        .mt(px(-EMOTE_OVERHANG))
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

        line
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();

        let at_live = self.at_live();

        div()
            .relative()
            .size_full()
            .text_size(px(theme::TEXT_BODY))
            .line_height(px(theme::LINE_BODY))
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
                pane.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(theme::TEXT_META))
                        .text_color(theme::text_dim())
                        // Safe to pulse: this state always ends, either at the
                        // join or at the first disconnect notice, and both put
                        // a row in the list.
                        .child(motion::waiting(
                            "chat-connecting",
                            div().child(self.channel.clone()),
                        )),
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
                                "↓ jump to live",
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
            })
    }
}
