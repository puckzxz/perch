//! The shell's own furniture: the pills, the toasts, the now-playing bar
//! along the bottom of the browse page, and the follows rail. None of it is a
//! page; all of it is drawn by one.

use std::time::Duration;

use gpui::{div, prelude::*, px, Context, ElementId, IntoElement, SharedString, Window};

use super::{Page, RootView, Toast, ToastAction};
use crate::browse::Tab;
use crate::{channel_page, controls, motion, sidebar, theme};

/// How long a toast stays up: time to read a "went live" and reach for it,
/// or to take back a forget.
const TOAST_LIFETIME: Duration = Duration::from_secs(8);

/// Thumbnail width in the now-playing bar. Small on purpose, and not only for
/// the room: render size follows the element, so a stream shown this big decodes
/// into a buffer this big.
const MINI_WIDTH: f32 = 96.0;

impl RootView {
    pub(super) fn toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_with(text, None, cx);
    }

    /// A notice that can be acted on: the text opens what it names.
    pub(super) fn toast_with(
        &mut self,
        text: impl Into<SharedString>,
        action: Option<ToastAction>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_toast;
        self.next_toast += 1;
        self.toasts.push(Toast {
            id,
            text: text.into(),
            fade: motion::Fade::entering(),
            action,
        });

        // Two stages, because dropping the element is what stops it being
        // drawn: start the fade at the end of the lifetime, and only remove it
        // once the fade has had time to run.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_LIFETIME).await;
            let _ = this.update(cx, |this: &mut RootView, cx| {
                if let Some(toast) = this.toasts.iter_mut().find(|toast| toast.id == id) {
                    toast.fade.set(false);
                    cx.notify();
                }
            });
            cx.background_executor().timer(theme::MOTION_ENTER).await;
            let _ = this.update(cx, |this: &mut RootView, cx| {
                this.toasts.retain(|toast| toast.id != id);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The rail, and everything it needs to know about what is already open.
    pub(super) fn follows_rail(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        // Folded away, the rail holds nothing still; see `hold_live`.
        if self.settings.sidebar_collapsed {
            self.hold_live(super::follows::LiveList::Rail, false, cx);
        }
        // Live panes only: a recording open beside the rail does not make
        // its channel's row "watching".
        let watching: Vec<String> = self
            .slots
            .iter()
            .filter(|slot| slot.is_live())
            .map(|slot| slot.channel.clone())
            .collect();
        sidebar::rail(
            &self.follows,
            &self.avatars,
            &watching,
            self.can_add(),
            self.settings.sidebar_collapsed,
            &self.cache,
            &self.rail_scroll,
            |this: &mut RootView, window, cx| this.toggle_sidebar(window, cx),
            |this: &mut RootView, action, window, cx| this.on_browse_action(action, window, cx),
            cx,
        )
    }

    /// One of the shell's controls, wired to a method on this view.
    ///
    /// The styling lives in [`controls`]; what is left here is the listener,
    /// which needs `cx` and so cannot.
    pub(super) fn pill(
        &self,
        id: &'static str,
        label: SharedString,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        controls::pill(id, label, controls::Variant::Pill)
            .on_click(cx.listener(move |this, _event, window, cx| on_click(this, window, cx)))
    }

    /// A pill that says whether it is the list you are looking at.
    ///
    /// None is while search results are up: they come from the box in the
    /// header, not from a tab, and the tab left lit behind them read as though
    /// the results were part of it — the history's, as often as not.
    pub(super) fn tab_pill(&self, tab: Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let variant = if self.discovery.tab == tab && self.discovery.search.is_none() {
            controls::Variant::Selected
        } else {
            controls::Variant::Pill
        };
        controls::pill(tab.label(), tab.label(), variant)
            .on_click(cx.listener(move |this, _event, window, cx| this.show_tab(tab, window, cx)))
    }

    /// Transient notices, top-right.
    ///
    /// Offset below the browse header rather than pinned to the window, because
    /// that corner is not empty there: the search box and the refresh and
    /// settings pills are in it, and a "went live" toast landed squarely on top
    /// of them. The watch page has no header, so there the offset is nothing.
    pub(super) fn toast_stack(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let top = match self.page {
            Page::Browse => theme::HEADER_HEIGHT + theme::GAP_TIGHT,
            Page::Watch => theme::GAP,
        };

        let mut stack = div()
            .absolute()
            .top(px(top))
            .right(px(theme::GAP))
            .flex()
            .flex_col()
            .gap(px(theme::GAP_TIGHT))
            .items_end();

        let can_add = self.can_add();
        for toast in &self.toasts {
            let id = toast.id;
            // The text opens what the toast names, when it names something:
            // the same lift under the pointer a channel's name gets in a
            // pane header, so it reads as the link it is.
            let watch = matches!(toast.action, Some(ToastAction::Watch(_)));
            let undo = matches!(toast.action, Some(ToastAction::Undo(_)));
            let text = if watch {
                div()
                    .id(("toast-open", id))
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::accent()))
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.act_on_toast(id, true, window, cx)
                    }))
                    .child(toast.text.clone())
                    .into_any_element()
            } else {
                div().child(toast.text.clone()).into_any_element()
            };
            let card = div()
                // Per card rather than on the stack: the stack is
                // `items_end`, so its box is as wide as the widest
                // toast and would blanket the search box beside it.
                .block_mouse_except_scroll()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP))
                .px(px(theme::PANEL_PAD))
                .py(px(theme::GAP))
                .rounded(px(theme::RADIUS_LG))
                .bg(theme::surface_raised())
                .border_l_2()
                .border_color(theme::accent())
                .shadow_lg()
                .text_size(px(theme::TEXT_META))
                .line_height(px(theme::LINE_BODY))
                .text_color(theme::text())
                .child(text)
                // Beside what is playing, when something is: the same offer
                // a card makes, in the same words.
                .when(can_add && watch, |card| {
                    card.child(
                        controls::pill(("toast-add", id), "+ add", controls::Variant::Pill)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.act_on_toast(id, false, window, cx)
                            })),
                    )
                })
                .when(undo, |card| {
                    card.child(
                        controls::pill(("toast-undo", id), "undo", controls::Variant::Pill)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.act_on_toast(id, true, window, cx)
                            })),
                    )
                });
            stack = stack.child(toast.fade.apply(("toast", id), theme::MOTION_ENTER, card));
        }
        stack
    }

    /// Do what toast `id` offers — watch alone or add beside, or undo — and
    /// take the toast down, since it has been answered. `solo` means nothing
    /// to an undo.
    fn act_on_toast(&mut self, id: u64, solo: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.toasts.iter().position(|toast| toast.id == id) else {
            return;
        };
        let toast = self.toasts.remove(index);
        match toast.action {
            Some(ToastAction::Watch(login)) => self.open_channel(login, solo, window, cx),
            Some(ToastAction::Undo(forgotten)) => self.restore_history(forgotten, cx),
            None => {}
        }
        cx.notify();
    }

    /// What is playing while you browse: a bar along the bottom of the page,
    /// or nothing at all when the miniplayer is turned off — in which case
    /// there is nothing playing to put in it.
    ///
    /// This was a floating strip of 220px thumbnails in the bottom-right
    /// corner, which worked at one window size. At 1000px it covered two cards;
    /// four streams would have been 900px of tiles laid over the bottom row of
    /// the grid — and every one of them needed its own `block_mouse` so that
    /// clicking a thumbnail did not also open whatever card was underneath it.
    ///
    /// Docked, none of that is true: the grid ends where the bar begins, the
    /// bar is a row rather than a wall, and each stream gets its own close
    /// button, which the floating version never had room for — the only way
    /// out of a stream from this page was to stop all of them.
    pub(super) fn now_playing(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.slots.is_empty() || !self.settings.miniplayer {
            return None;
        }

        let mut bar = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP))
            .px(px(theme::PAGE_PAD))
            .py(px(theme::GAP_TIGHT))
            .bg(theme::surface())
            .border_t_1()
            .border_color(theme::border());

        for (index, slot) in self.slots.iter().enumerate() {
            let id = ElementId::from(SharedString::from(format!("mini-{}", slot.key)));
            // The name the pane header uses; see `display_name`.
            let name = self.display_name(slot);
            // Under it, what is on, as the pane header's second line says it:
            // the game for a stream, and that a recording is one. Every entry
            // used to say "muted", which was true of all of them at once and
            // is said once now, beside the controls.
            let about = match slot.recording() {
                Some(video) => channel_page::kind_tag(video.kind).to_string(),
                None => self
                    .stream_info(&slot.channel)
                    .map(|stream| stream.game_name.clone())
                    .unwrap_or_default(),
            };
            let mut entry = div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_TIGHT))
                .pr(px(theme::GAP_TIGHT))
                .rounded(px(theme::RADIUS))
                .cursor_pointer()
                .hover(|style| style.bg(theme::hover()))
                .active(|style| style.bg(theme::pressed()))
                .on_click(cx.listener(|this, _event, _window, cx| this.go_watch(cx)));

            // A pane still starting has no picture yet, and a placeholder the
            // same size keeps the bar from reflowing when it arrives.
            entry = entry.child(
                div()
                    .flex_none()
                    .w(px(MINI_WIDTH))
                    .h(px(MINI_WIDTH * 9.0 / 16.0))
                    .rounded(px(theme::RADIUS))
                    .overflow_hidden()
                    .bg(theme::player_bg())
                    .children(slot.video().cloned()),
            );

            entry = entry.child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(theme::TEXT_LABEL))
                            .font_weight(theme::weight_label())
                            .line_height(px(theme::LINE_TIGHT))
                            .text_color(theme::text())
                            .child(SharedString::from(name)),
                    )
                    .when(!about.is_empty(), |text| {
                        text.child(
                            div()
                                .text_size(px(theme::TEXT_META))
                                .line_height(px(theme::LINE_TIGHT))
                                .text_color(theme::text_dim())
                                .child(SharedString::from(about)),
                        )
                    }),
            );

            // The same word the pane header uses, and the same size: a lone
            // `×` was a target a few pixels wide beside a thumbnail.
            bar = bar.child(div().flex().flex_row().items_center().child(entry).child(
                controls::destructive(("mini-close", index), "close").on_click(cx.listener(
                    move |this: &mut Self, _event, window, cx| this.close_slot(index, window, cx),
                )),
            ));
        }

        Some(
            bar.child(div().flex_1())
                // Once for the bar, not once per stream: everything in it is.
                .child(
                    div()
                        .flex_none()
                        .text_size(px(theme::TEXT_META))
                        .text_color(theme::text_dim())
                        .child("muted while you browse"),
                )
                .child(self.pill(
                    "mini-watch",
                    "back to watching".into(),
                    cx,
                    |this, _w, cx| this.go_watch(cx),
                ))
                // Styled as what it is. It used to be a twin of the pill beside
                // it, and the two read as two ways of going somewhere.
                .child(controls::destructive("mini-stop", "stop all").on_click(
                    cx.listener(|this: &mut Self, _event, _window, cx| this.stop_all(cx)),
                )),
        )
    }
}
