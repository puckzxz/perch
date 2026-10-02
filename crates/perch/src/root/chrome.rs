//! The shell's own furniture: the pills, the toasts and the follows rail.
//! None of it is a page. The pills are drawn by the pages; the rail and the
//! toasts by the root, beside and over whichever page is up, so they stay put
//! when the page changes. What plays on while you browse is `mini_player`,
//! drawn over the page the same way.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{canvas, div, prelude::*, px, Context, IntoElement, SharedString, Window};

use super::{Page, RootView, Toast, ToastAction};
use crate::browse::Tab;
use crate::{controls, layout, motion, rail_preview, sidebar, theme};

/// How long a toast stays up: time to read a "went live" and reach for it,
/// or to take back a forget. Time with the pointer on the stack does not
/// count; see [`RootView::hold_toasts`].
const TOAST_LIFETIME: Duration = Duration::from_secs(8);

/// The least a toast has left once the pointer leaves the stack. A toast
/// held in its last second would otherwise go the moment the pointer moved
/// off it, which reads as the pointer having knocked it down.
const TOAST_RESUME_FLOOR: Duration = Duration::from_secs(2);

/// A toast's clock: how long it has left, and the moment that started
/// running down, or nothing while it is held.
///
/// Measured rather than ticked: nothing counts while the clock runs, and the
/// one timer the toast holds sleeps for exactly what is left. Stopping it
/// works out what is left from when it started.
pub(super) struct Countdown {
    left: Duration,
    since: Option<Instant>,
}

impl Countdown {
    /// A new toast's: its whole lifetime, not yet running.
    pub(super) fn new() -> Self {
        Self {
            left: TOAST_LIFETIME,
            since: None,
        }
    }

    /// Stop the clock at `now`, keeping what is left. A clock already
    /// stopped keeps what it had.
    fn pause(&mut self, now: Instant) {
        if let Some(since) = self.since.take() {
            self.left = time_left(self.left, now.saturating_duration_since(since));
        }
    }

    /// Start the clock at `now`, and say how long the toast's one timer
    /// should sleep: what it had left, but never less than
    /// [`TOAST_RESUME_FLOOR`]. Nothing for a clock already running, whose
    /// timer is still the one that counts; a second would take the toast
    /// down early.
    fn run(&mut self, now: Instant) -> Option<Duration> {
        if self.since.is_some() {
            return None;
        }
        self.left = resumed(self.left);
        self.since = Some(now);
        Some(self.left)
    }
}

/// What a toast with `left` to go has once `ran` of it has passed: none at
/// all, rather than an underflow, for a toast held after its timer ran out
/// and while it faded.
fn time_left(left: Duration, ran: Duration) -> Duration {
    left.saturating_sub(ran)
}

/// What a toast gets back when the pointer lets it go with `left` to go.
fn resumed(left: Duration) -> Duration {
    left.max(TOAST_RESUME_FLOOR)
}

/// The stack's clocks as the pointer comes onto it at `now`: every one
/// stops, keeping what it had left.
fn hold<'a>(clocks: impl IntoIterator<Item = &'a mut Countdown>, now: Instant) {
    for clock in clocks {
        clock.pause(now);
    }
}

/// The stack's clocks as the pointer leaves it at `now`: every stopped one
/// starts again, and what comes back is the timers to start, by the
/// clock's place in the stack and how long each sleeps. A clock that was
/// never started, a toast that arrived while the stack was held, gets its
/// whole lifetime; one already running gets no second timer.
fn release<'a>(
    clocks: impl IntoIterator<Item = &'a mut Countdown>,
    now: Instant,
) -> Vec<(usize, Duration)> {
    clocks
        .into_iter()
        .enumerate()
        .filter_map(|(index, clock)| clock.run(now).map(|wait| (index, wait)))
        .collect()
}

