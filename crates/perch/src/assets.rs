//! The icons the widget library asks for.
//!
//! `gpui-component` draws its chevrons, its eye and its clear button by asking
//! the *host* for `icons/<name>.svg` — the crate ships none of its own. With no
//! [`AssetSource`] installed, `Application::new()` answers `None` to every one
//! of those, and gpui renders nothing at all: the quality dropdown had no
//! chevron and was indistinguishable from the text field above it, and the
//! credential fields had an invisible-but-clickable eye at their right edge.
//! Silent, because a missing asset is not an error anywhere in that path.
//!
//! So they live here, hand-drawn rather than vendored: eleven paths in a 24×24
//! box is less to carry than an icon set, and gpui only keeps the alpha anyway
//! — [`gpui::SvgRenderer`] rasterises and throws the colour away, tinting the
//! mask with whatever `text_color` the widget asked for. That is also why these
//! are stroked in flat black: nothing downstream ever sees it.

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
const ICONS: [(&str, &[u8]); 11] = [
    icon!("chevron-down"),
    icon!("chevron-left"),
    icon!("chevron-right"),
    icon!("chevron-up"),
    icon!("circle-x"),
    icon!("close"),
    icon!("eye"),
    icon!("inbox"),
    icon!("minus"),
    icon!("plus"),
    icon!("search"),
];

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
