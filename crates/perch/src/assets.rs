//! The icons: the ones the widget library asks for, and the ones Perch draws.
//!
//! `gpui-component` draws its chevrons, its eye and its clear button by asking
//! the *host* for `icons/<name>.svg` — the crate ships none of its own. With no
//! [`AssetSource`] installed, `Application::new()` answers `None` to every one
//! of those, and gpui renders nothing at all: the quality dropdown had no
//! chevron and was indistinguishable from the text field above it, and the
//! credential fields had an invisible-but-clickable eye at their right edge.
//! Silent, because a missing asset is not an error anywhere in that path.
//!
//! So they live here, hand-drawn rather than vendored: a few strokes each in
//! a 24×24 box are less to carry than an icon set, and gpui only keeps the
//! alpha anyway — [`gpui::SvgRenderer`] rasterises and throws the colour away,
//! tinting the mask with whatever `text_color` the element asked for. That is
//! also why these are stroked in flat black: nothing downstream ever sees it.
//! A new one is drawn the same way — the 24 box, a stroke of 2, round caps
//! and joins, no fill — or it will not sit beside the rest.
//!
//! Perch's own controls draw from the same table — the title bar's rail
//! toggle, back, forward and settings gear, the window's caption buttons, the
//! mini player's, the rail's pin and the fold over its offline follows, and
//! the player's control bar — through [`Icon`], so a control names an icon by
//! a variant the compiler checks rather than by a path string nothing does. A
//! variant is named for what it means, not what it looks like, and a file can
//! serve a variant and the widget library both: the fold's chevrons are the
//! dropdown's.
//! An `svg` element paints with its *own* `text_color` and nothing inherited,
//! so a glyph that was not given one draws nothing; see `controls`.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Pairs an icon's asset path with its bytes from one name, so the two cannot
/// disagree — the failure this whole module exists to fix is a path that
/// resolves to nothing.
macro_rules! icon {
    ($name:literal) => {
        (
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")).as_slice(),
        )
    };
}
/// The path a widget asks for, and the bytes it gets.
///
/// A table rather than a directory walk, because the binary should carry its
/// icons rather than depend on what happens to be beside the executable.
/// Anything not on this list is answered `None`, which is exactly what the app
/// did for all of them until now — so an icon nobody drew degrades to the
/// blank it already was rather than to a crash.
const ICONS: [(&str, &[u8]); 28] = [
    icon!("arrow-left"),
    icon!("arrow-right"),
    icon!("chat"),
    icon!("chat-off"),
    icon!("chevron-down"),
    icon!("chevron-left"),
    icon!("chevron-right"),
    icon!("chevron-up"),
    icon!("circle-x"),
    icon!("close"),
    icon!("expand"),
    icon!("eye"),
    icon!("fullscreen"),
    icon!("fullscreen-exit"),
    icon!("inbox"),
    icon!("minus"),
    icon!("more"),
    icon!("panel-left"),
    icon!("pause"),
    icon!("pin"),
    icon!("play"),
    icon!("plus"),
    icon!("search"),
    icon!("settings"),
    icon!("volume"),
    icon!("volume-off"),
    icon!("window-maximize"),
    icon!("window-restore"),
];

/// Declares the icons Perch draws itself, once, as `Variant => "file-stem"`,
/// and derives the rest from that one list: the [`Icon`] enum, the path each
/// variant asks [`Icons`] for, and — for the tests only — every variant.
///
/// `ALL` is `cfg(test)` because this crate is a binary with no library half:
/// a constant only the tests read is dead code to `clippy --all-targets`.
macro_rules! perch_icons {
    ($($variant:ident => $stem:literal),* $(,)?) => {
        /// An icon one of Perch's own controls draws. Each names a file on
        /// the [`ICONS`] table; `every_icon_perch_draws_is_on_the_table`
        /// holds every variant to that.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon {
            $($variant),*
        }

        impl Icon {
            #[cfg(test)]
            const ALL: &'static [Icon] = &[$(Icon::$variant),*];

            /// The asset path, in the form `svg().path(..)` hands to
            /// [`Icons::load`].
            pub fn path(self) -> &'static str {
                match self {
                    $(Icon::$variant => concat!("icons/", $stem, ".svg")),*
                }
            }
        }
    };
}

