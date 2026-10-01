//! The watch page: one to four streams, each with its own player and chat.
//!
//! Every pane is independent — its own volume, its own quality, its own chat —
//! because the point of watching two streams at once is that they are two
//! streams, not one with a picture-in-picture. The arrangement comes from
//! [`crate::layout`], which derives a grid from the window rather than looking
//! one up per stream count.
//!
//! A pane with no picture is `status`'s: the one reading of what it is
//! showing ([`Showing`]), which the mini player's tiles read too, and the
//! screen drawn from it. A pane's header is `header`'s, and where it goes —
//! in the chat panel, or over the top of the picture when there is no chat on
//! screen — is [`Placement`]. Whatever a pane asks of its owner — a close, a
//! retry, a press that makes it the active one — is a [`PaneAction`],
//! addressed by the pane's key.

mod header;
mod status;

use chrono::{DateTime, Utc};
use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Context, CursorStyle, ElementId, Entity,
    IntoElement, MouseButton, MouseDownEvent, Pixels, SharedString, Task, Window,
};
use streamlink::StreamSupervisor;

use twitch_api::{LiveStream, Video};

pub use self::header::Placement;
pub use self::status::{showing, Showing};

use crate::chat::ChatView;
use crate::layout;
use crate::motion;
use crate::target::{self, Target};
use crate::theme;
use crate::video::PositionHandle;
use crate::video_view::VideoView;

/// Beyond this, panes are too small to read chat in and the CPU cost stops
/// being worth it.
pub const MAX_PANES: usize = 4;

/// Element ids inside a pane identify its **key**, never its position.
///
/// Closing a pane reindexes every pane after it. Position-keyed ids make the
/// survivor inherit the closed pane's element state - including whether GPUI
/// thinks it is hovered - and since `on_hover` only fires when that value
/// *changes*, a stale `true` means the header never returns until the pointer
/// leaves the pane and comes back. Same lesson as the animated-emote ids: the
/// id has to name the thing, not the slot it happens to be in.
fn pane_id(key: &str, role: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("pane-{role}-{key}")))
}

/// What a pane is playing.
pub enum Source {
    /// A channel, as it broadcasts.
    Live,
    /// One of a channel's past broadcasts.
    Video {
        video: Box<Video>,
        /// Where in it the pane is. The pane's rather than the player's, so
        /// it outlives a quality change, and so the chat replay can follow
        /// it from the moment the pane opens — see [`PositionHandle`].
        position: PositionHandle,
    },
}

pub enum StreamState {
    Starting,
    Playing(Entity<VideoView>),
    /// The channel was not broadcasting when the pane opened.
    Offline,
    /// It was, and then it stopped. Distinct from [`Offline`](Self::Offline)
    /// because the two are different news: one is a channel you could not
    /// watch, the other is one you were watching until a moment ago.
    Ended,
    Failed(SharedString),
}

/// One stream and everything that belongs to it.
///
/// Dropping a slot stops its streamlink (the supervisor) and its mpv (the
/// view), so removing a pane needs no explicit teardown.
pub struct Slot {
    /// What identifies this pane: the login for a live stream, or
    /// [`Slot::video_key`] for a recording. Element ids, the active pane and
    /// every lookup use this — never the position, see [`pane_id`], and never
    /// the login alone, so a channel's stream and one of its recordings can be
    /// open side by side.
    pub key: String,
    /// The channel, whether live or recorded: what volume and hidden chat are
    /// remembered against, and whose name the header carries.
    pub channel: String,
    pub source: Source,
    /// A quality picked from this pane's controls, overriding the saved
    /// preference until the pane closes.
    pub quality_override: Option<String>,
    pub state: StreamState,
    /// The chat beside the video: live, or replayed against where the
    /// recording is. `None` only for a video whose chat cannot be replayed:
    /// a highlight is cut from ranges of a broadcast, so its offsets mean
    /// nothing, and an upload was never a broadcast at all.
    pub chat: Option<Entity<ChatView>>,
    /// Where a recording picks up when its player is started again: after a
    /// quality change, or from the top once it has finished.
    pub resume_at: f64,
    pub supervisor: Option<StreamSupervisor>,
    pub pump: Option<Task<()>>,
    /// Whether the pointer is over this pane's video, measured rather than
    /// reported — see `VideoView::hovered` for why that distinction matters.
    /// Two things follow it: the rising edge makes the pane the one the keys
    /// talk to, and while it holds, a header over the picture is up. Both
    /// are worked out in [`point`](Self::point).
    pub hovered: bool,
    /// The header over the top of the picture, when there is no chat on
    /// screen for it to sit above ([`Placement::OverPicture`]): whether it
    /// is up, and how far through fading. Worked out by
    /// [`point`](Self::point) every frame the pane is drawn.
    ///
    /// The slot's rather than the player's, so it outlives the player: a
    /// quality change builds a new one, and a pane with no player at all —
    /// starting, offline, ended — still has a header to show.
    pub header: motion::Fade,
    /// Brought up for a moment by a key, without the pointer: a pane key
    /// made this the pane the keys talk to, or `C` hid its chat and sent its
    /// header over the picture. Set and cleared by `RootView::reveal_header`,
    /// one pane at a time.
    pub revealed: bool,
    /// Whether this pane's chat is hidden: beside the picture, the picture
    /// takes chat's column; stacked, it keeps its box (see `chat_or_why`).
    /// Either way the header goes over the picture ([`Placement`]).
    ///
    /// Per pane, like everything else here, and remembered per channel: a
    /// channel you watch for the game is not a statement about the next one.
    pub chat_hidden: bool,
    /// Held silent by Mute all: a mute that is not a preference. The slot's
    /// rather than the player's, so it outlives the player — a quality change
    /// or a re-pick builds a new one, which is born quiet from this. Ends at
    /// the pane's first deliberate change of level, and is never saved.
    pub quiet: bool,
    /// When this pane last found nothing to play: the moment streamlink said
    /// the channel was off, or the moment the broadcast ended. A follows poll
    /// that lists the channel live with a `started_at` later than this is a
    /// broadcast this pane has not tried, and it tries it — see
    /// `RootView::on_streams`.
    pub stalled_at: Option<DateTime<Utc>>,
}

