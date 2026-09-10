//! The two pages, assembled: the browse page with its header and lists, and
//! the watch page with its grid of panes. Each is one function that hands the
//! root's state to the module that draws it.

use gpui::{div, prelude::*, px, Context, IntoElement, Window};
use gpui_component::input::Input;
use twitch_api::LiveStream;

use super::RootView;
use crate::browse::Tab;
use crate::{browse, sidebar, theme, watch, APP_NAME};

impl RootView {
    pub(super) fn browse_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let watching = self.slots.len();
        let header = div()
            .w_full()
            .flex_none()
            // Fixed rather than however tall its contents happen to be: the
            // toast stack is anchored to the window and has to clear this, and
            // one constant read by both is what makes them agree.
            .h(px(theme::HEADER_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP))
            .px(px(theme::PAGE_PAD))
            .border_b_1()
            .border_color(theme::border())
            // Only when the rail is folded away. Open, it has its own control,
            // and two of them would be two things that do one thing.
            .when(self.settings.sidebar_collapsed, |header| {
                header.child(sidebar::expand(
                    |this: &mut RootView, window, cx| this.toggle_sidebar(window, cx),
                    cx,
                ))
            })
            .child(
                div()
                    .text_size(px(theme::TEXT_TITLE))
                    .font_weight(theme::weight_title())
                    .text_color(theme::text())
                    .child(APP_NAME),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_dim())
                    .child(self.sign_in.summary()),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .gap(px(theme::GAP_TIGHT))
                    .children(Tab::ALL.map(|tab| self.tab_pill(tab, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .w(px(260.))
                    .child(Input::new(&self.search).cleanable(true)),
            )
            .when(watching > 0, |header| {
                header.child(self.pill(
                    "resume",
                    format!("watching {watching}").into(),
                    cx,
                    |this, _window, cx| this.go_watch(cx),
                ))
            })
            .child(self.pill(
                "refresh",
                if self.refreshing {
                    "refreshing…".into()
                } else {
                    "refresh".into()
                },
                cx,
                |this, _window, cx| this.refresh(cx),
            ))
            .child(self.pill(
                "open-settings",
                "settings".into(),
                cx,
                |this, window, cx| this.toggle_settings(window, cx),
            ));

        let width = self.body(window).width;
        // Whether the channel whose page is open is on right now, which is
        // what its bar offers beside the recordings: the stream, or the chat.
        let channel_live = self
            .discovery
            .channel
            .as_ref()
            .is_some_and(|page| self.stream_info(&page.login).is_some());

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(theme::bg())
            .children(self.follows_rail(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(browse::page(
                        &self.follows,
                        &self.offline,
                        self.filter.read(cx).value().as_ref(),
                        Input::new(&self.filter).cleanable(true).into_any_element(),
                        &self.discovery,
                        &self.sign_in,
                        self.follows_loaded,
                        width,
                        &self.cache,
                        self.can_add(),
                        &self.scrolls,
                        channel_live,
                        |this: &mut RootView, action, window, cx| {
                            this.on_browse_action(action, window, cx)
                        },
                        cx,
                    ))
                    .children(self.now_playing(cx)),
            )
    }

    pub(super) fn watch_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Resolved here, for the panes on screen only: `stream_info` walks
        // every list the app holds, and doing that per pane per frame inside
        // the page would be the same walk four times over.
        let info: Vec<Option<&LiveStream>> = self
            .slots
            .iter()
            .map(|slot| {
                // A recording's header speaks for the recording; the live
                // numbers would be about a different broadcast.
                if slot.is_live() {
                    self.stream_info(&slot.channel)
                } else {
                    None
                }
            })
            .collect();
        let grid = div()
            .flex_1()
            .min_w_0()
            .relative()
            .child(watch::page(
                &self.slots,
                &info,
                self.body(window),
                self.settings.chat_width,
                self.settings.video_share,
                self.active_slot(),
                |this: &mut RootView, index, window, cx| this.close_slot(index, window, cx),
                |this: &mut RootView, index, window, cx| {
                    if let Some(key) = this.slots.get(index).map(|slot| slot.key.clone()) {
                        this.retry_stream(&key, window, cx);
                    }
                },
                |this: &mut RootView, index, cx| {
                    let Some(key) = this.slots.get(index).map(|slot| slot.key.clone()) else {
                        return;
                    };
                    if this.active.as_deref() != Some(key.as_str()) {
                        this.active = Some(key);
                        cx.notify();
                    }
                },
                |this: &mut RootView, start, window, cx| this.start_resize(start, window, cx),
                |this: &mut RootView, index, hovered, cx| {
                    // Only repaint when the pointer crosses a boundary; most
                    // moves are within the pane it is already in.
                    match this.slots.get_mut(index) {
                        Some(slot) if slot.hovered != hovered => slot.hovered = hovered,
                        _ => return,
                    }
                    // Sticky, unlike `hovered`: a keyboard shortcut has to keep
                    // working once the pointer has moved into chat or off the
                    // window entirely, and the pane you last looked at is the
                    // one you meant.
                    if hovered {
                        this.active = this.slots.get(index).map(|slot| slot.key.clone());
                    }
                    let over_video = this.slots.iter().any(|slot| slot.hovered);
                    this.nav.set(over_video);
                    cx.notify();
                },
                cx,
            ))
            .child(
                // The only page-level control on the watch page, in the corner
                // a back control belongs in, and revealed by the same gesture
                // as everything else: point at the video and the controls come
                // up, look away and the picture is all that is left.
                //
                // Settings is not here on purpose: it is set once and forgotten,
                // and per-stream quality already lives in the control bar. It is
                // on the follows page, one click away.
                self.nav.apply(
                    "watch-nav",
                    theme::MOTION_HOVER,
                    div()
                        .absolute()
                        .top(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        // Pinned to the width the pane header keeps clear for
                        // it, so it cannot grow past the space reserved.
                        .w(px(theme::NAV_RESERVE))
                        .flex()
                        .flex_row()
                        .gap(px(theme::GAP_TIGHT))
                        .child(
                            self.pill("back", "← follows".into(), cx, |this, _window, cx| {
                                this.go_browse(cx)
                            }),
                        )
                        // Bringing the rail back is chrome like everything else
                        // here: it comes up with the video controls and goes
                        // away with them, so a window left on one stream stays
                        // the stream. Beside the back pill rather than in the
                        // opposite corner, because that corner is chat's.
                        .when(self.settings.sidebar_collapsed, |nav| {
                            nav.child(sidebar::expand(
                                |this: &mut RootView, window, cx| this.toggle_sidebar(window, cx),
                                cx,
                            ))
                        }),
                ),
            );

        div()
            .size_full()
            .flex()
            .flex_row()
            .children(self.follows_rail(cx))
            .child(grid)
    }
}
