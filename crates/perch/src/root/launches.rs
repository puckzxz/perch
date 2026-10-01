//! What the command line named: this process's own at startup, and every
//! later launch's, which `instance` hands over while this one runs.
//!
//! A later launch is the window coming forward, and whatever it names
//! opening beside what is playing — the answer a fresh launch gives too, where
//! the first is alone only because nothing else is open yet.

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{Context, Task, Window};

use super::RootView;
use crate::instance;
use crate::launch::{self, Launch};
use crate::target::Target;
use crate::watch::MAX_PANES;

impl RootView {
    /// Open what a launch named, side by side: alone if nothing is playing,
    /// beside it otherwise.
    pub(super) fn open_targets(
        &mut self,
        targets: Vec<Target>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for target in targets.into_iter().take(MAX_PANES) {
            match target {
                Target::Channel(channel) => {
                    let solo = self.slots.is_empty();
                    self.open_channel(channel, solo, window, cx);
                }
                // Named by a link, so it has to be looked up first, and that
                // needs a session; it opens once there is one, by the same
                // rule.
                Target::Video { id, start_secs } => self.open_video_link(id, start_secs, cx),
            }
        }
    }

    /// Open, one by one, what later launches hand over, for as long as this
    /// view lives.
    pub(super) fn pump_launches(
        mut launches: UnboundedReceiver<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(args) = launches.next().await {
                let launch = Launch::read(args);
                let alive = this.update_in(cx, |this: &mut RootView, window, cx| {
                    this.on_launch(launch, window, cx)
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    /// A later launch: the window to the front, then what it named.
    fn on_launch(&mut self, launch: Launch, window: &mut Window, cx: &mut Context<Self>) {
        instance::bring_forward(window, cx);
        // Its `--volume` is for what it opens, as it would have been for the
        // session it would otherwise have started.
        if let Some(level) = launch.volume {
            self.volume_override = Some(level);
        }
        self.open_targets(launch.targets, window, cx);
        // Written to a console nobody has, at startup; here there is a window
        // to say it in.
        if launch.help {
            self.toast(launch::USAGE, cx);
        }
        for warning in launch.warnings {
            self.toast(warning, cx);
        }
        cx.notify();
    }
}
