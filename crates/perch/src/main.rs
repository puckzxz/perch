//! Twitch in one native window.
//!
//!     perch [channel...] [--volume 0-100]
//!
//! Two pages: browse what you follow, and watch up to four of them at once.
//! The watch page is deliberately bare — players and their chats, nothing else
//! — because chrome you stare past for three hours should not be there.
//!
//! This file is the process: the arguments, the window, and where stderr
//! goes. The app itself is `root`.

// No console window in a real build. Debug builds keep one, because that is
// where `--help` and a live stderr are worth more than the tidiness. See
// `diagnostics` for where the output goes instead.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod browse;
mod channel_page;
mod chat;
mod chat_text;
mod clock;
mod controls;
mod cpu_log;
mod diagnostics;
mod keys;
mod layout;
mod motion;
mod palette;
mod root;
mod seek_bar;
mod settings_view;
mod sidebar;
mod target;
mod theme;
mod twitch;
mod video;
mod video_view;
mod vod;
mod watch;
mod widget_theme;

use gpui::{
    point, prelude::*, px, size, AnyView, App, Application, Bounds, Pixels, Size, TitlebarOptions,
    WindowBounds, WindowOptions,
};
use settings::{Settings, WindowPlacement};

use root::RootView;
use target::Target;
use watch::MAX_PANES;

pub const APP_NAME: &str = "perch";

/// The window the app opens with when it has never been closed, before being
/// fitted to the display it lands on.
const DEFAULT_WINDOW: Size<Pixels> = size(px(1600.), px(920.));

/// How much of a display the default window may take. Not all of it: a window
/// that opens exactly the size of the screen reads as a maximised one that has
/// forgotten its chrome.
const DEFAULT_WINDOW_SHARE: f32 = 0.9;

/// Where to open the window: where it was last closed, if that is still on
/// a display, or else centred on the primary display at a size that fits it.
///
/// The fixed default this replaces was larger than a laptop's screen, and the
/// platform's answer to that is a window with its bottom edge off the display.
fn initial_window_bounds(placement: Option<WindowPlacement>, cx: &App) -> WindowBounds {
    if let Some(saved) = placement {
        let bounds = Bounds {
            origin: point(px(saved.x), px(saved.y)),
            size: size(px(saved.width), px(saved.height)),
        };
        // Only if some display still holds it: a window last closed on a
        // monitor that has since been unplugged would open where nobody can
        // reach it.
        let visible = cx
            .displays()
            .iter()
            .any(|display| display.bounds().intersects(&bounds));
        if visible {
            return if saved.maximized {
                WindowBounds::Maximized(bounds)
            } else {
                WindowBounds::Windowed(bounds)
            };
        }
    }

    let fitted = match cx.primary_display() {
        Some(display) => {
            let room = display.bounds().size;
            size(
                DEFAULT_WINDOW.width.min(room.width * DEFAULT_WINDOW_SHARE),
                DEFAULT_WINDOW
                    .height
                    .min(room.height * DEFAULT_WINDOW_SHARE),
            )
        }
        None => DEFAULT_WINDOW,
    };
    WindowBounds::Windowed(Bounds::centered(None, fitted, cx))
}

/// What to remember about a window that is closing.
fn placement_of(bounds: WindowBounds) -> WindowPlacement {
    let (rect, maximized) = match bounds {
        WindowBounds::Windowed(rect) => (rect, false),
        WindowBounds::Maximized(rect) => (rect, true),
        // The restore size, and not fullscreen: a window that opens fullscreen
        // with no chrome is a window somebody has to remember a key to escape.
        WindowBounds::Fullscreen(rect) => (rect, false),
    };
    WindowPlacement {
        x: f32::from(rect.origin.x),
        y: f32::from(rect.origin.y),
        width: f32::from(rect.size.width),
        height: f32::from(rect.size.height),
        maximized,
    }
}