impl Slot {
    /// A pane that has just been asked for: starting, with nothing running
    /// yet, nothing picked from its menu, and the pointer not on it.
    ///
    /// The one way a slot is made. Both kinds of pane come through here, so a
    /// field added to `Slot` gets its first value once, rather than once per
    /// place a pane is opened from — and a test can make one without a
    /// window. `start_stream` gives it its streamlink and pump.
    pub fn new(
        key: String,
        channel: String,
        source: Source,
        chat: Option<Entity<ChatView>>,
        resume_at: f64,
        chat_hidden: bool,
    ) -> Self {
        Self {
            key,
            channel,
            source,
            quality_override: None,
            state: StreamState::Starting,
            chat,
            resume_at,
            supervisor: None,
            pump: None,
            hovered: false,
            header: motion::Fade::hidden(),
            revealed: false,
            chat_hidden,
            quiet: false,
            stalled_at: None,
        }
    }

    pub fn video(&self) -> Option<&Entity<VideoView>> {
        match &self.state {
            StreamState::Playing(view) => Some(view),
            _ => None,
        }
    }

    /// Whether the pane has a picture up: a player, and a frame decoded in
    /// it. A player still waiting for its first frame draws no picture and
    /// no bar, so for what goes over the top of the pane it is a pane with
    /// nothing to cover, like one starting or stopped.
    pub fn has_picture(&self, cx: &App) -> bool {
        self.video().is_some_and(|view| view.read(cx).has_picture())
    }

    /// Where the pointer is, from the pane's probe, every frame the pane is
    /// drawn: `inside` it or not, with the pane showing a `picture` or not,
    /// and a `menu_open` on its bar or not. Says whether the pointer has just
    /// come in, which is what makes a pane the one the keys talk to, and
    /// whether anything drawn changed.
    ///
    /// Every frame rather than on crossings, because the header follows more
    /// than the pointer — a picture arriving, a menu opening, a reveal — and
    /// because a pane can be pointed at with no crossing to report:
    /// `RootView::go_watch_pane` counts every pane as pointed at already, so
    /// the pane under the pointer after a mini-player tile's click has none.
    pub fn point(&mut self, inside: bool, picture: bool, menu_open: bool) -> Pointed {
        let entered = inside && !self.hovered;
        self.hovered = inside;
        let wanted = band_wanted(placement(self), inside, self.revealed, picture, menu_open);
        Pointed {
            entered,
            changed: self.header.set(wanted) || entered,
        }
    }

    /// Move to `state`, noting the time if it is one with nothing playing —
    /// see [`stalled_at`](Self::stalled_at). Every change of state goes
    /// through here, so the note cannot be forgotten by one of them.
    pub fn set_state(&mut self, state: StreamState) {
        self.stalled_at = match state {
            StreamState::Offline | StreamState::Ended | StreamState::Failed(_) => Some(Utc::now()),
            StreamState::Starting | StreamState::Playing(_) => None,
        };
        self.state = state;
    }

    /// The key a recording's pane gets. Prefixed so it can never collide with
    /// a login, which is letters, digits and underscores.
    pub fn video_key(video_id: &str) -> String {
        format!("vod:{video_id}")
    }

