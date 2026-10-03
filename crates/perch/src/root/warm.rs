//! Streamlink started ahead for a live pane's quality menu, so a pick from
//! it starts beside the picture with the slow part already done.
//!
//! A switch's wait is almost all streamlink starting. Measured with the
//! streamlink CLI run as perch runs it (HANDOFF, the swap trap under
//! Video): the serving run takes about 1.7 s from its spawn to `Starting
//! server` — some 450 ms for the interpreter and its imports, 200 ms for the
//! plugin, and a second for Twitch's access token and playlist — and then
//! sits idle: it opens the stream, and fetches anything at all, only when a
//! player connects. Idle, it holds about 45 MB and uses no CPU and no
//! network. So the menu opening starts one for each of the renditions most
//! likely to be picked ([`to_warm`], at most [`MAX_WARM`]), each serving its
//! exact rendition from the list the menu shows (`StreamOptions::offered`),
//! and a pick of one that has said `Ready` hands that relay to the new
//! player at once: what is left of the wait is the player opening it and
//! taking over. The filter is untouched: each is the same serving run a
//! start beside the picture makes, ads skipped from the stream's opening on.
//!
//! What one is, is a [`WarmStart`] on the pane's slot, numbered like every
//! start (`streams::next_generation`) and pumped like every start
//! (`RootView::pump`), so that taking it is only a matter of making it the
//! pane's pending start: its later events then reach `pending_event` by the
//! rule they always go by (`video_view::route`). Until then they are nobody
//! else's: `route` calls them stale, and `apply_stream_event` hands them
//! here ([`warm_event`](RootView::warm_event)), which keeps a `Ready` for
//! the pick and lets one that failed go.
//!
//! They are let go [`WARM_GRACE`] after the menu closes, which a press on a
//! row does before the pick reaches the root, so the pick still finds its
//! own (and a menu opened again straight away finds the rest); with the
//! pane's player, whatever state it leaves for (`Slot::set_state`); by a
//! cold start, which is new settings or a new credential
//! (`start_stream`); and with the slot. One older than [`WARM_FOR`] is not
//! taken, in case what Twitch told it no longer holds, and so is let go at
//! that age even while the menu stays open ([`expire_warm`](RootView::expire_warm)).
//! Nothing is started ahead for the rendition a pick is already starting
//! beside the picture (`PendingStart::rendition`): it is about to be what
//! plays, so it could never be taken.

use std::time::{Duration, Instant};

use gpui::{Context, Window};
use streamlink::{StreamEvent, StreamSupervisor};

use super::streams::next_generation;
use super::RootView;
use crate::watch::{PendingStart, Restart, WarmStart};

/// How many renditions a menu opening starts ahead. A Twitch ladder
/// usually has five or six, one of them playing, so four leaves out only
/// the one farthest from it — most often the smallest.
const MAX_WARM: usize = 4;

/// How long the ones started ahead outlive the menu closing: the pick that
/// closed it has to reach the root after the close does, and a menu opened
/// again within it finds them still there.
const WARM_GRACE: Duration = Duration::from_secs(5);

/// How old a start ahead may be and still be taken. Its access token is
/// good for twenty minutes (the `expires` Twitch puts in it); what else the
/// playlist it was given leans on is not known, so this stays well inside
/// that. One older is let go and the pick starts afresh.
const WARM_FOR: Duration = Duration::from_secs(120);

impl RootView {
    /// The quality menu of the pane `owner` names opened, or closed
    /// (`VideoEvent::QualityMenu`).
    pub(super) fn quality_menu(
        &mut self,
        owner: &str,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.slot_index(owner) else {
            return;
        };
        if open {
            self.warm_up(index, window, cx);
        } else {
            self.cool_down(index, cx);
        }
    }

