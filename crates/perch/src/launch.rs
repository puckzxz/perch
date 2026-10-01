//! What a launch asks for, read from its arguments.
//!
//! One reader for two moments: this process starting, and a later launch
//! handing its arguments to this one because it is already running (see
//! `instance`). Read in two places, the same words would one day mean two
//! things.

use crate::target::{self, Target};

/// The one line `--help` adds to what the arguments can be.
pub const USAGE: &str = "usage: perch [channel or link...] [--volume 0-100]";

/// What one launch asked for.
#[derive(Debug, Default, PartialEq)]
pub struct Launch {
    /// What to open: logins, and twitch.tv links to channels or recordings.
    pub targets: Vec<Target>,
    /// `--volume`: the level panes open at, for the rest of the session.
    pub volume: Option<u8>,
    /// What could not be used, to be said in the window. A release build has
    /// no console, so anything only printed would never be seen.
    pub warnings: Vec<String>,
    /// `--help`, which a starting process answers by writing the usage down
    /// and leaving, and a running one by showing it.
    pub help: bool,
}

impl Launch {
    /// Read `args`, the command line after the program's name.
    pub fn read(args: impl IntoIterator<Item = String>) -> Self {
        let mut launch = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--volume" => match args.next().and_then(|v| v.parse::<u8>().ok()) {
                    Some(level) => launch.volume = Some(level.min(100)),
                    None => launch
                        .warnings
                        .push("--volume needs a number from 0 to 100; ignored".into()),
                },
                "--help" | "-h" => launch.help = true,
                // Anything else that looks like an option is a mistake worth
                // naming, rather than a channel called `--foo` that streamlink
                // fails on a few seconds later with a message about a URL.
                flag if flag.starts_with('-') => {
                    launch
                        .warnings
                        .push(format!("unknown option {flag}; ignored"));
                }
                // A login, or a twitch.tv link to a channel or a recording,
                // read the one way the app reads either — see `target`.
                other => match target::parse(other) {
                    Some(target) => launch.targets.push(target),
                    None => launch.warnings.push(format!(
                        "{other:?} is not a Twitch channel or recording; skipped"
                    )),
                },
            }
        }
        launch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(args: &[&str]) -> Launch {
        Launch::read(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn nothing_asks_for_nothing() {
        assert_eq!(read(&[]), Launch::default());
    }

    #[test]
    fn channels_and_links_are_targets_in_the_order_given() {
        let launch = read(&["forsen", "https://www.twitch.tv/videos/2884829425?t=1h2m3s"]);
        assert_eq!(
            launch.targets,
            [
                Target::Channel("forsen".into()),
                Target::Video {
                    id: "2884829425".into(),
                    start_secs: Some(3723),
                },
            ]
        );
        assert!(launch.warnings.is_empty());
    }

    /// Louder than full is full; a level that is not a number is said so,
    /// and the channel after it is still read as one.
    #[test]
    fn the_volume_is_a_level_up_to_a_hundred() {
        assert_eq!(read(&["--volume", "30"]).volume, Some(30));
        assert_eq!(read(&["--volume", "250"]).volume, Some(100));

        let loud = read(&["--volume", "loud", "xqc"]);
        assert_eq!(loud.volume, None);
        assert_eq!(loud.warnings.len(), 1);
        assert_eq!(loud.targets, [Target::Channel("xqc".into())]);
    }

    #[test]
    fn what_cannot_be_used_is_named_rather_than_dropped() {
        let launch = read(&["--loud", "twitch.tv/directory", "-h"]);
        assert!(launch.help);
        assert!(launch.targets.is_empty());
        assert_eq!(
            launch.warnings,
            [
                "unknown option --loud; ignored",
                "\"twitch.tv/directory\" is not a Twitch channel or recording; skipped",
            ]
        );
    }
}