    pub fn is_live(&self) -> bool {
        matches!(self.source, Source::Live)
    }

    pub fn recording(&self) -> Option<&Video> {
        match &self.source {
            Source::Video { video, .. } => Some(video),
            Source::Live => None,
        }
    }

    /// The twitch.tv page for what this pane plays: the channel, or the
    /// recording — and with `at_position`, the recording at the moment the
    /// pane is at, which pasted back into Perch opens it there.
    ///
    /// A live pane's link has no time either way: a moment in a broadcast
    /// still going is a moment in its archive, which the pane does not know.
    /// The header's name passes `false`, since a name says which and never
    /// when; More and the palette pass `true`.
    pub fn link(&self, at_position: bool) -> String {
        target::link(&match &self.source {
            Source::Live => Target::Channel(self.channel.clone()),
            Source::Video { video, position } => Target::Video {
                id: video.id.clone(),
                start_secs: at_position
                    .then(|| target::moment(position.get()))
                    .flatten(),
            },
        })
    }
}

/// What [`Slot::point`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pointed {
    /// The pointer has just come into the pane, so it is now the one the keys
    /// talk to.
    pub entered: bool,
    /// Something drawn changed — the header over the picture came or went, or
    /// the active pane moved — so the page wants drawing again.
    pub changed: bool,
}

/// Where `slot`'s header goes; see [`Placement::of`].
pub fn placement(slot: &Slot) -> Placement {
    Placement::of(slot.chat_hidden, slot.chat.is_some())
}

/// Whether a pane's header is up over its picture.
///
/// Only when it lives there at all. Then: while the pointer is on the pane,
/// or a key has `revealed` it, or always when there is no `picture` to cover
/// — a status screen is not a picture, and a stopped pane with chat hidden
/// would otherwise have nothing on screen to say whose it is or to close it.
/// And never over an open menu: the menu is what was asked for last, and the
/// two would stack over the same picture.
fn band_wanted(
    placement: Placement,
    inside: bool,
    revealed: bool,
    picture: bool,
    menu_open: bool,
) -> bool {
    placement == Placement::OverPicture && (inside || revealed || !picture) && !menu_open
}

/// What the root knows about a pane beyond its slot, resolved once per frame
/// in `RootView::watch_page` rather than per pane inside the page.
pub struct PaneInfo<'a> {
    /// The live record the header's numbers, title and game come from, when
    /// a list the app has fetched carries the channel — see
    /// `RootView::stream_info`. `None` for a recording, whose header speaks
    /// for the recording, and for a channel opened by name that is in no list.
    pub stream: Option<&'a LiveStream>,
    /// What the pane calls its channel, in its header and its status line —
    /// see `RootView::display_name`.
    pub name: SharedString,
}

/// What a pane asks of whoever owns it: its controls, its player's bar and
/// menus, the palette's rows about it, and a press on it.
///
/// One vocabulary, so a new thing a pane can ask for is a variant here and an
/// arm in `RootView::on_pane_action`, rather than one more callback threaded
/// through `page`, `pane` and the header. Always sent with the pane's key,
/// never its position: a press can land after the panes have moved — one
/// closed, another opened — and an index read when the pane was drawn would
/// then name its neighbour (see [`pane_id`]). The player sends its own as
/// `VideoEvent::Pane`, which the root answers by the key it subscribed with.
#[derive(Clone, Debug)]
pub enum PaneAction {
    /// Close the pane: its header's ×.
    Close,
    /// Ask for its stream again: the pill a stopped pane offers.
    Retry,
    /// Make it the pane the keys talk to: a press anywhere in it, the band
    /// over its picture included.
    Activate,
    /// Show or hide its chat, as `C` does: the chat glyph on the bar, and
    /// on the header over a pane with no picture, which has no bar.
    ToggleChat,
    /// Open what it plays on twitch.tv, at the moment it is at: More's
    /// `Open on twitch.tv` row, and the palette's.
    OpenOnTwitch,
    /// Put the same link on the clipboard: More's, and the palette's.
    CopyLink,
}

/// How every pane in the current grid is arranged. Identical for all of them,
/// so it is computed once and passed down rather than recomputed per pane.
#[derive(Clone, Copy)]
struct PaneLayout {
    /// Chat under the video rather than beside it.
    portrait: bool,
    /// Beside: how wide chat is, with the video taking the rest.
    chat_width: f32,
    /// The cell itself, so a stacked pane can size its video box from the
    /// shape of its own stream — see [`layout::stacked_video_height`]. Below
    /// the video, chat takes the rest: the two arrangements are opposites on
    /// purpose, because a tall window is tall because you want more chat, not
    /// more letterboxing.
    cell_width: f32,
    cell_height: f32,
    /// A dragged share of the cell for the video, or zero to derive it.
    video_share: f32,
    /// Whether to mark the pane the keyboard is acting on. Only worth saying
    /// with more than one pane on screen: with one, it is the answer to a
    /// question nobody asked.
    mark_active: bool,
}

