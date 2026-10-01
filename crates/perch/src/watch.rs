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
//! screen drawn from it. Whatever a pane asks of its owner — a close, a retry,
//! a press that makes it the active one — is a [`PaneAction`], addressed by
//! the pane's key.

mod status;

use chrono::{DateTime, Utc};
use gpui::{
    canvas, div, prelude::*, px, AnyElement, Context, CursorStyle, ElementId, Entity, IntoElement,
    MouseButton, MouseDownEvent, Pixels, SharedString, Task, Window,
};
use streamlink::StreamSupervisor;

use twitch_api::{LiveStream, Video};

pub use self::status::{showing, Showing};

use crate::browse;
use crate::channel_page;
use crate::chat::ChatView;
use crate::controls;
use crate::layout;
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
    /// Kept only to tell when the pointer comes into a pane: that rising edge
    /// is what makes the pane the one the keys talk to.
    pub hovered: bool,
    /// Whether this pane's chat is hidden, so the video has the whole cell.
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
    /// Close the pane: its header's close.
    Close,
    /// Ask for its stream again: the pill a stopped pane offers.
    Retry,
    /// Make it the pane the keys talk to: a press anywhere in it.
    Activate,
    /// Show or hide its chat, as `C` does: the chat glyph on the bar.
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

/// Everything true about a stream that is not playback: who it is, how many
/// people are there, how long it has been going.
///
/// This lives above chat rather than over the video. It is static information,
/// and static information on a moving picture is the thing you end up staring
/// past for three hours. Chat is already a panel, so it costs nothing here.
fn chat_header<V: 'static>(
    slot: &Slot,
    pane: &PaneInfo,
    active: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let key = slot.key.clone();
    let recording = slot.recording();
    let info = pane.stream;
    let name = pane.name.clone();

    // Both are absent for a channel opened by name that you do not follow:
    // the follows poll is where these numbers come from, and it only knows
    // about channels you follow. The same shape as the browse card's overlay,
    // "358 · 8h 20m", and for the same reason: the live dot beside it already
    // says what the first number counts, and a header 340px wide has no room
    // to say it again in words once `muted` has to fit too.
    let meta = info
        .into_iter()
        .flat_map(|stream| {
            [
                Some(browse::format_viewers(stream.viewer_count)),
                browse::uptime(&stream.started_at),
            ]
        })
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    // What is actually on, which this header has never said: it knew who you
    // were watching and how many others were, and not a word about what they
    // were doing. The title and the game share one line, joined like `meta`
    // above, rather than taking one each — a pane header is 340px wide, and a
    // row of chrome here costs a row of chat in every pane on the page.
    //
    // A title is routinely longer than that line, so the whole of it — and the
    // game under it — is a hover away, through the same builder the browse
    // cards use.
    //
    // For a recording the second line is its title and when it was, in place
    // of a live stream's title and game: the game is not something Helix
    // says about a video, and the date is what tells two recordings apart.
    let about: Vec<SharedString> = match recording {
        Some(video) => vec![
            video.title.clone(),
            channel_page::describe(video, chrono::Utc::now()),
        ],
        None => info
            .into_iter()
            .flat_map(|stream| [stream.title.clone(), stream.game_name.clone()])
            .collect(),
    }
    .into_iter()
    .map(SharedString::from)
    .filter(|text| !text.trim().is_empty())
    .collect();
    let about_line = about
        .iter()
        .map(SharedString::as_ref)
        .collect::<Vec<_>>()
        .join(" · ");

    // Whether this pane is showing a picture, which is not the same as whether
    // it exists: an offline or failed pane used to draw the app's only
    // saturated red beside its name while the video underneath said the channel
    // was not streaming. A recording never gets the dot: nothing about it is
    // happening now.
    let playing = matches!(slot.state, StreamState::Playing(_)) && slot.is_live();
    // A stream that has finished takes its live numbers with it. They come
    // from a list that will not know for up to a minute, and an uptime that
    // goes on counting beside "ended the stream" is the same lie the frozen
    // last frame used to tell.
    let ended = matches!(slot.state, StreamState::Ended);
    // What the player is doing, read off the view rather than copied onto the
    // slot, so there is one source. These used to be visible only while the
    // pointer was over the video: a channel saved muted opened silent with
    // nothing on screen to say so, and a paused pane looked like a stalled
    // stream. The header is the one static place a pane has, so they go here.
    // The quality does not: it is on the control bar, and the header has no
    // room for a fourth thing.
    let (muted, paused) = slot
        .video()
        .map(|view| {
            let player = view.read(cx);
            (player.is_muted(), player.is_paused())
        })
        .unwrap_or((false, false));
    // The name opens what the pane is playing: the channel, or this one
    // recording, from its start — the name says which, never when. The
    // moment is More's to offer, on the bar.
    let url = slot.link(false);
    let tooltip = match recording {
        Some(_) => SharedString::from("Open this broadcast on twitch.tv"),
        None => SharedString::from(format!("Open twitch.tv/{}", slot.channel)),
    };

    div()
        .flex_none()
        .w_full()
        .flex()
        // A column now, because what is on is a line of its own under who is
        // on. It does not fit beside them: the first row is already the name,
        // the numbers, `muted`, `paused` and the close button.
        .flex_col()
        .gap(px(theme::GAP_WORD))
        .px(px(theme::ROW_PAD_X))
        .pb(px(theme::GAP_TIGHT))
        .border_b_1()
        // Which pane the keyboard is talking to. `Space`, `M`, the arrows and
        // `Ctrl+W` all act on the pane you last pointed at, and with four on
        // screen nothing said which that was — so every press was a guess. One
        // line under one header, and only when there is more than one pane to
        // tell apart.
        .border_color(if active {
            theme::accent()
        } else {
            theme::border()
        })
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_TIGHT))
                .when(playing, |header| {
                    // The same dot the browse cards use, for the same reason:
                    // it says the numbers beside it are live rather than a
                    // playback position.
                    header.child(controls::live_dot())
                })
                .child(
                    // Chat here is read-only by design. This is the way out of that:
                    // the one thing the app deliberately cannot do, one click from the
                    // name of the channel you would be saying it in.
                    div()
                        .id(pane_id(&slot.key, "open"))
                        .flex_none()
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text())
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme::accent()))
                        .tooltip(controls::tip(tooltip))
                        .on_click(cx.listener(move |_, _event, _window, cx| cx.open_url(&url)))
                        .child(name),
                )
                // Said where `muted` and `paused` are said, and for the same
                // reason: it is a fact about the pane that the picture alone
                // does not carry.
                // Which kind of recording, since the three play alike:
                // `replay` for a broadcast, and a highlight or an upload by
                // its own name.
                .when_some(recording, |header, video| {
                    header.child(controls::tag(channel_page::kind_tag(video.kind)))
                })
                .when(!ended && !meta.is_empty(), |header| {
                    header.child(
                        // `text_ellipsis` plus `line_clamp`, not `truncate`:
                        // see the handoff on why the latter clips mid-glyph in
                        // a flex row with no definite width, which is what this
                        // row is.
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .line_clamp(1)
                            .text_size(px(theme::TEXT_META))
                            .text_color(theme::text_muted())
                            .child(SharedString::from(meta)),
                    )
                })
                .when(muted, |header| header.child(controls::tag("muted")))
                .when(paused, |header| header.child(controls::tag("paused")))
                .child(div().flex_1())
                // On every pane, a lone one included. It used to go when only
                // one pane was left, while `Ctrl+W` went on closing that one —
                // a control the keyboard had and the pointer did not.
                .child(
                    controls::destructive(pane_id(&slot.key, "close"), "Close").on_click(
                        cx.listener(move |view, _event, window, cx| {
                            on_pane(view, &key, PaneAction::Close, window, cx)
                        }),
                    ),
                ),
        )
        .when(!about.is_empty(), |header| {
            header.child(
                // `w_full`, not `min_w_0`: this one is a child of a flex
                // column, where a definite width is what the measure pass
                // needs before it will ellipsise at all.
                div()
                    .id(pane_id(&slot.key, "about"))
                    .w_full()
                    .text_ellipsis()
                    .line_clamp(1)
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_TIGHT))
                    .text_color(theme::text_dim())
                    .tooltip(controls::full_text(about))
                    .child(SharedString::from(about_line)),
            )
        })
}

