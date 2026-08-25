//! Prove whether GPUI advances GIF frames, and what it takes.
//!
//! GPUI only requests an animation frame when the image element has a global
//! id (`img.rs`: `if global_id.is_some() && data.frame_count() > 1`). This
//! example renders the same GIF twice — once with an id, once without — so the
//! difference is visible side by side.
//!
//!     cargo run -p nativetwitch --example gif_animation -- <path-to.gif>

use std::path::PathBuf;

use gpui::{
    div, img, prelude::*, px, rgb, size, App, Application, Bounds, Context, Window, WindowBounds,
    WindowOptions,
};

struct Demo {
    path: PathBuf,
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let label = |text: &'static str| {
            div()
                .text_color(rgb(0x948ca5))
                .text_sm()
                .child(text)
                .into_any_element()
        };

        div()
            .size_full()
            .bg(rgb(0x131118))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap_12()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(label("with id (should animate)"))
                    .child(img(self.path.clone()).id("animated").h(px(160.))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(label("no id (should freeze)"))
                    .child(img(self.path.clone()).h(px(160.))),
            )
    }
}

fn main() {
    let Some(path) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: gif_animation <path-to.gif>");
        std::process::exit(2);
    };

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(700.), px(320.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_window, cx| cx.new(|_| Demo { path: path.clone() }),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