/// Where a drag of the video/chat divider started.
///
/// The pointer's position and which way the pane is split; the *sizes* it moves
/// come from settings, which is where they end up again — so the drag itself
/// holds nothing that has to be kept in step with anything.
#[derive(Clone, Copy)]
pub struct ResizeStart {
    pub origin: gpui::Point<Pixels>,
    pub portrait: bool,
    /// Which pane's divider was pulled. The share that results applies to
    /// every pane, but the drag has to start from where *this* pane's divider
    /// is, and with the box sized from each stream's shape that differs.
    pub index: usize,
}

/// The seam between video and chat, as something you can pull.
///
/// Six pixels of grab area, and none of them in the layout: the strip sits
/// astride the boundary, three pixels into the video and three into chat,
/// so video and chat touch. It used to be six pixels of the grid background
/// between them, which with the box sized to the picture was the only thing
/// left separating the two. A divider you can see is not the same thing as a
/// divider you can hit, and the pane gap elsewhere is three pixels precisely
/// because nothing is meant to grab *it*.
fn divider<V: 'static>(
    key: &str,
    index: usize,
    portrait: bool,
    on_resize: impl Fn(&mut V, ResizeStart, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let half = theme::DIVIDER_GRAB / 2.0;
    let handle = div()
        .id(pane_id(key, "divider"))
        .absolute()
        .map(|handle| {
            if portrait {
                handle
                    .left_0()
                    .right_0()
                    .top(px(-half))
                    .h(px(theme::DIVIDER_GRAB))
            } else {
                handle
                    .top_0()
                    .bottom_0()
                    .left(px(-half))
                    .w(px(theme::DIVIDER_GRAB))
            }
        })
        .cursor(if portrait {
            CursorStyle::ResizeUpDown
        } else {
            CursorStyle::ResizeLeftRight
        })
        .hover(|style| style.bg(theme::accent_dim()))
        .active(|style| style.bg(theme::accent()))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                on_resize(
                    view,
                    ResizeStart {
                        origin: event.position,
                        portrait,
                        index,
                    },
                    window,
                    cx,
                )
            }),
        );

    // Zero in the flow, so the two panes it separates meet. The grab strip is
    // positioned off this and reaches into both.
    div()
        .flex_none()
        .relative()
        .map(|seam| {
            if portrait {
                seam.w_full().h(px(0.))
            } else {
                seam.h_full().w(px(0.))
            }
        })
        .child(handle)
}

/// What goes where chat does: the chat, or — for a stacked pane that has
/// none to show — a word on why it is not there.
///
/// Stacked, a pane with chat hidden keeps the shape a pane with chat has: the
/// picture in the same box, and the space chat had under it. It does not hand
/// the picture the whole cell, which would put it out of line with every
/// neighbour the moment `C` was pressed. A cell stacks only when it is
/// narrower than `PORTRAIT_ASPECT`, which is narrower than any landscape
/// stream, so in the whole cell a landscape picture could be no wider than
/// the cell — and with the divider where it is derived, the box already
/// gives a 16:9 picture the cell's width, so the whole cell would buy it
/// only black above and below. A 4:3 stream in a cell just narrow enough to
/// stack is capped a few percent short of that, and a dragged divider keeps
/// the box the user chose, smaller or not: the divider still works with
/// chat hidden, and the share it sets is every pane's. The quality is chosen
/// against the pane's height regardless of chat, so it is not given up
/// either. A vertical stream is the exception, and is left capped.
///
/// No control here to bring chat back: the bar has one, over the picture,
/// and with no picture up the header over it does.
fn chat_or_why(slot: &Slot) -> AnyElement {
    match (&slot.chat, placement(slot)) {
        (Some(chat), Placement::Panel) => chat.clone().into_any_element(),
        (chat, _) => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::TEXT_META))
            .text_color(theme::text_dim())
            .child(if chat.is_some() {
                "Chat hidden · press C"
            } else {
                "No chat replay for this video"
            })
            .into_any_element(),
    }
}