fn main() {
    // Before anything that could go wrong: a windowed build has no console, so
    // stderr must be pointed somewhere first or the first failure is silent.
    // If this fails there is, by construction, nowhere to say so.
    if !cfg!(debug_assertions) {
        let _ = diagnostics::capture_stderr();
    }
    // After stderr, so the path it reports has somewhere to be written. Opt-in
    // by environment variable in any build; a debug build's numbers are not
    // worth much, but the switch is the same one.
    if let Some(path) = cpu_log::start() {
        eprintln!("cpu log: {}", path.display());
    }

    let mut args = std::env::args().skip(1);
    let mut targets: Vec<Target> = Vec::new();
    let mut volume = None;
    // What could not be used, to be said in the window. A release build has
    // no console, so anything only printed here is never seen.
    let mut warnings: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--volume" => match args.next().and_then(|v| v.parse::<u8>().ok()) {
                Some(level) => volume = Some(level.min(100)),
                None => warnings.push("--volume needs a number from 0 to 100; ignored".into()),
            },
            "--help" | "-h" => {
                eprintln!("usage: {APP_NAME} [channel...] [--volume 0-100]");
                eprintln!();
                eprintln!("A release build is windowed, so this text goes to");
                eprintln!("{}", diagnostics::log_path().display());
                eprintln!();
                eprintln!("Name up to {MAX_PANES} channels, or twitch.tv links to channels or");
                eprintln!("recordings, to open them side by side.");
                eprintln!("With no channel, opens on the follows page.");
                eprintln!(
                    "Settings live at {}",
                    settings::default_path(APP_NAME).display()
                );
                std::process::exit(0);
            }
            // Anything else that looks like an option is a mistake worth
            // naming, rather than a channel called `--foo` that streamlink
            // fails on a few seconds later with a message about a URL.
            flag if flag.starts_with('-') => {
                warnings.push(format!("unknown option {flag}; ignored"));
            }
            // A login, or a twitch.tv link to a channel or a recording, read
            // the one way the app reads either — see `target`.
            other => match target::parse(other) {
                Some(target) => targets.push(target),
                None => warnings.push(format!(
                    "{other:?} is not a Twitch channel or recording; skipped"
                )),
            },
        }
    }

    // Where the window was last closed, read before the app exists so the
    // window can open there rather than open elsewhere and jump.
    let placement = Settings::load(&settings::default_path(APP_NAME))
        .ok()
        .and_then(|settings| settings.window)
        .filter(WindowPlacement::is_usable);

    // The assets are the widget library's icons; see `assets`. Without them
    // every chevron, eye and clear button in the app renders as nothing, and
    // silently — a missing asset is not an error anywhere in that path.
    Application::new()
        .with_assets(assets::Icons)
        .run(move |cx: &mut App| {
            // Must come before any gpui-component widget is constructed.
            gpui_component::init(cx);
            // And this must come after it: `init` installs the palette this
            // overwrites, seeded from the *operating system's* light/dark setting.
            widget_theme::apply(cx);
            // After it, not before: same-depth ties are won by whoever registered
            // last, and these bindings are the ones that must stand aside.
            keys::init(cx);

            let options = WindowOptions {
                window_bounds: Some(initial_window_bounds(placement, cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                let root = cx
                    .new(|cx| RootView::new(targets.clone(), volume, warnings.clone(), window, cx));

                // Remember where the window was, on the way out. The platform
                // reports the restore bounds for a maximised or fullscreen
                // window, so what is saved is always a size that can be
                // opened windowed.
                window.on_window_should_close(cx, {
                    let root = root.downgrade();
                    move |window, cx| {
                        let placement = placement_of(window.window_bounds());
                        root.update(cx, |this, cx| this.remember_window(placement, cx))
                            .ok();
                        true
                    }
                });

                // Root is required as the window's first child so overlay layers
                // have somewhere to render.
                cx.new(|cx| gpui_component::Root::new(AnyView::from(root), window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
}
