//! Keyboard shortcuts.
//!
//! Every control in this app that you reach for while watching — pause, mute,
//! volume, close, back — is a hover-revealed overlay on the video. That is the
//! right place for them when you are already holding the mouse, and no place at
//! all when you are not, which for a window left open for hours is most of the
//! time.
//!
//! GPUI's keyboard stack has two gates, and both are easy to get subtly wrong:
//!
//! **A key only reaches a handler if something is focused.** The dispatch path
//! is derived entirely from `window.focus`; with nothing focused it is the bare
//! root node, whose context stack is empty — and an empty stack fails *every*
//! predicate. Nothing in this app focused anything before this module existed,
//! so `RootView` now takes focus on open and takes it back whenever it is lost.
//! Without that, every binding below is silently dead.
//!
//! **A context-free binding outranks a context-scoped one.** `None` is scored
//! at maximum depth and wins ties by later registration, so a bare `escape` or
//! `m` registered after `gpui_component::init` beats the text input's own
//! bindings and eats what you are typing — on Windows the character is simply
//! lost, because no `WM_CHAR` is ever generated. Hence [`TYPING`]: every
//! binding here is scoped, and every one of them stands aside for a focused
//! input or dropdown.
//!
//! There is deliberately no on-screen feedback for pause, mute or volume.
//! Each one announces itself through the thing it controls — a paused picture
//! stops moving, and the other two are audible — so a flash of UI would only be
//! telling you what you already know.

use gpui::{actions, App, KeyBinding};

actions!(
    perch,
    [
        /// Pause or resume the active pane.
        TogglePlayback,
        /// Mute or unmute the active pane, restoring the previous level.
        ToggleMute,
        VolumeUp,
        VolumeDown,
        /// Show or hide the active pane's chat.
        ToggleChat,
        /// Close the active pane.
        ClosePane,
        /// Leave the watch page, keeping the streams as muted thumbnails.
        GoBrowse,
        /// Open the settings sheet, or close it if it is already open.
        ToggleSettings,
        /// Put the cursor in the search box.
        FocusSearch,
        /// Ask Twitch again for whichever list is on screen.
        Refresh,
    ]
);

/// How much one press moves the volume.
///
/// Coarse on purpose: the slider is where a precise level is set, and a step
/// too small to hear turns one press into five.
pub const VOLUME_STEP: i16 = 5;

// The identifiers a context is built from, and that a predicate tests for.
//
// These are two different grammars and they do not mix: a *context* is
// whitespace-separated identifiers (`Perch Watch`), while a *predicate*
// is a boolean expression over them (`Perch && Watch`). Interpolating a
// context into a predicate parses as far as the first space and then fails —
// which `KeyBinding::new` reports by panicking at startup, and which the test
// below exists to catch first.
const APP: &str = "Perch";
const WATCH: &str = "Watch";
const BROWSE: &str = "Browse";
const MODAL: &str = "Modal";

/// What `RootView` reports while watching, browsing, and with the settings
/// sheet open.
///
/// The sheet replaces the page name rather than adding to it, so a shortcut
/// scoped to a page cannot fire through a modal without anybody having to
/// remember to write `!Modal`.
pub const CONTEXT_WATCH: &str = "Perch Watch";
pub const CONTEXT_BROWSE: &str = "Perch Browse";
pub const CONTEXT_MODAL: &str = "Perch Modal";

/// Everything in gpui-component that claims keys for itself while it is
/// focused. A binding guarded by this cannot swallow a keystroke meant for the
/// search box, a settings field or a dropdown.
///
/// `!X` scans the whole dispatch path rather than just the current depth, which
/// is what makes it work: the app's context is an *ancestor* of the focused
/// input, so it stays in the stack the entire time you are typing.
const TYPING: &str = "!Input && !Select && !PopupMenu";

/// The keymap, built but not installed.
///
/// Separate from [`init`] so a test can construct every binding without an
/// `App`: `KeyBinding::new` panics on an unparseable keystroke or predicate,
/// and a panic at startup is a nicer failure than a shortcut that quietly does
/// nothing, but a test is nicer still.
fn bindings() -> Vec<KeyBinding> {
    let app = format!("{APP} && {TYPING}");
    let watch = format!("{APP} && {WATCH} && {TYPING}");
    let browse = format!("{APP} && {BROWSE} && {TYPING}");
    let modal = format!("{APP} && {MODAL} && {TYPING}");

    vec![
        // Watching. Bare letters and arrows are safe here only because of the
        // guard: nothing on the watch page takes typed input.
        KeyBinding::new("space", TogglePlayback, Some(&watch)),
        KeyBinding::new("m", ToggleMute, Some(&watch)),
        KeyBinding::new("c", ToggleChat, Some(&watch)),
        KeyBinding::new("up", VolumeUp, Some(&watch)),
        KeyBinding::new("down", VolumeDown, Some(&watch)),
        KeyBinding::new("ctrl-w", ClosePane, Some(&watch)),
        KeyBinding::new("escape", GoBrowse, Some(&watch)),
        // Browsing.
        KeyBinding::new("ctrl-f", FocusSearch, Some(&browse)),
        // Anywhere. `ctrl-,` is the platform-neutral settings gesture and also
        // closes the sheet, so the same key both opens and dismisses it.
        KeyBinding::new("ctrl-,", ToggleSettings, Some(&app)),
        KeyBinding::new("ctrl-r", Refresh, Some(&browse)),
        KeyBinding::new("escape", ToggleSettings, Some(&modal)),
    ]
}