/// One pane: a player, and its chat with a header.
///
/// Over the picture, on hover, go playback — the player's bar, with the
/// pane's chat, fullscreen and More — and, with no chat on screen, the
/// pane's header: its facts and its ×, on the band along the top. Over a
/// pane with no picture that band stays up at rest, since there is nothing
/// to keep clear. Nothing else is drawn on the picture.
#[allow(clippy::too_many_arguments)]
fn pane<V: 'static>(
    index: usize,
    slot: &Slot,
    info: &PaneInfo,
    layout: PaneLayout,
    active: bool,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    on_resize: impl Fn(&mut V, ResizeStart, &mut Window, &mut Context<V>) + 'static,
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> gpui::AnyElement {
    let video = match &slot.state {
        StreamState::Playing(view) => view.clone().into_any_element(),
        _ => status::screen(slot, &info.name, on_pane.clone(), cx),
    };

    // Built once, and drawn in one place: the panel, or the band over the
    // picture. Never both, since its elements' ids are the pane's.
    let placement = placement(slot);
    let header = header::pane_header(
        slot,
        info,
        placement,
        layout.mark_active && active,
        slot.has_picture(cx),
        window_hovered,
        on_pane.clone(),
        cx,
    );
    let (in_panel, on_picture) = match placement {
        Placement::Panel => (Some(header), None),
        Placement::OverPicture => (None, Some(header)),
    };

    // Where the pointer actually is, rather than what `on_hover` claims while
    // something in the window is being dragged. The listener below only exists
    // to wake a repaint so this runs again.
    let owner = cx.entity().downgrade();
    let hover_probe = canvas(
        move |bounds, window, cx| {
            let inside = window.is_window_hovered() && bounds.contains(&window.mouse_position());
            owner
                .update(cx, |view, cx| on_hover(view, index, inside, cx))
                .ok();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();

    // The header over the top of the picture, on the bar's wash: the band.
    // Mounted on every frame, empty when the header is in the panel, so its
    // fade never comes back after a frame without it and replays its last
    // flip (see `motion::Fade::apply`). It blocks the pointer only while it
    // is up — an invisible element still takes its hit test — and a press on
    // it does what a press anywhere in the pane does, makes it the active
    // one, which the cell below cannot hear through it. In the capture
    // phase, ahead of the header's own controls. Only the first press of a
    // run, for the cell's reason.
    //
    // Blocking the pointer hides it from the wake-up listeners under the
    // band too, the pane's and the player's, so while it is up the band has
    // one of its own. Without it, a pointer crossing onto or off the band
    // between two still panes woke nothing: the pane it came into never
    // became the active one, and the band and the bar it left stayed up.
    // Its own id, apart from the fade's (`band`), which wraps it.
    let band_key = slot.key.clone();
    let on_band = on_pane.clone();
    let band = div()
        .id(pane_id(&slot.key, "band-layer"))
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .when_some(on_picture, |band, header| {
            band.bg(theme::video_chrome())
                .pt(px(theme::GAP_TIGHT))
                .when(slot.header.is_visible(), |band| {
                    band.occlude()
                        .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
                        .capture_any_mouse_down(cx.listener(
                            move |view, event: &MouseDownEvent, window, cx| {
                                if event.click_count <= 1 {
                                    on_band(view, &band_key, PaneAction::Activate, window, cx);
                                }
                            },
                        ))
                })
                .child(header)
        });
    let band = slot
        .header
        .apply(pane_id(&slot.key, "band"), theme::MOTION_HOVER, band);

    // Below the video, the box is the shape of the stream, so chat starts
    // where the picture stops. 16:9 until the first frame says otherwise.
    let aspect = slot
        .video()
        .and_then(|view| view.read(cx).source_aspect())
        .unwrap_or(layout::VIDEO_ASPECT);
    let video_height = layout::stacked_video_height(
        layout.cell_width,
        layout.cell_height,
        aspect,
        layout.video_share,
    );

    // A flex container, so the player inside it is a flex item whose height is
    // the pane's height and nothing else. As a block, the player's `100%`
    // height did not resolve while the pane was being measured, and it fell
    // back to the aspect ratio gpui's `img` puts on its style from the frame
    // it holds - the frame rendered for the *previous* layout. The probe then
    // asked mpv for a frame that shape, which fixed the wrong height in
    // place: a stacked pane after a rail toggle showed its picture at four
    // fifths of the box, with black under it, until the app was restarted.
    let video_pane = div()
        .id(pane_id(&slot.key, "video"))
        .map(|pane| {
            // Stacked, the box is the stream's shape whether or not chat is
            // under it — see `chat_or_why` for why hiding chat does not
            // change that. Beside, the video takes whatever the cell leaves
            // it.
            if layout.portrait {
                pane.flex_none().h(px(video_height)).w_full()
            } else {
                pane.flex_1().min_w_0()
            }
        })
        .min_h_0()
        .overflow_hidden()
        .flex()
        .flex_col()
        .bg(theme::player_bg())
        .relative()
        .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
        .child(hover_probe)
        .child(video)
        .child(band);

    // Pointing at the video makes a pane active, which is right while you are
    // reaching for its controls and wrong the moment you go to read its chat:
    // the pointer sitting in one pane's messages left the *keyboard* still
    // talking to whichever video it crossed last. A click anywhere in the pane
    // — either half — says which one you mean, and says it deliberately.
    //
    // `on_mouse_down` rather than `on_click`, so it lands on the way down and
    // does not wait to find out whether the press was a click, a drag of the
    // volume slider or the start of a text selection. It does not consume the
    // event: a link in chat still opens, and the close button still closes.
    //
    // Only the first press of a run. The platform counts a second press near
    // the first as a double-click wherever the first one landed, and the
    // first can have been on a mini-player tile, which put this page under
    // the pointer with that tile's pane active: the second, landing in
    // whichever pane fills the corner the tile was in, would hand that pane
    // the keys instead. Any later press of a run is within a few pixels of
    // the first, so on a pane the first press already chose.
    let key = slot.key.clone();
    let cell = div().flex_1().min_w_0().min_h_0().flex().on_mouse_down(
        MouseButton::Left,
        cx.listener(move |view, event: &MouseDownEvent, window, cx| {
            if event.click_count <= 1 {
                on_pane(view, &key, PaneAction::Activate, window, cx);
            }
        }),
    );

    // Beside, a pane with no chat on screen is the picture and nothing else:
    // the picture takes chat's column, and the header is over its top. It
    // used to keep the header as a strip above the picture, since nothing
    // was drawn over the video at all — which cost the picture a strip of
    // its height, and moved it every time `C` was pressed.
    if placement == Placement::OverPicture && !layout.portrait {
        return cell.flex_row().child(video_pane).into_any_element();
    }

    let chat_pane = div()
        .flex()
        .flex_col()
        .pb(px(theme::GAP_TIGHT))
        .bg(theme::surface())
        .map(|pane| {
            if layout.portrait {
                // No padding above the header when it sits under the video:
                // the picture ends and the channel's name begins, with the
                // header's own bottom rule as the only line between them.
                pane.flex_1().min_h_0().w_full().pt(px(0.))
            } else {
                pane.flex_none()
                    .w(px(layout.chat_width))
                    .h_full()
                    .pt(px(theme::GAP_TIGHT))
            }
        })
        .children(in_panel)
        .child(div().flex_1().min_h_0().child(chat_or_why(slot)));

    cell.map(|cell| {
        if layout.portrait {
            cell.flex_col()
        } else {
            cell.flex_row()
        }
    })
    .child(video_pane)
    .child(divider(&slot.key, index, layout.portrait, on_resize, cx))
    .child(chat_pane)
    .into_any_element()
}

/// The whole watch page, laid out in the room the `body` says it has. See
/// [`layout::Body`] for why that is a type rather than the viewport.
///
/// `panes` is what the app knows about each slot, in the same order — see
/// [`PaneInfo`]. `window_hovered` is `window.is_window_hovered()`, which the
/// panes' headers give their tooltips by; see `header::pane_header`.
#[allow(clippy::too_many_arguments)]
pub fn page<V: 'static>(
    slots: &[Slot],
    panes: &[PaneInfo],
    body: layout::Body,
    chat_width: f32,
    video_share: f32,
    active: Option<usize>,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    on_resize: impl Fn(&mut V, ResizeStart, &mut Window, &mut Context<V>) + Clone + 'static,
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let (rows, cols) = layout::grid_shape(slots.len(), body.aspect());
    let cell = PaneLayout {
        portrait: layout::cell_is_portrait(layout::cell_aspect(body.aspect(), rows, cols)),
        // Clamped here rather than where it is stored: settings is a file
        // anyone can hand-edit, and a nonsense width would hide the video.
        chat_width: chat_width.clamp(theme::CHAT_WIDTH_MIN, theme::CHAT_WIDTH_MAX),
        cell_width: layout::cell_extent(body.width, cols),
        cell_height: layout::cell_extent(body.height, rows),
        video_share,
        mark_active: slots.len() > 1,
    };

    let mut grid = div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(theme::PANE_GAP))
        .bg(theme::bg());

    for row in 0..rows {
        let mut line = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .gap(px(theme::PANE_GAP));
        for col in 0..cols {
            let index = row * cols + col;
            let (Some(slot), Some(info)) = (slots.get(index), panes.get(index)) else {
                continue;
            };
            line = line.child(pane(
                index,
                slot,
                info,
                cell,
                active == Some(index),
                window_hovered,
                on_pane.clone(),
                on_resize.clone(),
                on_hover.clone(),
                cx,
            ));
        }
        grid = grid.child(line);
    }
    grid
}