// Only what something draws: an unused variant is dead code like any other.
perch_icons! {
    Rail => "panel-left",
    Back => "arrow-left",
    Forward => "arrow-right",
    Settings => "settings",
    Minimize => "minus",
    Maximize => "window-maximize",
    Restore => "window-restore",
    Close => "close",
    Volume => "volume",
    VolumeOff => "volume-off",
    Expand => "expand",
    Folded => "chevron-right",
    Unfolded => "chevron-down",
    Pin => "pin",
    Play => "play",
    Pause => "pause",
    Fullscreen => "fullscreen",
    FullscreenExit => "fullscreen-exit",
    Chat => "chat",
    ChatOff => "chat-off",
    More => "more",
}

/// What `Application::new().with_assets(..)` is handed.
pub struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons")
    }

    /// A file on disk that nothing includes is dead weight, and an entry
    /// pointing at a file that is gone does not compile — so this only has to
    /// catch the first. Derived from the directory rather than from a second
    /// list, or the guardrail would need the same maintenance as the thing it
    /// guards.
    #[test]
    fn every_icon_on_disk_is_reachable() {
        let listed: Vec<&str> = ICONS.iter().map(|(path, _)| *path).collect();

        for entry in std::fs::read_dir(icon_dir()).expect("the icon directory is missing") {
            let name = entry.expect("unreadable icon").file_name();
            let path = format!("icons/{}", name.to_string_lossy());
            assert!(
                listed.contains(&path.as_str()),
                "{path} is on disk but nothing asks for it"
            );
        }
    }

    /// Every one of these is asked for by name from inside `gpui-component`,
    /// so a typo here is a blank in the UI rather than a compile error.
    #[test]
    fn the_icons_the_widgets_actually_reach_are_present() {
        // Select's chevron, the masked input's eye, and the clear button on a
        // cleanable field: the three the settings sheet and the search box
        // were drawing as nothing.
        for path in [
            "icons/chevron-down.svg",
            "icons/eye.svg",
            "icons/circle-x.svg",
        ] {
            assert!(
                Icons.load(path).unwrap().is_some(),
                "{path} resolves to nothing"
            );
        }
    }

    /// Perch's own icons are asked for by a path the macro builds, so a stem
    /// with a typo compiles and draws nothing. Every variant has to resolve
    /// through the same loader the renderer uses.
    #[test]
    fn every_icon_perch_draws_is_on_the_table() {
        for icon in Icon::ALL {
            assert!(
                Icons.load(icon.path()).unwrap().is_some(),
                "{icon:?} asks for {}, which is not on the table",
                icon.path()
            );
        }
    }

    /// Every icon has to survive the renderer that will actually rasterise it.
    /// A malformed path or a missing viewBox is a silent blank, which is the
    /// same symptom as the bug this module fixes.
    #[test]
    fn every_icon_parses_as_svg() {
        for (path, bytes) in ICONS {
            let svg = std::str::from_utf8(bytes).unwrap_or_else(|_| panic!("{path} is not UTF-8"));
            assert!(svg.contains("viewBox"), "{path} has no viewBox to scale by");
            assert!(
                svg.contains("stroke-width"),
                "{path} has no stroke, so it would rasterise to nothing"
            );
        }
    }

    /// Nothing outside the table is answered, which is what keeps a missing
    /// icon a blank rather than a panic.
    #[test]
    fn an_unknown_path_is_simply_absent() {
        assert!(Icons.load("icons/not-a-real-icon.svg").unwrap().is_none());
        assert!(Icons.load("").unwrap().is_none());
    }
}
