//! The chat pane.
//!
//! Uses a bottom-anchored `ListState`, which is what keeps new messages pinned
//! to the bottom the way every chat client does, without any manual scrolling.

use gpui::{
    div, list, prelude::*, px, rgb, AnyElement, Context, ListAlignment, ListState, SharedString,
    Task, Window,
};
use twitch_chat::{ChatClient, ChatEvent, ChatMessage};

/// Messages kept in memory. Old ones are dropped from the top: chat runs
/// forever, and nobody scrolls back a thousand lines in a live stream.
const MAX_MESSAGES: usize = 500;

#[derive(Clone)]
enum Row {
    Message(Box<ChatMessage>),
    Notice(SharedString),
}

pub struct ChatView {
    rows: Vec<Row>,
    list: ListState,
    _client: ChatClient,
    _pump: Task<()>,
}

impl ChatView {
    pub fn new(channel: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
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

        let mut view = Self {
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Bottom, px(400.)),
            _client: client,
            _pump: pump,
        };
        view.push(Row::Notice(format!("connecting to #{channel}…").into()));
        view
    }

    fn apply(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Connected { channel } => {
                self.push(Row::Notice(format!("joined #{channel}").into()))
            }
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

            Row::Message(message) => {
                let name: SharedString = message.display_name.clone().into();
                let body: SharedString = if message.is_action {
                    format!("{} {}", name, message.text).into()
                } else {
                    message.text.clone().into()
                };

                let text_color = if message.is_action {
                    rgb(message.color)
                } else {
                    rgb(0xd8d3e0)
                };

                // w_full is what gives the row a definite width; without it the
                // row sizes to its content and long messages run off the pane
                // instead of wrapping.
                let mut line = div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap_1()
                    .px_3()
                    .py_0p5();

                // An action renders as one coloured sentence, so the name is
                // already inside the body and must not be repeated.
                if !message.is_action {
                    line = line.child(
                        div()
                            .flex_none()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(message.color))
                            .child(name),
                    );
                }

                line.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(text_color)
                        .child(body),
                )
                .into_any_element()
            }
        }
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