#[cfg(test)]
mod tests {
    use twitch_api::VideoKind;

    use super::*;

    /// A pane over `source`, the way `Slot::new` makes one. No chat: nothing
    /// here reads it, and a chat is an entity, which needs a window.
    fn slot(key: String, source: Source, resume_at: f64) -> Slot {
        Slot::new(key, "forsen".into(), source, None, resume_at, false)
    }

    /// A channel's live pane, just asked for.
    pub(super) fn live() -> Slot {
        slot("forsen".into(), Source::Live, 0.0)
    }

    /// A pane on one of the channel's past broadcasts, just asked for and
    /// opening `resume_at` seconds in.
    pub(super) fn recording(resume_at: f64) -> Slot {
        let video = Video {
            id: "2868644730".into(),
            stream_id: Some("318576165606".into()),
            user_id: "22484632".into(),
            user_login: "forsen".into(),
            user_name: "Forsen".into(),
            title: "Games and stuff".into(),
            created_at: "2026-09-30T18:00:00Z".into(),
            length_secs: 4 * 3600,
            thumbnail_url: String::new(),
            view_count: 0,
            kind: VideoKind::Archive,
            muted_segments: Vec::new(),
        };
        slot(
            Slot::video_key(&video.id),
            Source::Video {
                video: Box::new(video),
                position: PositionHandle::starting_at(resume_at),
            },
            resume_at,
        )
    }

