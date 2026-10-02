//! A pane's ad-break notice, as the root keeps it: begun when streamlink
//! says it is filtering an ad out of the pane's stream
//! (`StreamEvent::AdBreak`), counted down once a second, and taken down
//! when it is over. What it says and when it is over are `crate::ad_break`'s;
//! the header draws it (`watch::header`).

use std::time::Instant;

use gpui::Context;

use super::RootView;
use crate::ad_break::AdBreak;
use crate::watch::{placement, Placement};

impl RootView {
    /// Streamlink said pane `index`'s stream is in an ad break, of `secs`
    /// when it said a length. Only on a pane with a player: the pre-roll is
    /// said once the player has asked for the picture, so one said with no
    /// player is about a stream the pane has moved on from.
    ///
    /// One timer for the notice, made afresh with each word (the one before
    /// is dropped, which calls it off): it sleeps to the next change of the
    /// tag's words (`AdBreak::next_tick`), repaints, and takes the notice
    /// down once it is over — its length run out, or the player saying the
    /// picture has moved again since (`VideoView::resumed_at`). It ends with
    /// the notice; nothing ticks while there is none.
    ///
    /// A notice that begins on a pane whose header lives over its picture
    /// brings the header up for a moment (`reveal_header`, as a pane key
    /// does), since that header otherwise shows only while the pointer is
    /// on the pane, and a frozen picture nobody is pointing at is what the
    /// notice is for. Only when it begins: a length said after the pre-roll
    /// wait, or a second word about the same break, does not bring it up
    /// again.
    pub(super) fn ad_break_began(
        &mut self,
        index: usize,
        secs: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let slot = &mut self.slots[index];
        if slot.video().is_none() {
            return;
        }
        let now = Instant::now();
        let reveal = slot.ad_break.is_none() && placement(slot) == Placement::OverPicture;
        let ad = AdBreak::said(slot.ad_break, secs, now);
        eprintln!(
            "video: {} ad break{}",
            slot.key,
            secs.map(|secs| format!(" of {secs}s")).unwrap_or_default()
        );
        slot.ad_break = Some(ad);
        let key = slot.key.clone();
        let key_for_reveal = key.clone();
        slot.ad_tick = Some(cx.spawn(async move |this, cx| loop {
            let wait = {
                let Ok(Some(ad)) = this.update(cx, |this: &mut RootView, _| {
                    this.slot_index(&key)
                        .and_then(|index| this.slots[index].ad_break)
                }) else {
                    return;
                };
                ad.next_tick(Instant::now())
            };
            cx.background_executor().timer(wait).await;
            let going = this.update(cx, |this: &mut RootView, cx| {
                let Some(index) = this.slot_index(&key) else {
                    return false;
                };
                let slot = &mut this.slots[index];
                let Some(ad) = slot.ad_break else {
                    return false;
                };
                let resumed = slot.video().and_then(|view| view.read(cx).resumed_at());
                cx.notify();
                if ad.over(Instant::now(), resumed) {
                    // Not `end_ad_break`: that would drop this timer from
                    // inside itself. It ends by returning, below.
                    slot.ad_break = None;
                    return false;
                }
                true
            });
            if !matches!(going, Ok(true)) {
                return;
            }
        }));
        if reveal {
            self.reveal_header(&key_for_reveal, cx);
        }
        cx.notify();
    }
}