/// Install the keymap. Must run after `gpui_component::init`, which registers
/// the bindings these stand aside for.
pub fn init(cx: &mut App) {
    cx.bind_keys(bindings());
}

/// What the settings sheet lists, so a shortcut nobody can discover is not the
/// same as one that does not exist.
///
/// Each row is `(what is bound, what the sheet shows, what it does)`. The first
/// field is the point: it is written in the same grammar `KeyBinding::new`
/// takes, so [`every_documented_key_is_actually_bound`] can check the listing
/// against the real keymap. Being *next to* the bindings was never a guarantee
/// — a row could describe a key nobody had bound and nothing would notice.
/// The display column stays separate because `↑ / ↓` is two bindings a reader
/// thinks of as one, and `escape` should read as `Esc`.
pub const SHORTCUTS: [(&[&str], &str, &str); 9] = [
    (&["space"], "Space", "Pause or resume"),
    (&["m"], "M", "Mute or unmute"),
    (&["c"], "C", "Show or hide this chat"),
    (&["up", "down"], "↑ / ↓", "Volume"),
    (&["ctrl-w"], "Ctrl+W", "Close this pane"),
    (&["escape"], "Esc", "Back to follows"),
    (&["ctrl-f"], "Ctrl+F", "Search"),
    (&["ctrl-r"], "Ctrl+R", "Refresh this list"),
    (&["ctrl-,"], "Ctrl+,", "Settings"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyContext, Keystroke};

    /// Both halves of a shortcut fail quietly in their own way: an unparseable
    /// keystroke or predicate panics `KeyBinding::new` at startup, and a bad
    /// context string is swallowed by `key_context`'s `log_err`. Either way the
    /// symptom is "the key does nothing", which is a poor thing to debug.
    #[test]
    fn every_binding_and_every_context_parses() {
        assert_eq!(bindings().len(), 11);
        for context in [CONTEXT_WATCH, CONTEXT_BROWSE, CONTEXT_MODAL] {
            KeyContext::parse(context)
                .unwrap_or_else(|e| panic!("{context} is not a key context: {e}"));
        }
    }

    /// The predicates are assembled from the identifiers; the contexts are
    /// written out. Nothing else connects the two, so a rename on one side
    /// would leave every shortcut on that page silently dead.
    #[test]
    fn the_contexts_are_made_of_the_identifiers_the_predicates_test_for() {
        assert_eq!(CONTEXT_WATCH, format!("{APP} {WATCH}"));
        assert_eq!(CONTEXT_BROWSE, format!("{APP} {BROWSE}"));
        assert_eq!(CONTEXT_MODAL, format!("{APP} {MODAL}"));
    }

    /// The listing is for humans, so it is not derived from the bindings — but
    /// a line that describes no key at all is a documentation bug.
    #[test]
    fn the_listing_says_something_for_every_line() {
        for (_, display, description) in SHORTCUTS {
            assert!(!description.is_empty(), "{display} has no description");
            assert!(!display.is_empty(), "{description} shows no key");
        }
    }

    /// The claim the settings sheet makes about itself, enforced.
    ///
    /// A key that is documented but not bound is worse than one that is
    /// neither: the reader presses it, nothing happens, and the app looks
    /// broken rather than incomplete. Proximity in the file was never going to
    /// catch that; comparing against the keymap does.
    ///
    /// Both sides are normalised through `Keystroke::parse` rather than string
    /// equality, so `ctrl-w` and any other spelling of it agree.
    #[test]
    fn every_documented_key_is_actually_bound() {
        let bound: Vec<String> = bindings()
            .iter()
            .flat_map(|binding| binding.keystrokes())
            .map(|keystroke| keystroke.inner().to_string())
            .collect();

        for (keystrokes, display, _) in SHORTCUTS {
            for keystroke in keystrokes {
                let parsed = Keystroke::parse(keystroke)
                    .unwrap_or_else(|e| panic!("{display}: {keystroke} does not parse: {e}"))
                    .to_string();
                assert!(
                    bound.contains(&parsed),
                    "the sheet lists {display} ({keystroke}), which nothing binds"
                );
            }
        }
    }
}
