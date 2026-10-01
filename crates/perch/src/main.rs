//! Twitch in one native window.
//!
//!     perch [channel or link...] [--volume 0-100]
//!
//! Two pages: browse what you follow, and watch up to four of them at once.
//! The watch page is deliberately bare — players and their chats, nothing else
//! — because chrome you stare past for three hours should not be there.
//!
//! This file is the process: whether it is the first perch or a later launch
//! to hand over, the window, and where stderr goes. The app itself is `root`;
//! what the arguments mean is `launch`.

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
mod history_page;
mod instance;
mod keys;
mod launch;
mod layout;
mod loudness;
mod motion;
mod os_window;
mod palette;
mod recommended;
mod root;
mod seek_bar;
mod settings_view;
mod sidebar;
mod stage;
mod target;
mod theme;
mod trail;
mod twitch;
mod video;
mod video_view;
mod vod;
mod watch;
mod widget_theme;

use gpui::{
    point, prelude::*, px, size, AnyView, App, Application, Bounds, DisplayId, Pixels, Size,
    TitlebarOptions, WindowBounds, WindowOptions,
};
use settings::{Settings, WindowPlacement};

use instance::Claim;
use launch::Launch;
use root::RootView;
use watch::MAX_PANES;

pub const APP_NAME: &str = "perch";

/// The window the app opens with when it has never been closed, before being
/// fitted to the display it lands on.
const DEFAULT_WINDOW: Size<Pixels> = size(px(1600.), px(920.));

/// How much of a display the default window may take. Not all of it: a window
/// that opens exactly the size of the screen reads as a maximised one that has
/// forgotten its chrome.
const DEFAULT_WINDOW_SHARE: f32 = 0.9;

