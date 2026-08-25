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
    div, img, list, prelude::*, px, rgb, AnyElement, Context, ListAlignment, ListState,
    SharedString, Task, Window,
};

/// Emote names are worth showing on hover: half of chat is emotes, and knowing
/// what one is called is the difference between reading a message and guessing.
struct EmoteTooltip {
    name: SharedString,
}

impl Render for EmoteTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(0x241d38))
            .text_xs()
            .text_color(rgb(0xf2eff7))
            .child(self.name.clone())
    }
}
use twitch_chat::{ChatClient, ChatEvent, ChatMessage};

/// Messages kept in memory. Old ones drop off the top: chat runs forever, and
/// nobody scrolls back a thousand lines in a live stream.
const MAX_MESSAGES: usize = 500;

/// Rendered emote height. Twitch's 2.0 assets are around 56px, so this halves
/// them and keeps them crisp on a HiDPI display.
const EMOTE_HEIGHT: f32 = 28.0;

#[derive(Clone)]
enum Row {
    Message(Box<ChatMessage>),
    Notice(SharedString),
}

pub struct ChatView {
    rows: Vec<Row>,
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
            list: ListState::new(0, ListAlignment::Bottom, px(400.)),
            cache,
            emote_sets,
            emote_loader,
            _client: client,
            _pump: pump,
            _emote_pump: emote_pump,
        };
        view.push(Row::Notice(format!("connecting to #{channel}…").into()));
        view
    }

    fn apply(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Connected { channel } => {
                self.push(Row::Notice(format!("joined #{channel}").into()))
            }
            ChatEvent::RoomState { room_id } => self.emote_loader.load_channel(room_id),
            ChatEvent::Message(message) => self.push(Row::Message(message)),
            ChatEvent::Cleared { login } => {
                let text = match login {
                    Some(who) => format!("{who} was timed out or banned"),
                    None => "chat was cleared".to_string(),
                };
                self.push(Row::Notice(text.into()));
            }
            ChatEvent::Disconnected { reason } => {
                self.push(Row::Notice(format!("disconnected: {reason} — retrying").into()))
            }
        }
    }

    /// Append one row, trimming the backlog, keeping `ListState` in step.
    ///
    /// `ListState` tracks item count separately from our Vec, so every mutation
    /// here needs a matching splice or the list renders the wrong indices.
    fn push(&mut self, row: Row) {
        self.rows.push(row);
        let count = self.rows.len();
        self.list.splice(count - 1..count - 1, 1);

        if self.rows.len() > MAX_MESSAGES {
            let excess = self.rows.len() - MAX_MESSAGES;
            self.rows.drain(0..excess);
            self.list.splice(0..excess, 0);
        }
    }

    fn render_row(&self, index: usize) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };

        match row {
            Row::Notice(text) => div()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(rgb(0x6b6478))
                .child(text.clone())
                .into_any_element(),

            Row::Message(message) => self.render_message(message),
        }
    }

    fn render_message(&self, message: &ChatMessage) -> AnyElement {
        let text_color = if message.is_action {
            rgb(message.color)
        } else {
            rgb(0xd8d3e0)
        };

        // Wrapping happens between children, so every word is its own child.
        let mut line = div()
            .w_full()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_x_1()
            .px_3()
            .py_0p5();

        // An action puts the name inside the sentence, so it is not repeated.
        if !message.is_action {
            line = line.child(
                div()
                    .flex_none()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(message.color))
                    .child(SharedString::from(format!("{}:", message.display_name))),
            );
        } else {
            line = line.child(
                div()
                    .flex_none()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(message.color))
                    .child(SharedString::from(message.display_name.clone())),
            );
        }

        // Twitch's own emotes come from the tag with exact positions; the rest
        // are name lookups applied to whatever text is left.
        let tokens = tokenize(&message.text, message.emotes.as_deref(), true);
        let sets = self.emote_sets.clone();
        let tokens = apply_named_emotes(tokens, &move |name| sets.lookup(name));

        let mut emote_index = 0usize;
        for token in tokens {
            match token {
                Token::Text(text) => {
                    for word in text.split_whitespace() {
                        line = line.child(
                            div()
                                .text_color(text_color)
                                .child(SharedString::from(word.to_string())),
                        );
                    }
                }
                Token::Emote(emote) => {
                    let resolved = self.cache.get_or_request(&emote.url);
                    line = line.child(match resolved {
                        // Until the image lands, show the emote's name so the
                        // message still reads correctly.
                        None => div()
                            .text_color(rgb(0x8a8298))
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
                            img(path)
                                .id((SharedString::from(emote.url.clone()), emote_index))
                                .h(px(EMOTE_HEIGHT))
                                .tooltip(move |_window, cx| {
                                    cx.new(|_| EmoteTooltip { name: name.clone() }).into()
                                })
                                .into_any_element()
                        }
                    });
                    emote_index += 1;
                }
            }
        }

        line.into_any_element()
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();

        div().size_full().text_sm().child(
            list(self.list.clone(), move |index, _window, cx| {
                this.update(cx, |this: &mut ChatView, _cx| this.render_row(index))
                    .unwrap_or_else(|_| div().into_any_element())
            })
            .size_full(),
        )
    }
}