    /// Start streamlink ahead for the renditions pane `index`'s menu is
    /// most likely to be picked from, keeping those already started that
    /// are still good. Only on a live pane whose picture covers it: a pick
    /// anywhere else starts cold (`restart_how`), and a recording's start
    /// is a probe that nothing can take ahead of time.
    fn warm_up(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let slot = &mut self.slots[index];
        slot.cooling = None;
        // Calling off the grace may leave some from before with no timer;
        // every way out below arms the one that lets them go at their age.
        let Some(view) = slot.video() else {
            return self.expire_warm(index, cx);
        };
        let (playing, available, covers) = {
            let view = view.read(cx);
            (
                view.quality().to_string(),
                view.available().to_vec(),
                view.covers(),
            )
        };
        if !slot.is_live() || !covers {
            return self.expire_warm(index, cx);
        }
        let resolving = slot.pending.as_ref().and_then(|p| p.rendition.clone());
        let wanted = to_warm(&available, &playing, resolving.as_deref(), MAX_WARM);
        slot.warm
            .retain(|warm| fresh(warm.since.elapsed()) && wanted.contains(&warm.quality));
        let missing: Vec<String> = wanted
            .into_iter()
            .filter(|name| !slot.warm.iter().any(|warm| warm.quality == *name))
            .collect();
        if missing.is_empty() {
            return self.expire_warm(index, cx);
        }
        let (key, channel) = (slot.key.clone(), slot.channel.clone());
        eprintln!(
            "video: {key} starting {} ahead for the quality menu",
            missing.join(", ")
        );
        for quality in missing {
            let options = self.stream_options(Some(quality.clone()), Some(available.clone()));
            // The height only matters to a choice, and the rendition is
            // named exactly, so any will do.
            let (supervisor, events) = StreamSupervisor::start(channel.clone(), 0, options);
            let generation = next_generation();
            let pump = self.pump(key.clone(), &channel, generation, events, window, cx);
            self.slots[index].warm.push(WarmStart {
                quality,
                supervisor,
                pump,
                generation,
                since: Instant::now(),
                ready: None,
            });
        }
        self.expire_warm(index, cx);
    }

    /// Let each of pane `index`'s starts ahead go once it is [`WARM_FOR`]
    /// old, while the menu stays open: past that it can never be taken,
    /// and four of them idle are some 180 MB. The timer sits in `cooling`,
    /// which the menu closing replaces with its shorter grace and opening
    /// again calls off before it arms this anew; it wakes at the oldest
    /// one's deadline, lets go what has expired, and sleeps to the next,
    /// ending once none are left.
    fn expire_warm(&mut self, index: usize, cx: &mut Context<Self>) {
        let slot = &mut self.slots[index];
        if slot.warm.is_empty() {
            return;
        }
        let key = slot.key.clone();
        slot.cooling = Some(cx.spawn(async move |this, cx| loop {
            let next = this.update(cx, |this: &mut RootView, _| {
                let index = this.slot_index(&key)?;
                let warm = &mut this.slots[index].warm;
                warm.retain(|warm| fresh(warm.since.elapsed()));
                warm.iter()
                    .map(|warm| WARM_FOR.saturating_sub(warm.since.elapsed()))
                    .min()
            });
            let Ok(Some(wait)) = next else {
                break;
            };
            cx.background_executor().timer(wait).await;
        }));
    }

    /// Let what was started ahead for pane `index`'s menu go once it has
    /// been closed for [`WARM_GRACE`]. Nothing is stopped now: the press on
    /// a row closes the menu before its pick reaches the root.
    fn cool_down(&mut self, index: usize, cx: &mut Context<Self>) {
        let slot = &mut self.slots[index];
        if slot.warm.is_empty() {
            return;
        }
        let key = slot.key.clone();
        slot.cooling = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(WARM_GRACE).await;
            let _ = this.update(cx, |this: &mut RootView, _| {
                // Not `cooling` itself, which would drop this timer from
                // inside itself; it ends by returning.
                if let Some(index) = this.slot_index(&key) {
                    this.slots[index].warm.clear();
                }
            });
        }));
    }

    /// The streamlink started ahead for `quality` on pane `index`, taken off
    /// the slot for a start beside its picture, if there is one still good
    /// ([`fresh`]). One too old is let go here.
    pub(super) fn take_warm(&mut self, index: usize, quality: &str) -> Option<WarmStart> {
        let warm = &mut self.slots[index].warm;
        let at = warm.iter().position(|warm| warm.quality == quality)?;
        let taken = warm.remove(at);
        fresh(taken.since.elapsed()).then_some(taken)
    }

    /// Make `warm` the start beside pane `index`'s picture, for `reason`,
    /// as `start_stream` would have made a new one: through `set_pending`,
    /// so a pick says it is under way, and then, if streamlink has already
    /// said it is ready, on to the player at once (`pending_event`).
    pub(super) fn adopt_warm(
        &mut self,
        index: usize,
        warm: WarmStart,
        for_height: u32,
        reason: Restart,
        cx: &mut Context<Self>,
    ) {
        let WarmStart {
            quality,
            supervisor,
            pump,
            generation,
            since,
            ready,
        } = warm;
        eprintln!(
            "video: {} takes the {quality} started ahead {} ms ago ({})",
            self.slots[index].key,
            since.elapsed().as_millis(),
            if ready.is_some() {
                "ready"
            } else {
                "still starting"
            }
        );
        let pending = PendingStart {
            supervisor,
            pump,
            generation,
            for_height,
            reason,
            rendition: Some(quality),
        };
        self.set_pending(index, Some(pending), cx);
        if let Some(ready) = ready {
            self.pending_event(index, ready, cx);
        }
    }

    /// What streamlink said about the start `generation` of pane `index`,
    /// which is not the pane's stream nor the one beside it: one started
    /// ahead for its menu, if it still is, and otherwise nothing. Its
    /// `Ready` is kept for the pick that takes it; one that fails or finds
    /// the channel off is let go, so a pick starts afresh and says why.
    pub(super) fn warm_event(&mut self, index: usize, generation: u64, event: StreamEvent) {
        let slot = &mut self.slots[index];
        let Some(at) = slot.warm.iter().position(|w| w.generation == generation) else {
            return;
        };
        match event {
            StreamEvent::Ready { .. } => {
                let warm = &mut slot.warm[at];
                eprintln!(
                    "video: {} {} ready ahead after {} ms",
                    slot.key,
                    warm.quality,
                    warm.since.elapsed().as_millis()
                );
                warm.ready = Some(event);
            }
            StreamEvent::Offline | StreamEvent::Failed { .. } => {
                let warm = slot.warm.remove(at);
                eprintln!(
                    "video: {} couldn't start {} ahead: {event:?}",
                    slot.key, warm.quality
                );
            }
            // Nothing is opened before a player connects, so no ad is seen.
            StreamEvent::Resolving | StreamEvent::AdBreak { .. } => {}
        }
    }
}

