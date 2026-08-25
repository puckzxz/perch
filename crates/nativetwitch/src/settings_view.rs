//! The settings panel.
//!
//! Exists mainly so credentials never have to be typed into a JSON file. The
//! two Twitch tokens do different jobs and are easy to confuse, so the panel
//! says which is which rather than assuming anyone remembers.

use gpui::{
    div, prelude::*, px, rgb, rgba, Context, Entity, EventEmitter, SharedString, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    select::{Select, SelectState},
    IndexPath,
};
use settings::{QualityPreference, Settings};

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

pub enum SettingsEvent {
    Saved(Box<Settings>),
    Dismissed,
}

pub struct SettingsPanel {
    settings: Settings,
    client_id: Entity<InputState>,
    auth_token: Entity<InputState>,
    quality: Entity<SelectState<Vec<SharedString>>>,
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
                .default_value(settings.credentials.client_id.clone().unwrap_or_default())
        });

        let auth_token = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("auth-token cookie (optional)")
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
        let quality = cx.new(|cx| {
            SelectState::new(options, Some(IndexPath::new(selected)), window, cx)
        });

        Self {
            settings,
            client_id,
            auth_token,
            quality,
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

        self.settings = settings.clone();
        cx.emit(SettingsEvent::Saved(Box::new(settings)));
    }

    fn field(
        label: &'static str,
        help: &'static str,
        control: impl IntoElement,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0xf2eff7))
                    .child(label),
            )
            .child(control)
            .child(div().text_xs().text_color(rgb(0x6b6478)).child(help))
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            // A scrim, so the panel reads as modal rather than as a floating
            // rectangle over live video.
            .bg(rgba(0x0a0810cc))
            .child(
                div()
                    .w(px(480.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(0x1b1822))
                    .border_1()
                    .border_color(rgb(0x2e2939))
                    .shadow_lg()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0xf2eff7))
                            .child("Settings"),
                    )
                    .child(Self::field(
                        "Twitch Client ID",
                        "From an application you register at dev.twitch.tv. Set Client Type to Public; no secret is needed. Required to list your follows.",
                        Input::new(&self.client_id).cleanable(true),
                    ))
                    .child(Self::field(
                        "auth-token cookie",
                        "A different credential: the auth-token cookie from twitch.tv. Unlocks Prime/Turbo ad suppression and sub-only qualities. This is a full account credential and is stored in plain text.",
                        Input::new(&self.auth_token).mask_toggle().cleanable(true),
                    ))
                    .child(Self::field(
                        "Stream quality",
                        "Auto picks a stream that scales cleanly to the video pane. Best is usually the wrong choice: a 1440p stream in a 720px pane decodes four times the pixels and then throws them away.",
                        Select::new(&self.quality),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x948ca5))
                            .child(self.sign_in_status.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
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
            )
    }
}
