//! The settings panel.
//!
//! Exists mainly so credentials never have to be typed into a JSON file. The
//! two Twitch tokens do different jobs and are easy to confuse, so the panel
//! says which is which rather than assuming anyone remembers.

use gpui::{div, prelude::*, px, relative, Context, Entity, EventEmitter, SharedString, Window};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    select::{Select, SelectState},
    switch::Switch,
    IndexPath,
};
use settings::{QualityPreference, Settings};

use crate::keys;
use crate::motion;
use crate::theme;

/// How far the panel travels as it opens. Enough to read as arriving from
/// somewhere, not enough to watch.
const PANEL_RISE: f32 = 12.0;

/// Half the panel's content width, so the shortcut listing reads as two
/// columns rather than as one long list nobody scans to the end of.
const SHORTCUT_COLUMN: f32 = 210.0;
/// Wide enough for the longest key name in the listing.
const SHORTCUT_KEY: f32 = 56.0;

/// Offered in the quality dropdown. "Auto" first because it is usually the
/// right answer: it picks a stream that lands on a clean scale ratio, which
/// measurably costs less CPU than simply taking the best available.
/// Label shown, and the preference stored for it.
///
/// Stored by height rather than by exact streamlink name: channels name the
/// same rendition differently (`480p` vs `480p30`, or `936p60` on a stream with
/// an unusual aspect), so an exact name would silently stop matching.
const QUALITY_OPTIONS: [(&str, &str); 8] = [
    ("Auto (matches the video pane)", "auto"),
    ("Best available", "best"),
    ("1440p", "1440p"),
    ("1080p", "1080p"),
    ("720p", "720p"),
    ("480p", "480p"),
    ("360p", "360p"),
    ("160p", "160p"),
];

/// How much of a channel's chat a new pane opens with.
///
/// Off is on the list rather than being a value nobody can reach, because this
/// is the only thing in the app that asks a **third party** for content: Twitch
/// publishes no scrollback, so the messages come from the community service
/// Chatterino uses, and asking tells it which channels are being watched.
const HISTORY_OPTIONS: [(&str, usize); 4] = [
    ("100 messages", 100),
    ("250 messages", 250),
    ("500 messages", 500),
    ("Off — join with an empty pane", 0),
];

pub enum SettingsEvent {
    Saved(Box<Settings>),
    Dismissed,
}

pub struct SettingsPanel {
    settings: Settings,
    client_id: Entity<InputState>,
    auth_token: Entity<InputState>,
    quality: Entity<SelectState<Vec<SharedString>>>,
    history: Entity<SelectState<Vec<SharedString>>>,
    /// Held here rather than read back off the switch: a `Switch` reports
    /// clicks and keeps no state of its own.
    miniplayer: bool,
    sign_in_status: SharedString,
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}