/// Which of the renditions `available` offers (highest first) to start
/// ahead, with `playing` on screen: up to `cap`, nearest to `playing` in the
/// list first and the sharper of two as near first — a pick is most often a
/// step, and more often up than down — and never `playing` itself, nor
/// `resolving`, what a pick is already starting beside it. With `playing`
/// not in the list, the sharpest.
fn to_warm(
    available: &[String],
    playing: &str,
    resolving: Option<&str>,
    cap: usize,
) -> Vec<String> {
    let at = available.iter().position(|name| name == playing);
    let mut candidates: Vec<(usize, &String)> = available
        .iter()
        .enumerate()
        .filter(|(index, name)| Some(*index) != at && Some(name.as_str()) != resolving)
        .collect();
    candidates.sort_by_key(|(index, _)| (at.map_or(*index, |at| index.abs_diff(at)), *index));
    candidates
        .into_iter()
        .take(cap)
        .map(|(_, name)| name.clone())
        .collect()
}

/// Whether a start ahead `age` old may still be taken; see [`WARM_FOR`].
fn fresh(age: Duration) -> bool {
    age < WARM_FOR
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ladder(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// The renditions next to what plays come first, the sharper of two as
    /// near before the other, and what plays is never started again.
    #[test]
    fn the_nearest_renditions_are_started_ahead_sharper_first() {
        let offered = ladder(&["1080p60", "720p60", "480p", "360p", "160p"]);
        assert_eq!(
            to_warm(&offered, "480p", None, 4),
            ["720p60", "360p", "1080p60", "160p"]
        );
        assert_eq!(
            to_warm(&offered, "480p", None, 2),
            ["720p60", "360p"],
            "a step either way before two"
        );
        assert_eq!(
            to_warm(&offered, "1080p60", None, 4),
            ["720p60", "480p", "360p", "160p"],
            "from the top, only down"
        );
        assert_eq!(
            to_warm(&offered, "160p", None, 3),
            ["360p", "480p", "720p60"],
            "from the bottom, only up"
        );
    }

    /// A ladder shorter than the cap is started whole, less what plays; a
    /// rendition playing that the list does not name leaves the sharpest.
    #[test]
    fn a_short_ladder_or_an_unknown_playing_one() {
        let offered = ladder(&["720p60", "480p", "160p"]);
        assert_eq!(to_warm(&offered, "720p60", None, 4), ["480p", "160p"]);
        assert_eq!(
            to_warm(&offered, "source", None, 2),
            ["720p60", "480p"],
            "nothing plays from this list"
        );
        assert_eq!(to_warm(&[], "720p60", None, 4), Vec::<String>::new());
        assert_eq!(to_warm(&offered, "480p", None, 0), Vec::<String>::new());
    }

    /// What a pick is already starting beside the picture is not started
    /// again, and the next nearest takes its place.
    #[test]
    fn what_a_pick_is_starting_is_not_started_ahead() {
        let offered = ladder(&["1080p60", "720p60", "480p", "360p", "160p"]);
        assert_eq!(
            to_warm(&offered, "480p", Some("720p60"), 3),
            ["360p", "1080p60", "160p"]
        );
        assert_eq!(
            to_warm(&offered, "480p", Some("not offered"), 2),
            ["720p60", "360p"]
        );
    }

    /// A start ahead is taken while what Twitch told it is surely good, and
    /// not after.
    #[test]
    fn only_a_fresh_start_ahead_is_taken() {
        assert!(fresh(Duration::ZERO));
        assert!(fresh(WARM_FOR - Duration::from_millis(1)));
        assert!(!fresh(WARM_FOR));
        assert!(!fresh(Duration::from_secs(20 * 60)));
    }
}
