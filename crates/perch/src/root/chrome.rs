//! The shell's own furniture: the pills, the toasts and the follows rail.
//! None of it is a page. The pills are drawn by the pages; the rail and the
//! toasts by the root, beside and over whichever page is up, so they stay put
//! when the page changes. What plays on while you browse is `mini_player`,
//! drawn over the page the same way.

use std::time::Duration;

use gpui::{div, prelude::*, px, Context, IntoElement, SharedString, Window};

use super::{Page, RootView, Toast, ToastAction};
use crate::browse::Tab;
use crate::{controls, motion, sidebar, theme};

/// How long a toast stays up: time to read a "went live" and reach for it,
/// or to take back a forget.
const TOAST_LIFETIME: Duration = Duration::from_secs(8);

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

    /// The rail, and everything it needs to know about what is already open,
    /// or nothing when it is not `shown`: folded away, or in fullscreen. See
    /// `RootView::rail_shown`.
    pub(super) fn follows_rail(
        &mut self,
        shown: bool,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        // Not drawn, the rail holds nothing still; see `hold_live`.
        if !shown {
            self.hold_live(super::follows::LiveList::Rail, false, cx);
            return None;
        }
        // Live panes only: a recording open beside the rail does not make
        // its channel's row "watching".
        let watching: Vec<String> = self
            .slots
            .iter()
            .filter(|slot| slot.is_live())
            .map(|slot| slot.channel.clone())
            .collect();
        Some(sidebar::rail(
            sidebar::Rail {
                follows: &self.follows,
                offline: &self.offline,
                pinned: &self.settings.pinned,
                recommended: &self.recommended.shown,
                avatars: &self.avatars,
                watching: &watching,
                can_add: self.can_add(),
                follows_loaded: self.follows_loaded,
                offline_open: self.rail_offline_open,
            },
            &self.cache,
            &self.rail_scroll,
            |this: &mut RootView, action, window, cx| this.on_browse_action(action, window, cx),
            // For this session only; see the field.
            |this: &mut RootView, _window, cx| {
                this.rail_offline_open = !this.rail_offline_open;
                cx.notify();
            },
            cx,
        ))
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
    /// title bar, not from a tab, and the tab left lit behind them read as
    /// though the results were part of it — the history's, as often as not.
    pub(super) fn tab_pill(&self, tab: Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let variant = if self.discovery.tab == tab && self.discovery.search.is_none() {
            controls::Variant::Selected
        } else {
            controls::Variant::Pill
        };
        controls::pill(tab.label(), tab.label(), variant)
            .on_click(cx.listener(move |this, _event, window, cx| this.show_tab(tab, window, cx)))
    }

    /// Transient notices, top-right of the page.
    ///
    /// Measured from the top of the content area rather than of the window:
    /// the stack lives in the root's content area, which starts under the
    /// title bar, so the bar's height is already accounted for and nothing
    /// here repeats it. On the browse page it is offset below the tab strip
    /// too. The strip is left-aligned, but in a narrow window its Refresh
    /// reaches the right-hand side, where a "went live" toast would land on
    /// top of it and take the click. The old header had the search box and
    /// the refresh pill in that corner, and that is what toasts did to them.
    /// The watch page has no strip, so there the offset is a gap and nothing
    /// more.
    pub(super) fn toast_stack(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let top = match self.page {
            Page::Browse => theme::TAB_STRIP_HEIGHT + theme::GAP_TIGHT,
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
                // toast and would blanket whatever is beside it.
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
                        controls::pill(("toast-add", id), "+ Add", controls::Variant::Pill)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.act_on_toast(id, false, window, cx)
                            })),
                    )
                })
                .when(undo, |card| {
                    card.child(
                        controls::pill(("toast-undo", id), "Undo", controls::Variant::Pill)
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
    ///
    /// Takes the keyboard back for the root first, as every control the root
    /// cannot hear does. A toast card blocks the pointer from the root, so
    /// the root's own mouse-down never takes focus from a text box that had
    /// it — and the title bar's search box, which a search leaves the cursor
    /// in, stays on screen over the watch page a toast opens. Left there,
    /// every key on that page stood aside for the box, and Space typed a
    /// space into it.
    fn act_on_toast(&mut self, id: u64, solo: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
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
}