/// Where to open the window, and on which display: where it was last closed,
/// if that is still on a display, or else centred on the primary display at a
/// size that fits it.
///
/// The fixed default this replaces was larger than a laptop's screen, and the
/// platform's answer to that is a window with its bottom edge off the display.
///
/// The display is only named on Windows, and there it is not optional. gpui
/// keeps saved bounds only when their centre lies on the display it is handed,
/// and with none it is handed the primary — so a window last closed on a
/// second monitor failed that check and opened at a default size on the first,
/// every time. macOS reads the bounds the other way, relative to the display
/// it is given, and the saved bounds are global; naming a display there would
/// shift the window by that display's origin.
fn initial_window_bounds(
    placement: Option<WindowPlacement>,
    cx: &App,
) -> (WindowBounds, Option<DisplayId>) {
    if let Some(saved) = placement {
        let bounds = Bounds {
            origin: point(px(saved.x), px(saved.y)),
            size: size(px(saved.width), px(saved.height)),
        };
        // Only if some display still holds it: a window last closed on a
        // monitor that has since been unplugged would open where nobody can
        // reach it.
        let displays = cx.displays();
        let areas: Vec<Bounds<Pixels>> = displays.iter().map(|display| display.bounds()).collect();
        if let Some(home) = home_display(&areas, bounds) {
            let window = if saved.maximized {
                WindowBounds::Maximized(bounds)
            } else {
                WindowBounds::Windowed(bounds)
            };
            return (window, cfg!(windows).then(|| displays[home].id()));
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
    (
        WindowBounds::Windowed(Bounds::centered(None, fitted, cx)),
        None,
    )
}

/// Which of `displays` a window at `window` belongs on: the one holding its
/// centre, which is the display gpui checks the bounds against, or failing
/// that any that still shows part of it — a window whose centre fell between
/// two monitors of different heights. `None` when no display shows any of it.
fn home_display(displays: &[Bounds<Pixels>], window: Bounds<Pixels>) -> Option<usize> {
    let centre = window.center();
    displays
        .iter()
        .position(|display| display.contains(&centre))
        .or_else(|| {
            displays
                .iter()
                .position(|display| display.intersects(&window))
        })
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let settings_path = settings::default_path(APP_NAME);

    // First of all, whether this is the perch at all, or a later launch whose
    // arguments belong to the one already running — see `instance`. Before
    // stderr is pointed at the log, because that starts by moving the running
    // copy's log aside. Later launches queue here from the start, ahead of
    // the window that will open them.
    let (to_window, launches) = futures::channel::mpsc::unbounded();
    let closing = to_window.clone();
    let settings_dir = settings_path.parent().unwrap_or(&settings_path);
    let unguarded = match instance::claim(settings_dir, &args) {
        Claim::First(listener) => {
            listener.serve(move |args| to_window.unbounded_send(args).is_ok());
            None
        }
        // Said to a debug build's console; a windowed build has none, and
        // the window coming forward is the answer.
        Claim::Handed => {
            eprintln!("{APP_NAME} is already running; it has this launch now");
            return;
        }
        Claim::Unanswered => {
            eprintln!("{APP_NAME} is already running and did not answer");
            return;
        }
        Claim::Alone(e) => Some(e),
    };

    // Before anything else that could go wrong: a windowed build has no
    // console, so stderr must be pointed somewhere first or the first failure
    // is silent. If this fails there is, by construction, nowhere to say so.
    if !cfg!(debug_assertions) {
        let _ = diagnostics::capture_stderr();
    }
    if let Some(e) = unguarded {
        eprintln!("instance: {e}; running without the one-perch guard");
    }
    // After stderr, so the path it reports has somewhere to be written. Opt-in
    // by environment variable in any build; a debug build's numbers are not
    // worth much, but the switch is the same one.
    if let Some(path) = cpu_log::start() {
        eprintln!("cpu log: {}", path.display());
    }

    let launch = Launch::read(args);
    if launch.help {
        eprintln!("{}", launch::USAGE);
        eprintln!();
        eprintln!("A release build is windowed, so this text goes to");
        eprintln!("{}", diagnostics::log_path().display());
        eprintln!();
        eprintln!("Name up to {MAX_PANES} channels, or twitch.tv links to channels or");
        eprintln!("recordings, to open them side by side.");
        eprintln!("With no channel, opens on the follows page.");
        eprintln!("While {APP_NAME} runs, launching it again brings the window forward");
        eprintln!("and opens what is named there.");
        eprintln!("Settings live at {}", settings_path.display());
        std::process::exit(0);
    }

    // Where the window was last closed, read before the app exists so the
    // window can open there rather than open elsewhere and jump.
    let placement = Settings::load(&settings_path)
        .ok()
        .and_then(|settings| settings.window)
        .filter(WindowPlacement::is_usable);

    // The assets are the icons, the widget library's and Perch's own; see
    // `assets`. Without them every chevron, eye and clear button in the app,
    // and every button on the title bar, renders as nothing, and silently — a
    // missing asset is not an error anywhere in that path.
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

            let (bounds, display_id) = initial_window_bounds(placement, cx);
            let options = WindowOptions {
                window_bounds: Some(bounds),
                display_id,
                // Perch draws its own title bar; see `root::title_bar`. The
                // platform's is made transparent rather than left out: the
                // title still names the window to the taskbar and Alt+Tab,
                // and on macOS `titlebar: None` would also take away the
                // close, resize and minimise the window is entitled to.
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(
                        px(theme::TRAFFIC_LIGHT_X),
                        px(theme::TRAFFIC_LIGHT_Y),
                    )),
                }),
                // No narrower than the bar Perch draws, or its caption buttons
                // run off the window's right edge; see `root::title_bar`.
                window_min_size: Some(root::window_min_size()),
                ..Default::default()
            };

            cx.open_window(options, move |window, cx| {
                let root = cx.new(|cx| RootView::new(launch, launches, window, cx));

                // Remember where the window was, on the way out. The platform
                // reports the restore bounds for a maximised or fullscreen
                // window, so what is saved is always a size that can be
                // opened windowed.
                //
                // And take every pop-out with it. Windows quits once the last
                // window is destroyed, and a pop-out left open would be a
                // player with no app around it. This window goes first — its
                // destruction is queued as it drops — and the process ends
                // when the last pop-out's goes through.
                window.on_window_should_close(cx, {
                    let root = root.downgrade();
                    move |window, cx| {
                        let placement = placement_of(window.window_bounds());
                        root.update(cx, |this, cx| {
                            this.remember_window(placement, cx);
                            this.remember_watching(cx);
                            this.close_pop_outs(cx);
                        })
                        .ok();
                        // A launch arriving from here on is turned away, and
                        // becomes the next perch once this one lets go, rather
                        // than being handed to a window that is going.
                        closing.close_channel();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(width), px(height)),
        }
    }

    /// The layout the bug was found on: a 4K primary with a 1440p monitor to
    /// its right, sitting lower. A window closed on the second monitor has to
    /// go back to the second monitor, not merely to "some display".
    #[test]
    fn a_window_goes_back_to_the_display_holding_its_centre() {
        let displays = [rect(0., 0., 3840., 2160.), rect(3840., 712., 2560., 1440.)];

        let on_second = rect(4213., 1213., 1536., 864.);
        assert_eq!(home_display(&displays, on_second), Some(1));

        let on_first = rect(1152., 660., 1536., 864.);
        assert_eq!(home_display(&displays, on_first), Some(0));

        // Straddling the seam: it belongs where most of it — its centre — is.
        let straddling = rect(3000., 800., 1200., 600.);
        assert_eq!(home_display(&displays, straddling), Some(0));
    }

    /// A centre that falls in the gap beside a shorter monitor still has a
    /// home, as long as some of the window is on a display.
    #[test]
    fn a_centre_off_every_display_falls_back_to_one_that_shows_the_window() {
        let displays = [rect(0., 0., 3840., 2160.), rect(3840., 712., 2560., 1440.)];
        let centre_in_the_gap = rect(3700., 150., 600., 400.);
        assert_eq!(home_display(&displays, centre_in_the_gap), Some(0));
    }

    /// A monitor that has been unplugged since takes the window with it; the
    /// caller then opens a default one on the primary.
    #[test]
    fn a_window_on_no_display_has_no_home() {
        let displays = [rect(0., 0., 1920., 1080.)];
        assert_eq!(home_display(&displays, rect(5000., 100., 800., 600.)), None);
    }
}