    /// Whatever opened it, a new pane is waiting for its stream and has
    /// nothing of its own running yet: `start_stream` gives it those, and
    /// everything else a pane picks up — a quality from its menu, the
    /// pointer, Mute all — comes later.
    #[test]
    fn a_new_slot_is_starting_with_nothing_running() {
        for slot in [live(), recording(42.0)] {
            assert!(matches!(slot.state, StreamState::Starting));
            assert!(slot.supervisor.is_none() && slot.pump.is_none());
            assert!(slot.video().is_none());
            assert_eq!(slot.quality_override, None);
            assert!(!slot.hovered && !slot.quiet && !slot.revealed);
            assert!(!slot.header.is_visible());
            assert_eq!(slot.stalled_at, None);
        }
        assert_eq!(recording(42.0).resume_at, 42.0);
        assert_eq!(recording(42.0).key, "vod:2868644730");
    }

    /// Every state with nothing playing notes when it began, which is what a
    /// later poll compares a broadcast's start against; asking again clears
    /// it.
    #[test]
    fn set_state_notes_when_a_pane_stalls() {
        let mut slot = live();
        for stopped in [
            StreamState::Offline,
            StreamState::Ended,
            StreamState::Failed("streamlink exited".into()),
        ] {
            slot.set_state(StreamState::Starting);
            assert_eq!(slot.stalled_at, None);
            slot.set_state(stopped);
            assert!(slot.stalled_at.is_some());
        }
        slot.set_state(StreamState::Starting);
        assert_eq!(slot.stalled_at, None, "asking again is not a stall");
    }

    /// A live pane hands out its channel, with or without a moment asked
    /// for: there is no moment in a broadcast still going to give.
    #[test]
    fn a_live_pane_links_its_channel() {
        for at_position in [false, true] {
            assert_eq!(live().link(at_position), "https://www.twitch.tv/forsen");
        }
    }

    /// A recording hands out where it is, which reads back as that moment.
    #[test]
    fn a_recording_links_where_it_is() {
        let link = recording(3723.0).link(true);
        assert_eq!(link, "https://www.twitch.tv/videos/2868644730?t=1h2m3s");
        assert_eq!(
            target::parse(&link),
            Some(Target::Video {
                id: "2868644730".into(),
                start_secs: Some(3723),
            })
        );
    }

    /// In its first second a recording is at its start, and its link says
    /// so by saying no time at all.
    #[test]
    fn a_recording_at_the_top_has_no_time() {
        for position in [0.0, 0.6] {
            assert_eq!(
                recording(position).link(true),
                "https://www.twitch.tv/videos/2868644730"
            );
        }
    }

    /// The header's name says which recording, never when: its link is the
    /// recording from the top, however far in the pane is.
    #[test]
    fn the_header_link_never_carries_a_time() {
        assert_eq!(
            recording(3723.0).link(false),
            "https://www.twitch.tv/videos/2868644730"
        );
    }

