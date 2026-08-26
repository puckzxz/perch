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
    div, img, list, prelude::*, px, AnyElement, Context, ListAlignment, ListState, SharedString,
    Task, Window,
};

use twitch_chat::{ChatClient, ChatEvent, ChatMessage};

use crate::chat_text::{self, Kind};
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
            .rounded_sm()
            .bg(theme::surface_raised())
            .text_size(px(theme::TEXT_LABEL))
            .text_color(theme::text())
            .child(self.name.clone())
    }
}

/// Messages kept in memory. Old ones drop off the top: chat runs forever, and
/// nobody scrolls back a thousand lines in a live stream.
const MAX_MESSAGES: usize = 500;

/// Rendered emote height. Twitch's 2.0 assets are around 56px, so this halves
/// them and keeps them crisp on a HiDPI display.
///
/// An emote overhangs its line rather than growing it — see the wrapper in
/// `render_message` — so at this height it comes within about half a pixel of
/// the hairline above and below. That is deliberate: shrinking the emote to
/// buy clearance costs more than the crowding does.
const EMOTE_HEIGHT: f32 = 28.0;

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
    Notice(SharedString),
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
    /// Name lookups for FFZ / BTTV / 7TV emotes, filled in as they load.
    emote_sets: EmoteSets,
    emote_loader: EmoteLoader,
    _client: ChatClient,
    _pump: Task<()>,
    _emote_pump: Task<()>,
}

impl ChatView {
    pub fn new(
        channel: String,
        cache: Arc<ImageCache>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (client, mut events) = ChatClient::connect(&channel);

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

        let mut view = Self {
            rows: Vec::new(),
            colors: std::collections::HashMap::new(),
            striped: false,
            next_seq: 0,
            list: ListState::new(0, ListAlignment::Bottom, px(400.)),
            cache,
            emote_sets,
            emote_loader,
            _client: client,
            _pump: pump,
            _emote_pump: emote_pump,
        };
        view.push(
            RowKind::Notice(format!("connecting to #{channel}…").into()),
            None,
        );
        view
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

    /// The frame every row shares: a fixed timestamp column, then the content.
    ///
    /// Two columns rather than one wrapping row, so the stamps line up as a
    /// ruler. Added as the first child of a `flex_wrap` row they would re-flow
    /// with the text and give no column at all.
    fn row_frame(row: &Row) -> gpui::Div {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .gap_x(px(theme::GAP_TIGHT))
            .px(px(theme::ROW_PAD_X))
            .py(px(theme::ROW_PAD_Y))
            .border_b_1()
            .border_color(theme::divider())
            .when(row.striped, |line| line.bg(theme::stripe()))
            .child(
                div()
                    .flex_none()
                    .w(px(theme::STAMP_WIDTH))
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_BODY))
                    .text_color(theme::text_dim())
                    .child(row.stamp.clone()),
            )
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };

        match &row.kind {
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

            RowKind::Message(message) => self.render_message(row, message, cx),
        }
    }

    /// One word of message text, styled for whatever it turned out to be.
    ///
    /// The punctuation around a word is rendered separately so a trailing comma
    /// is neither underlined nor sent to the browser. Everything is wrapped in
    /// one `flex_none` row so a word never wraps in the middle of itself.
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
                    .text_color(text_color)
                    .child(SharedString::from(word.to_string()))
                    .into_any_element();
            }
            Kind::Link => {
                let url = parsed.url();
                div()
                    .id(SharedString::from(format!("chat-link-{seq}-{position}")))
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
                    .font_weight(theme::weight_title())
                    .text_color(color)
                    .child(SharedString::from(parsed.body.to_string()))
                    .into_any_element()
            }
        };

        let punctuation = |text: &str| {
            div()
                .text_color(text_color)
                .child(SharedString::from(text.to_string()))
        };

        div()
            .flex_none()
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

    fn render_message(
        &self,
        row: &Row,
        message: &ChatMessage,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                                        .id((SharedString::from(emote.url.clone()), emote_index))
                                        .h(px(EMOTE_HEIGHT))
                                        .mt(px((theme::LINE_BODY - EMOTE_HEIGHT) / 2.0))
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

        Self::row_frame(row).child(line).into_any_element()
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();

        div()
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
    }
}