impl RootView {
    pub(super) fn toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_with(text, None, cx);
    }

    /// A notice that can be acted on: the text opens what it names.
    ///
    /// One that arrives with the pointer on the stack waits with the rest,
    /// its whole lifetime still to come, so the toasts go on as one.
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
            countdown: Countdown::new(),
            timer: None,
        });
        if !self.toasts_held {
            let index = self.toasts.len() - 1;
            if let Some(wait) = self.toasts[index].countdown.run(Instant::now()) {
                self.arm_toast(index, wait, cx);
            }
        }
        cx.notify();
    }

    /// Give the toast at `index`, whose countdown has just started, the
    /// one-shot timer that takes it down after `wait`.
    fn arm_toast(&mut self, index: usize, wait: Duration, cx: &mut Context<Self>) {
        let toast = &mut self.toasts[index];
        let id = toast.id;
        // Two stages, because dropping the element is what stops it being
        // drawn: start the fade when the countdown runs out, and only remove
        // it once the fade has had time to run. The task is the toast's, so
        // holding the toast or taking it down cancels it at either stage.
        toast.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
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
        }));
    }

    /// Note whether the pointer is on a toast, from the stack's probes (see
    /// [`toast_stack`](Self::toast_stack)), once a frame.
    ///
    /// While it is, no toast counts down: each stops its clock and drops its
    /// timer. All of them rather than the one pointed at, because the stack
    /// hangs from the top, and one going above the toast under the pointer
    /// would pull that one out from under it as it was reached for. One
    /// already fading out comes back, since it was reached for in its last
    /// moment. When the pointer leaves, each gets back what it had left, but
    /// at least [`TOAST_RESUME_FLOOR`], on a one-shot timer of its own.
    ///
    /// Nothing changes, and nothing is drawn again, on a frame that says
    /// what the last one did.
    pub(super) fn hold_toasts(&mut self, pointed: bool, cx: &mut Context<Self>) {
        if pointed == self.toasts_held {
            return;
        }
        self.toasts_held = pointed;
        let now = Instant::now();
        let clocks = self.toasts.iter_mut().map(|toast| &mut toast.countdown);
        if pointed {
            hold(clocks, now);
            for toast in &mut self.toasts {
                toast.timer = None;
                if toast.fade.set(true) {
                    cx.notify();
                }
            }
        } else {
            for (index, wait) in release(clocks, now) {
                self.arm_toast(index, wait, cx);
            }
        }
    }

    /// The rail, and everything it needs to know about what is already open,
    /// or nothing when it is not `shown`: folded away, or in fullscreen. See
    /// `RootView::rail_shown`.
    ///
    /// Its rows' preview cards (`crate::rail_preview`) only while the pointer
    /// is in the window, no modal is over the rail (the modal rule,
    /// [`modal_open`](Self::modal_open)), and the window has room for a
    /// card beside the rail and under the title bar
    /// ([`sidebar::previews_fit`]): the rows' probes see through anything
    /// drawn over them, and a card is drawn over everything. Otherwise the
    /// row the pointer was on is forgotten, since no probe is there to say
    /// it was left. So is a row that is no longer drawn at all — a stream
    /// that ended at a poll, a channel moved to another group — whose probe
    /// went with it: held on to, it would count as a card still up, and
    /// let the next row's card skip its wait.
    pub(super) fn follows_rail(
        &mut self,
        shown: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let viewport = window.viewport_size();
        let previews = shown
            && window.is_window_hovered()
            && !self.modal_open()
            && sidebar::previews_fit(
                f32::from(viewport.width),
                f32::from(viewport.height),
                layout::title_bar_height(window.is_fullscreen()),
            );
        let gone = self.rail_preview.pointed().is_some_and(|key| {
            !sidebar::groups(
                &self.follows,
                &self.offline,
                &self.recommended.shown,
                &self.settings.pinned,
                self.follows_loaded,
            )
            .draws_live_row(key)
        });
        if !previews || gone {
            self.rail_preview.clear();
        }
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
                last_live: &self.last_live,
                watching: &watching,
                can_add: self.can_add(),
                follows_loaded: self.follows_loaded,
                offline_open: self.rail_offline_open,
                previews,
                preview_shown: self.rail_preview.shown(Instant::now()),
                preview_pointed: self.rail_preview.pointed(),
            },
            &self.cache,
            &self.rail_scroll,
            // A press on the rail puts away the card of the row it was on:
            // the row has done what it was pressed for.
            |this: &mut RootView, action, window, cx| {
                if this.rail_preview.dismiss() {
                    cx.notify();
                }
                this.on_browse_action(action, window, cx)
            },
            |this: &mut RootView, key, pointed, cx| this.point_rail_row(&key, pointed, cx),
            // For this session only; see the field.
            |this: &mut RootView, _window, cx| {
                this.rail_offline_open = !this.rail_offline_open;
                cx.notify();
            },
            cx,
        ))
    }

    /// A live rail row's probe says whether the pointer is on it: the card
    /// comes up, goes, or starts its wait, and a timer wakes the rail when
    /// the wait is over, if the pointer is still on that row. See
    /// `rail_preview::Preview::point`.
    ///
    /// No `cx.notify()` for what changes now: this is heard in prepaint,
    /// where a notify asks for no frame, and the probe that reported has
    /// asked for the next one itself (`sidebar::preview_probe`). The
    /// timer's notify comes from outside drawing, where it does.
    fn point_rail_row(&mut self, key: &str, pointed: bool, cx: &mut Context<Self>) {
        match self.rail_preview.point(key, pointed, Instant::now()) {
            rail_preview::Change::None | rail_preview::Change::Now => {}
            rail_preview::Change::After(wait) => {
                let key = key.to_string();
                cx.spawn(async move |this, cx| {
                    let mut wait = wait;
                    loop {
                        cx.background_executor().timer(wait).await;
                        // Asked again rather than trusted to the timer: a
                        // wake a moment early waits out the rest instead of
                        // leaving the card down for good.
                        let left = this.update(cx, |this: &mut RootView, cx| {
                            let left = this
                                .rail_preview
                                .wait_left(&key, Instant::now())
                                .filter(|left| !left.is_zero());
                            if left.is_none() {
                                cx.notify();
                            }
                            left
                        });
                        match left {
                            Ok(Some(left)) => wait = left,
                            _ => break,
                        }
                    }
                })
                .detach();
            }
        }
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
        // Whether the pointer is on a card, measured by each card's probe
        // and reported once, by the probe after them all: gpui prepaints
        // children in order, so by then every card has had its say. Once
        // rather than per card, so a pointer moving from one card to the
        // next never reads as having left between the two reports.
        let pointed = Rc::new(Cell::new(false));
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
            // Measured, not taken from `on_hover`: a pointer that leaves the
            // window sends no move, so the listener's word would stand. The
            // listener is only there to wake a repaint as the pointer comes
            // and goes, so the probe runs again; nothing else draws a frame
            // then.
            let probe = {
                let pointed = pointed.clone();
                canvas(
                    move |bounds, window, _cx| {
                        if window.is_window_hovered() && bounds.contains(&window.mouse_position()) {
                            pointed.set(true);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            };
            let card = div()
                .id(("toast-card", id))
                .relative()
                .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
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
                })
                .child(probe);
            stack = stack.child(toast.fade.apply(("toast", id), theme::MOTION_ENTER, card));
        }
        // Absolute, so it takes no place in the column and adds no gap; and
        // drawn with no toasts too, so a stack emptied under the pointer lets
        // go of its hold. Not under a modal: the palette and the settings
        // sheet are drawn over the stack, scrim and all, so a pointer there
        // is on the modal, and a toast nobody can see or reach should go on
        // counting down.
        let covered = self.modal_open();
        let owner = cx.entity().downgrade();
        let report = canvas(
            move |_, _, cx| {
                owner
                    .update(cx, |this: &mut RootView, cx| {
                        this.hold_toasts(!covered && pointed.get(), cx)
                    })
                    .ok();
            },
            |_, _, _, _| {},
        )
        .absolute();
        stack.child(report)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(secs: f32) -> Duration {
        Duration::from_secs_f32(secs)
    }

    #[test]
    fn a_pause_keeps_what_was_left() {
        assert_eq!(time_left(secs(8.0), secs(3.0)), secs(5.0));
        // Held after its timer ran out, while it faded: nothing left, not
        // an underflow.
        assert_eq!(time_left(secs(8.0), secs(8.2)), Duration::ZERO);
    }

    #[test]
    fn letting_go_gives_back_at_least_the_floor() {
        assert_eq!(resumed(secs(5.0)), secs(5.0));
        assert_eq!(resumed(secs(0.5)), TOAST_RESUME_FLOOR);
        assert_eq!(resumed(Duration::ZERO), TOAST_RESUME_FLOOR);
    }

    #[test]
    fn a_new_toast_runs_its_whole_lifetime() {
        let mut countdown = Countdown::new();
        assert_eq!(countdown.run(Instant::now()), Some(TOAST_LIFETIME));
    }

    #[test]
    fn a_held_toast_loses_no_time_however_long_it_is_held() {
        let start = Instant::now();
        let mut countdown = Countdown::new();
        countdown.run(start);
        countdown.pause(start + secs(3.0));
        // A second pause, as a stack held twice over would ask for, takes
        // nothing more.
        countdown.pause(start + secs(60.0));
        assert_eq!(countdown.run(start + secs(90.0)), Some(secs(5.0)));
    }

    #[test]
    fn a_toast_held_in_its_last_moment_gets_the_floor() {
        let start = Instant::now();
        let mut countdown = Countdown::new();
        countdown.run(start);
        countdown.pause(start + secs(7.5));
        assert_eq!(countdown.run(start + secs(20.0)), Some(TOAST_RESUME_FLOOR));
        // Held again with a second of the floor left, it gets the whole
        // floor back: every release gives at least `TOAST_RESUME_FLOOR`.
        countdown.pause(start + secs(21.0));
        assert_eq!(countdown.run(start + secs(30.0)), Some(TOAST_RESUME_FLOOR));
    }

    #[test]
    fn only_a_stopped_clock_is_given_a_timer() {
        let start = Instant::now();
        let mut countdown = Countdown::new();
        assert!(countdown.run(start).is_some());
        // Already running: its timer is the one that counts, and a second
        // would take it down early.
        assert_eq!(countdown.run(start + secs(1.0)), None);
        countdown.pause(start + secs(2.0));
        assert_eq!(countdown.run(start + secs(4.0)), Some(secs(6.0)));
    }

    #[test]
    fn letting_go_of_the_stack_gives_every_toast_one_timer() {
        let start = Instant::now();
        let mut clocks = vec![Countdown::new(), Countdown::new()];
        assert_eq!(release(&mut clocks, start).len(), 2);
        hold(&mut clocks, start + secs(3.0));
        // Arrived while the stack was held: never started.
        clocks.push(Countdown::new());
        assert_eq!(
            release(&mut clocks, start + secs(40.0)),
            vec![(0, secs(5.0)), (1, secs(5.0)), (2, TOAST_LIFETIME)]
        );
        // Running now, so a second release starts nothing more.
        assert!(release(&mut clocks, start + secs(41.0)).is_empty());
    }
}