    /// Every combination of what the band follows, for the rules that hold
    /// across all of them: `(inside, revealed, picture, menu_open)`.
    fn every_moment() -> impl Iterator<Item = (bool, bool, bool, bool)> {
        (0..16).map(|bits| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0))
    }

    /// Chat on screen has a panel, and the header sits on it.
    #[test]
    fn with_chat_the_header_sits_in_the_panel() {
        assert_eq!(Placement::of(false, true), Placement::Panel);
    }

    /// Hidden chat leaves no panel, in either arrangement, so the header
    /// goes over the picture rather than taking a strip of its own.
    #[test]
    fn hidden_chat_puts_the_header_on_the_picture() {
        assert_eq!(Placement::of(true, true), Placement::OverPicture);
    }

    /// A recording with no chat to replay has no panel either, whatever the
    /// channel's chat was saved as.
    #[test]
    fn a_chatless_recording_places_its_header_like_hidden_chat() {
        for hidden in [false, true] {
            assert_eq!(Placement::of(hidden, false), Placement::OverPicture);
        }
        assert_eq!(placement(&recording(0.0)), Placement::OverPicture);
    }

    /// Over a picture the header is up only while the pointer is on it, and
    /// goes with the pointer.
    #[test]
    fn the_band_waits_for_the_pointer_over_a_picture() {
        let mut pane = recording(0.0);
        pane.point(false, true, false);
        assert!(
            !pane.header.is_visible(),
            "up over a picture nobody pointed at"
        );
        assert!(pane.point(true, true, false).changed);
        assert!(
            pane.header.is_visible(),
            "the pointer came in and it stayed down"
        );
        assert!(pane.point(false, true, false).changed);
        assert!(
            !pane.header.is_visible(),
            "it stayed up after the pointer left"
        );
    }

    /// A pane with no picture has nothing to keep clear, and says whose it
    /// is and offers its × with the pointer anywhere.
    #[test]
    fn the_band_stays_up_over_a_status_screen() {
        let mut pane = recording(0.0);
        pane.set_state(StreamState::Offline);
        for inside in [false, true] {
            pane.point(inside, false, false);
            assert!(pane.header.is_visible(), "inside: {inside}");
        }
    }

    /// An open menu is what was asked for last; the header gives it the
    /// picture, pointer or no pointer.
    #[test]
    fn the_band_steps_aside_for_an_open_menu() {
        let mut pane = recording(0.0);
        pane.point(true, true, false);
        assert!(pane.header.is_visible());
        assert!(pane.point(true, true, true).changed);
        assert!(!pane.header.is_visible());
    }

    /// A header in the panel is always on screen there, so nothing raises
    /// it over the picture as well.
    #[test]
    fn a_panel_header_never_raises_the_band() {
        for (inside, revealed, picture, menu_open) in every_moment() {
            assert!(
                !band_wanted(Placement::Panel, inside, revealed, picture, menu_open),
                "inside {inside}, revealed {revealed}, picture {picture}, menu {menu_open}"
            );
        }
    }

    /// The pointer coming into a pane is said once, on the way in, and
    /// again only after it has left. A pane already counted as pointed at —
    /// every pane, after a mini-player tile's click — reports no entry but
    /// still raises its header.
    #[test]
    fn coming_in_is_reported_once() {
        let mut pane = recording(0.0);
        assert!(pane.point(true, true, false).entered);
        assert!(!pane.point(true, true, false).entered);
        assert!(!pane.point(false, true, false).entered);
        assert!(pane.point(true, true, false).entered);
        assert_eq!(
            pane.point(true, true, false),
            Pointed {
                entered: false,
                changed: false,
            },
            "a pointer moving within the pane changes nothing"
        );

        let mut pointed_already = recording(0.0);
        pointed_already.hovered = true;
        let pointed = pointed_already.point(true, true, false);
        assert!(
            !pointed.entered,
            "a tile's click must keep its own pane active"
        );
        assert!(pointed.changed && pointed_already.header.is_visible());
    }

    /// A pane key's reveal brings the header up with the pointer elsewhere,
    /// over a picture, and it goes when the reveal ends.
    #[test]
    fn a_revealed_band_shows_without_the_pointer() {
        let mut pane = recording(0.0);
        pane.revealed = true;
        pane.point(false, true, false);
        assert!(pane.header.is_visible());
        pane.revealed = false;
        assert!(pane.point(false, true, false).changed);
        assert!(!pane.header.is_visible());
    }

    /// Not even a reveal covers an open menu.
    #[test]
    fn a_reveal_never_covers_an_open_menu() {
        for (inside, revealed, picture, _) in every_moment() {
            assert!(
                !band_wanted(Placement::OverPicture, inside, revealed, picture, true),
                "inside {inside}, revealed {revealed}, picture {picture}"
            );
        }
        let mut pane = recording(0.0);
        pane.revealed = true;
        pane.point(false, true, true);
        assert!(!pane.header.is_visible());
    }
}