impl SettingsPanel {
    pub fn new(
        settings: Settings,
        sign_in_status: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let client_id = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Client ID from dev.twitch.tv")
                .masked(true)
                .default_value(settings.credentials.client_id.clone().unwrap_or_default())
        });

        let auth_token = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("auth-token cookie (optional)")
                .masked(true)
                .default_value(settings.credentials.auth_token.clone().unwrap_or_default())
        });

        let selected = match &settings.quality {
            QualityPreference::Auto => 0,
            QualityPreference::Fixed(stored) => QUALITY_OPTIONS
                .iter()
                .position(|(_, value)| value == stored)
                .unwrap_or(0),
        };
        let options: Vec<SharedString> = QUALITY_OPTIONS
            .iter()
            .map(|(label, _)| SharedString::from(*label))
            .collect();
        let quality =
            cx.new(|cx| SelectState::new(options, Some(IndexPath::new(selected)), window, cx));

        // A hand-edited value that is not on the list falls back to the first
        // option rather than being silently kept and then silently overwritten
        // on the next save.
        let selected = HISTORY_OPTIONS
            .iter()
            .position(|(_, value)| *value == settings.chat_history)
            .unwrap_or(0);
        let options: Vec<SharedString> = HISTORY_OPTIONS
            .iter()
            .map(|(label, _)| SharedString::from(*label))
            .collect();
        let history =
            cx.new(|cx| SelectState::new(options, Some(IndexPath::new(selected)), window, cx));

        Self {
            miniplayer: settings.miniplayer,
            settings,
            client_id,
            auth_token,
            quality,
            history,
            sign_in_status,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let trimmed = |value: String| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        };

        let mut settings = self.settings.clone();
        settings.credentials.client_id = trimmed(self.client_id.read(cx).value().to_string());
        settings.credentials.auth_token = trimmed(self.auth_token.read(cx).value().to_string());

        // Index 0 is Auto; the rest map to streamlink quality names verbatim.
        let index = self
            .quality
            .read(cx)
            .selected_index(cx)
            .map(|path| path.row)
            .unwrap_or(0);
        settings.quality = if index == 0 {
            QualityPreference::Auto
        } else {
            QualityPreference::Fixed(QUALITY_OPTIONS[index].1.to_string())
        };

        let index = self
            .history
            .read(cx)
            .selected_index(cx)
            .map(|path| path.row)
            .unwrap_or(0);
        settings.chat_history = HISTORY_OPTIONS[index].1;
        settings.miniplayer = self.miniplayer;

        self.settings = settings.clone();
        cx.emit(SettingsEvent::Saved(Box::new(settings)));
    }

    /// The keymap, as a reference rather than a control.
    ///
    /// It reads from `keys::SHORTCUTS`, which carries the keystroke each row is
    /// documenting alongside the text — and a test there checks every one of
    /// them against the real keymap, so a key documented here is one that is
    /// actually bound.
    fn shortcuts() -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_y(px(theme::GAP_TIGHT))
            .children(keys::SHORTCUTS.iter().map(|(_, key, description)| {
                div()
                    .w(px(SHORTCUT_COLUMN))
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(theme::GAP_TIGHT))
                    .text_size(px(theme::TEXT_META))
                    .child(
                        div()
                            .flex_none()
                            .w(px(SHORTCUT_KEY))
                            .font_weight(theme::weight_label())
                            .text_color(theme::text())
                            .child(*key),
                    )
                    .child(div().text_color(theme::text_muted()).child(*description))
            }))
    }

    /// A heading over a group of fields.
    ///
    /// The sheet was five fields in a flat column, which is five things to read
    /// in order rather than three subjects to find. Credentials, playback and
    /// chat are what somebody comes here to change; the keyboard listing is a
    /// reference and sits last.
    fn section(title: &'static str) -> impl IntoElement {
        div()
            .text_size(px(theme::TEXT_LABEL))
            .font_weight(theme::weight_label())
            .text_color(theme::text_dim())
            .child(title)
    }

    fn field(
        label: &'static str,
        help: &'static str,
        control: impl IntoElement,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_TIGHT))
            .child(
                div()
                    .text_size(px(theme::TEXT_LABEL))
                    .font_weight(theme::weight_label())
                    .text_color(theme::text())
                    .child(label),
            )
            .child(control)
            .child(
                div()
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_BODY))
                    .text_color(theme::text_dim())
                    .child(help),
            )
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            // A modal has to swallow input, not merely cover it. GPUI hit-tests
            // every overlapping element rather than only the topmost, so
            // without this a click inside the panel also reaches whatever it is
            // drawn over. `occlude` rather than the scroll-permitting variant:
            // the page behind a modal should not scroll either.
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            // A scrim, so the panel reads as modal rather than as a floating
            // rectangle over live video.
            .bg(theme::scrim())
            .child(motion::arrive(
                "settings-panel",
                PANEL_RISE,
                div()
                    .w(px(480.))
                    // The sheet is taller than a small window, and used to be
                    // centred inside one with no way to reach either end: at
                    // 1000x640 the title was cut off the top and Save and Close
                    // were cut off the bottom, so the sheet could be opened and
                    // dismissed but not used. The title and the buttons are
                    // pinned now and only the fields scroll.
                    .max_h(relative(0.9))
                    .flex()
                    .flex_col()
                    .rounded(px(theme::RADIUS_LG))
                    .bg(theme::surface_raised())
                    .border_1()
                    .border_color(theme::border())
                    .shadow_lg()
                    .child(
                        div()
                            .flex_none()
                            .px(px(theme::PAGE_PAD))
                            .pt(px(theme::PAGE_PAD))
                            .pb(px(theme::PANEL_PAD))
                            .text_size(px(theme::TEXT_TITLE))
                            .font_weight(theme::weight_title())
                            .text_color(theme::text())
                            .child("Settings"),
                    )
                    .child(
                        div()
                    .id("settings-fields")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(theme::GAP_SECTION))
                    .px(px(theme::PAGE_PAD))
                    .pb(px(theme::PAGE_PAD))
                    .child(Self::section("Twitch account"))
                    .child(Self::field(
                        "Twitch Client ID",
                        "From an application you register at dev.twitch.tv. Set Client Type to Public; no secret is needed. Required to list your follows.",
                        Input::new(&self.client_id).mask_toggle().cleanable(true),
                    ))
                    .child(Self::field(
                        "auth-token cookie",
                        "A different credential: the auth-token cookie from twitch.tv. Unlocks Prime/Turbo ad suppression and sub-only qualities. This is a full account credential and is stored in plain text.",
                        Input::new(&self.auth_token).mask_toggle().cleanable(true),
                    ))
                    .child(Self::section("Playback"))
                    .child(Self::field(
                        "Stream quality",
                        "Auto picks a stream that scales cleanly to the video pane. Best is usually the wrong choice: a 1440p stream in a 720px pane decodes four times the pixels and then throws them away.",
                        Select::new(&self.quality),
                    ))
                    .child(Self::field(
                        "Keep playing while browsing",
                        "What you are watching carries on, muted, in a bar along the bottom of the follows page. Turned off, leaving the watch page stops the stream — which is the cheaper answer if you go there to pick the next thing rather than to glance at the list.",
                        Switch::new("miniplayer")
                            .checked(self.miniplayer)
                            .label(if self.miniplayer { "On" } else { "Off" })
                            .on_click(cx.listener(|this: &mut Self, checked: &bool, _, cx| {
                                this.miniplayer = *checked;
                                cx.notify();
                            })),
                    ))
                    .child(Self::section("Chat"))
                    .child(Self::field(
                        "Chat history on join",
                        "Opens a new chat pane with what was already being said. Twitch publishes no scrollback, so these come from the community service Chatterino uses — which means the request tells someone other than Twitch which channels you watch.",
                        Select::new(&self.history),
                    ))
                    .child(Self::section("Keyboard"))
                    .child(Self::field(
                        "Keyboard shortcuts",
                        "Player keys act on the pane you last pointed at. All of them stand aside while the cursor is in a box like this one.",
                        Self::shortcuts(),
                    ))
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(theme::GAP))
                            .px(px(theme::PAGE_PAD))
                            .py(px(theme::PANEL_PAD))
                            .border_t_1()
                            .border_color(theme::border())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(theme::TEXT_META))
                                    .text_color(theme::text_muted())
                                    .child(self.sign_in_status.clone()),
                            )
                            .child(Button::new("cancel").ghost().label("Close").on_click(
                                cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Dismissed)),
                            ))
                            .child(
                                Button::new("save")
                                    .primary()
                                    .label("Save")
                                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                            ),
                    ),
            ))
    }
}