/// What goes where chat does: the chat, or — for a stacked pane that has
/// none to show — a word on why it is not there.
///
/// Stacked, a pane with chat hidden keeps the shape a pane with chat has: the
/// picture in the same box, its header under it. It used to put the header on
/// top and centre the picture in the rest, which for a 16:9 stream in a tall
/// cell bought nothing but black above and below it — the picture is as wide
/// as the cell either way — and left its header at a different height from
/// every neighbour's.
fn chat_or_why(slot: &Slot, chatless: bool) -> AnyElement {
    match (&slot.chat, chatless) {
        (Some(chat), false) => chat.clone().into_any_element(),
        (chat, _) => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::TEXT_META))
            .text_color(theme::text_dim())
            .child(if chat.is_some() {
                "chat hidden · press C"
            } else {
                "no chat replay for this video"
            })
            .into_any_element(),
    }
}

/// One pane: a player, and its chat with a header.
///
/// Nothing static is drawn over the video. What appears there on hover is
/// the player's bar: playback, the pane's chat, fullscreen and More.
#[allow(clippy::too_many_arguments)]
fn pane<V: 'static>(
    index: usize,
    slot: &Slot,
    info: &PaneInfo,
    layout: PaneLayout,
    active: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    on_resize: impl Fn(&mut V, ResizeStart, &mut Window, &mut Context<V>) + 'static,
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> gpui::AnyElement {
    let video = match &slot.state {
        StreamState::Playing(view) => view.clone().into_any_element(),
        _ => status::screen(slot, &info.name, on_pane.clone(), cx),
    };

    let header = chat_header(
        slot,
        info,
        layout.mark_active && active,
        on_pane.clone(),
        cx,
    );

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
    // A pane with no chat to show lays out the way hidden chat does.
    let chatless = slot.chat_hidden || slot.chat.is_none();
    let video_pane = div()
        .id(pane_id(&slot.key, "video"))
        .map(|pane| {
            // Stacked, the box is the stream's shape whether or not chat is
            // under it — see below for why hiding chat does not change that.
            // Beside, the video takes whatever the cell leaves it.
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
        .child(video);

    // With chat hidden the cell is a column whatever the grid shape, because
    // the only thing left beside the video is the header strip.
    //
    // That strip stays rather than going with the chat it used to sit on. It
    // carries the channel's name and the close button, and nothing is drawn
    // over the video on purpose — so losing it would leave a pane with no
    // identity and no way to close it but the keyboard.
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

    if chatless && !layout.portrait {
        // Beside the video, hiding chat gives the video chat's column: the
        // cell becomes the header strip over the picture.
        //
        // That strip stays rather than going with the chat it used to sit on:
        // it carries the channel's name and the close button, and nothing is
        // drawn over the video on purpose — so dropping it would leave a pane
        // with no identity and no way to close it but the keyboard.
        return cell
            .flex_col()
            .bg(theme::surface())
            .child(
                // The same vertical padding chat used to give it. Without this
                // the header sits flush against the top of the cell and the top
                // of the video, and the pane reads as clipped rather than as
                // deliberately bare.
                div()
                    .flex_none()
                    .w_full()
                    .py(px(theme::GAP_TIGHT))
                    .child(header),
            )
            .child(video_pane)
            .into_any_element();
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
        .child(header)
        .child(div().flex_1().min_h_0().child(chat_or_why(slot, chatless)));

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
/// [`PaneInfo`].
#[allow(clippy::too_many_arguments)]
pub fn page<V: 'static>(
    slots: &[Slot],
    panes: &[PaneInfo],
    body: layout::Body,
    chat_width: f32,
    video_share: f32,
    active: Option<usize>,
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
            assert!(!slot.hovered && !slot.quiet);
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
}
