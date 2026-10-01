//! Keyboard shortcuts.
//!
//! The controls you reach for while watching — pause, mute, volume, chat,
//! fullscreen — are icons on a bar that comes up over the video under the
//! pointer, each naming its key in its tooltip ([`Hint`]); a pane's × sits
//! in its header and names its key the same way, and the rest live in the
//! title bar or the settings sheet. That is the right place for them when
//! you are already holding the mouse, and no place at all when you are not,
//! which for a window left open for hours is most of the time.
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
//! binding here is scoped, and every one a text box could want stands aside
//! for a focused input or dropdown. The three chords that cannot be typing
//! do not; see `bindings`.
//!
//! There is deliberately no on-screen feedback for pause, mute or volume.
//! Each one announces itself through the thing it controls — a paused picture
//! stops moving, and the other two are audible — so a flash of UI would only be
//! telling you what you already know. The bar's glyphs follow the same state,
//! but the bar is up only while the pointer is on the picture, so a key
//! pressed with the mouse elsewhere still puts nothing on screen.
//!
//! A key that changes which pane is active is the exception: `1`–`4` and
//! `Tab`, with more than one pane, and `C` hiding a chat. Those bring the
//! pane's header up over its picture for a moment (`RootView::reveal_header`),
//! when chat is hidden and the header lives there. Nothing the pane does
//! answers "which pane are the keys talking to now?", and the pointer cannot
//! either: pointing at a pane is what makes it the active one.

use gpui::{actions, Action, App, KeyBinding};

use crate::watch::MAX_PANES;

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
        /// Fold the follows rail away, or bring it back.
        ToggleSidebar,
        /// Close the active pane.
        ClosePane,
        /// Leave the watch page, keeping the streams playing, with their
        /// sound, in the mini player — or stopping them, with it turned off.
        GoBrowse,
        /// Open the settings sheet, or close it if it is already open.
        ToggleSettings,
        /// Put the cursor in the title bar's search box.
        FocusSearch,
        /// Ask Twitch again for whichever list is on screen.
        Refresh,
        /// Open the command palette, or close it if it is already open.
        TogglePalette,
        /// Put the video and chat back to the sizes they are derived at.
        ResetLayout,
        /// Take the window fullscreen, or bring it back.
        ToggleFullscreen,
        /// Skip back in a past broadcast.
        SeekBack,
        /// Skip ahead in a past broadcast.
        SeekForward,
        /// Point the keyboard at the next pane along, or the one before.
        NextPane,
        PreviousPane,
        /// One step out of wherever the browse page has got to: a channel's
        /// page, a search or a category closes; with none open, back to
        /// whatever is playing. Out, not back: it goes up from where you
        /// are, whichever way you came, where [`NavigateBack`] retraces your
        /// steps. Named apart so the two meanings cannot share a handler.
        StepOut,
        /// Back along the trail of places the app has been: a tab, a
        /// category, a channel's page, a search, the watch page. See
        /// `crate::trail`.
        NavigateBack,
        /// Forward again along the trail, after going back.
        NavigateForward,
    ]
);

/// Point the keyboard at the pane numbered `index`, counting from the top
/// left. One action carrying the number rather than one per pane, so the
/// bindings are a loop and the handler a lookup. `no_json` because the keymap
/// here is code, not a file: nothing ever builds one of these from JSON.
#[derive(Clone, PartialEq, Eq, Action)]
#[action(namespace = perch, no_json)]
pub struct ActivatePane {
    pub index: usize,
}

/// The key for each pane, in grid order. Sized by `MAX_PANES` so a fifth
/// pane could not arrive without a key to reach it.
const PANE_KEYS: [&str; MAX_PANES] = ["1", "2", "3", "4"];

/// How much one press moves the volume.
///
/// Coarse on purpose: the slider is where a precise level is set, and a step
/// too small to hear turns one press into five.
pub const VOLUME_STEP: i16 = 5;

/// How far one press moves a past broadcast, in seconds. Ten is what every
/// player binds the arrows to; the bar is there for anything larger.
pub const SEEK_STEP: f64 = 10.0;

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
const SHEET: &str = "Sheet";

/// What `RootView` reports while watching, browsing, with the palette open,
/// and with the settings sheet open.
///
/// A modal replaces the page name rather than adding to it, so a shortcut
/// scoped to a page cannot fire through one without anybody having to
/// remember to write `!Modal`. The sheet is a modal that also says which it
/// is, for the one key that has to tell the two apart: `Ctrl+K` closes the
/// palette it opened, and does nothing over the sheet (see `bindings`).
pub const CONTEXT_WATCH: &str = "Perch Watch";
pub const CONTEXT_BROWSE: &str = "Perch Browse";
pub const CONTEXT_MODAL: &str = "Perch Modal";
pub const CONTEXT_SHEET: &str = "Perch Modal Sheet";

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
    let watch = format!("{APP} && {WATCH} && {TYPING}");
    let browse = format!("{APP} && {BROWSE} && {TYPING}");
    let modal = format!("{APP} && {MODAL} && {TYPING}");
    // The palette, the settings and refresh stand aside for nothing. A chord
    // on the command key types no character, and gpui-component binds none
    // of these three in any widget — so standing aside only meant that after
    // a search, which leaves the cursor in the box, `Ctrl+K` did nothing
    // until the page was clicked. Scoped all the same: `None` would outrank
    // the widgets' own bindings, these three included if one ever took them.
    let anywhere = APP;
    let browsing = format!("{APP} && {BROWSE}");
    // The palette's key, anywhere but over the settings sheet. The palette is
    // drawn under the sheet, so opening it there put a box nobody could see
    // behind the sheet with the keyboard in it: typing went into the hidden
    // box, and `Enter` could open a channel behind the sheet. So the key does
    // nothing until the sheet is closed: the two never stack, as the gear and
    // `Ctrl+,` already make sure from the other side by putting the sheet in
    // the palette's place.
    let unless_sheet = format!("{APP} && !{SHEET}");

    let mut bindings = vec![
        // Watching. Bare letters and arrows are safe here only because of the
        // guard. The title bar's search box is over the watch page too, and
        // `Ctrl+F` puts the cursor in it there, so without `TYPING` an `m`
        // typed into a search would mute the stream instead.
        KeyBinding::new("space", TogglePlayback, Some(&watch)),
        KeyBinding::new("m", ToggleMute, Some(&watch)),
        KeyBinding::new("c", ToggleChat, Some(&watch)),
        KeyBinding::new("up", VolumeUp, Some(&watch)),
        KeyBinding::new("down", VolumeDown, Some(&watch)),
        // Sideways is time, the way up and down are volume. On a live pane
        // there is nowhere to go and the press does nothing.
        KeyBinding::new("left", SeekBack, Some(&watch)),
        KeyBinding::new("right", SeekForward, Some(&watch)),
        KeyBinding::new("secondary-w", ClosePane, Some(&watch)),
        KeyBinding::new("escape", GoBrowse, Some(&watch)),
        // The rail is on both pages, so its key is too — twice rather than in
        // the `app` context, which a modal's own context also satisfies. A
        // bare letter is safe on either page for one reason: `TYPING` stands
        // the whole keymap aside while a text box has the cursor.
        KeyBinding::new("b", ToggleSidebar, Some(&watch)),
        KeyBinding::new("b", ToggleSidebar, Some(&browse)),
        // The search box is in the title bar, over both pages, so its key is
        // on both — twice, for the reason the rail's is. A search typed on
        // the watch page leaves it for the results, the way `Esc` would.
        KeyBinding::new("secondary-f", FocusSearch, Some(&watch)),
        KeyBinding::new("secondary-f", FocusSearch, Some(&browse)),
        // Anywhere. `secondary-,` is the settings gesture on both platforms —
        // literally so on macOS, where ⌘, opens preferences in everything —
        // and it also closes the sheet, so the same key opens and dismisses it.
        KeyBinding::new("secondary-,", ToggleSettings, Some(anywhere)),
        KeyBinding::new("secondary-r", Refresh, Some(&browsing)),
        KeyBinding::new("secondary-k", TogglePalette, Some(&unless_sheet)),
        // Only where the sizes exist. `secondary-0` is the reset gesture every
        // browser and editor already uses for the same kind of thing.
        KeyBinding::new("secondary-0", ResetLayout, Some(&watch)),
        // Fullscreen. `f` is what every video player binds while watching,
        // and it is a bare letter, so it is scoped to the watch page like the
        // other bare letters. `f11` is the browser gesture and is safe on
        // either page, so it is bound on both.
        KeyBinding::new("f", ToggleFullscreen, Some(&watch)),
        KeyBinding::new("f11", ToggleFullscreen, Some(&watch)),
        KeyBinding::new("f11", ToggleFullscreen, Some(&browse)),
        KeyBinding::new("escape", ToggleSettings, Some(&modal)),
        // Out, on the browse page: of a list that has taken the page over,
        // or to whatever is playing. The watch page's `escape` is the other
        // direction, and the two never meet: each is scoped to its page.
        KeyBinding::new("escape", StepOut, Some(&browse)),
        // Back and forward along the trail, on both pages — twice rather than
        // `anywhere`, which a modal's own context satisfies too, and nothing
        // should navigate behind the sheet. Guarded like the bare keys
        // although they are chords: in a text box `alt-left` moves the cursor
        // a word on macOS, where it is Option, and that is what somebody
        // typing a search meant by it.
        KeyBinding::new("alt-left", NavigateBack, Some(&watch)),
        KeyBinding::new("alt-left", NavigateBack, Some(&browse)),
        KeyBinding::new("alt-right", NavigateForward, Some(&watch)),
        KeyBinding::new("alt-right", NavigateForward, Some(&browse)),
        // The pane the keys talk to, without reaching for the mouse: the next
        // one along, or the one before.
        KeyBinding::new("tab", NextPane, Some(&watch)),
        KeyBinding::new("shift-tab", PreviousPane, Some(&watch)),
    ];
    // Or its number. Bare digits are safe here for the reason the letters
    // are: `TYPING` stands them aside while the title bar's search box has the
    // cursor.
    for (index, key) in PANE_KEYS.iter().enumerate() {
        bindings.push(KeyBinding::new(key, ActivatePane { index }, Some(&watch)));
    }
    bindings
}

/// Install the keymap. Must run after `gpui_component::init`, which registers
/// the bindings these stand aside for.
pub fn init(cx: &mut App) {
    cx.bind_keys(bindings());
}

/// How this platform writes the modifier `secondary-` binds to.
///
/// A macro rather than a `const` because [`SHORTCUTS`] is a const array of
/// `&'static str`, and `concat!` is the only way to build one of those at
/// compile time. The no-argument arm is the prefix on its own, so the test that
/// checks a label against its keystroke reads it from here rather than
/// repeating the string a third time.
///
/// `⌘` rather than `Cmd+` on macOS: the glyph is how every other Mac app writes
/// it, and a menu bar full of `⌘,` next to a settings sheet saying `Cmd+,` is
/// the app looking foreign for no reason.
#[cfg(target_os = "macos")]
macro_rules! secondary {
    () => {
        "\u{2318}"
    };
    ($key:literal) => {
        concat!(secondary!(), $key)
    };
}

#[cfg(not(target_os = "macos"))]
macro_rules! secondary {
    () => {
        "Ctrl+"
    };
    ($key:literal) => {
        concat!(secondary!(), $key)
    };
}

/// How this platform writes the Alt modifier: `⌥` on macOS, where the key is
/// Option and every app draws it as the glyph, and `Alt+` elsewhere. The
/// same shape as [`secondary!`], for the same reason — a const table can only
/// be built with `concat!` — and checked the same way, against what the
/// keystroke beside it parses to.
#[cfg(target_os = "macos")]
macro_rules! alt {
    () => {
        "\u{2325}"
    };
    ($key:literal) => {
        concat!(alt!(), $key)
    };
}

#[cfg(not(target_os = "macos"))]
macro_rules! alt {
    () => {
        "Alt+"
    };
    ($key:literal) => {
        concat!(alt!(), $key)
    };
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
/// thinks of as one, and `escape` should read as `Esc`. It is not free-form
/// though: anything on the secondary modifier is built with [`secondary!`], so
/// a label cannot claim `Ctrl` on a machine that binds `⌘`. That was the one
/// way this table could still lie after the check below — the keystrokes
/// normalise through `Keystroke::parse`, and the labels used to normalise
/// through nothing at all. The Alt modifier is held to the same rule through
/// [`alt!`].
///
/// Back and forward are two rows rather than one `Alt+← / →`: the key column
/// is sized for the longest label in it, and that one would not fit.
pub const SHORTCUTS: [(&[&str], &str, &str); 18] = [
    (&["space"], "Space", "Pause or resume"),
    (&["m"], "M", "Mute or unmute"),
    (&["c"], "C", "Show or hide this chat"),
    (&["b"], "B", "Show or hide the follows rail"),
    (&["f", "f11"], "F / F11", "Fullscreen"),
    (&["up", "down"], "↑ / ↓", "Volume"),
    (&["left", "right"], "← / →", "Skip 10 s in a past broadcast"),
    (&PANE_KEYS, "1 – 4", "Talk to that pane"),
    (&["tab", "shift-tab"], "Tab", "The next pane"),
    (&["secondary-w"], secondary!("W"), "Close this pane"),
    (&["escape"], "Esc", "Back to browsing, or to watching"),
    (
        &["alt-left"],
        alt!("←"),
        "Back (also the mouse's back button)",
    ),
    (&["alt-right"], alt!("→"), "Forward"),
    (&["secondary-f"], secondary!("F"), "Search"),
    (&["secondary-r"], secondary!("R"), "Refresh this list"),
    (&["secondary-,"], secondary!(","), "Settings"),
    (&["secondary-k"], secondary!("K"), "Command palette"),
    (&["secondary-0"], secondary!("0"), "Reset the pane sizes"),
];

/// Declares [`Hint`] once, as `Variant => "keystroke" as Action`, and derives
/// the rest from that one list: the enum, the keystroke each variant names,
/// and — for the tests only — every variant and the action its key has to
/// fire. The same shape as `assets::perch_icons!`, so a hint cannot exist
/// without `every_hint_names_a_listed_and_bound_key` checking it; `ALL` and
/// `action` are `cfg(test)` because only the tests read them, and this crate
/// has no library half to excuse dead code.
macro_rules! hints {
    ($($(#[$doc:meta])* $variant:ident => $keystroke:literal as $action:ty),* $(,)?) => {
        /// A key a control's tooltip names, as in `Settings (Ctrl+,)`.
        ///
        /// The tooltip does not spell the key itself. It names the keystroke
        /// here, in the grammar `KeyBinding::new` takes, and borrows the label
        /// [`SHORTCUTS`] already shows for it — so a tooltip cannot name a key
        /// nothing binds, a key bound to some other action, or write `Ctrl` on
        /// a machine that binds `⌘`, without
        /// `every_hint_names_a_listed_and_bound_key` failing. Variants arrive
        /// with the first control that names them: an unused one is dead code.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Hint {
            $($(#[$doc])* $variant),*
        }

        impl Hint {
            #[cfg(test)]
            const ALL: &'static [Hint] = &[$(Hint::$variant),*];

            fn keystroke(self) -> &'static str {
                match self {
                    $(Hint::$variant => $keystroke),*
                }
            }

            /// What the key has to do for the tooltip to be telling the
            /// truth. A bound key is not enough: `Settings (Ctrl+R)` names a
            /// real key, just not this control's.
            #[cfg(test)]
            fn action(self) -> std::any::TypeId {
                match self {
                    $(Hint::$variant => std::any::TypeId::of::<$action>()),*
                }
            }
        }
    };
}

hints! {
    Rail => "b" as ToggleSidebar,
    Back => "alt-left" as NavigateBack,
    Forward => "alt-right" as NavigateForward,
    Settings => "secondary-," as ToggleSettings,
    Playback => "space" as TogglePlayback,
    Mute => "m" as ToggleMute,
    // Up names the pair: the sheet lists `up` and `down` as one row, `↑ / ↓`,
    // and the tooltip borrows that row's label whole.
    Volume => "up" as VolumeUp,
    Chat => "c" as ToggleChat,
    // The same for `f` and `f11`, as `F / F11`.
    Fullscreen => "f" as ToggleFullscreen,
    // A pane header's ×, as `Ctrl+W` — `⌘W` on a Mac, from the sheet's label.
    Close => "secondary-w" as ClosePane,
}

impl Hint {
    /// How the settings sheet writes this key: the label of the [`SHORTCUTS`]
    /// row that lists its keystroke.
    fn label(self) -> Option<&'static str> {
        SHORTCUTS
            .iter()
            .find(|(keystrokes, _, _)| keystrokes.contains(&self.keystroke()))
            .map(|(_, label, _)| *label)
    }

    /// `text`, then the key in brackets — or `text` alone, should the key ever
    /// drop off the listing, rather than an empty pair of brackets.
    pub fn tooltip(self, text: &str) -> String {
        match self.label() {
            Some(label) => format!("{text} ({label})"),
            None => text.to_string(),
        }
    }
}

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
        assert_eq!(bindings().len(), 32);
        for context in [CONTEXT_WATCH, CONTEXT_BROWSE, CONTEXT_MODAL, CONTEXT_SHEET] {
            KeyContext::parse(context)
                .unwrap_or_else(|e| panic!("{context} is not a key context: {e}"));
        }
    }

    /// With the cursor in a text box — where a search leaves it — the three
    /// chords still answer, and a bare letter still goes to the box. And the
    /// search box is a keystroke away from the watch page too, since the box
    /// is in the title bar over both, so the watch page's letters, digits and
    /// arrows have to stand aside for it there as well. Asked of gpui's own
    /// keymap, so it is the resolution that runs, not a reading of the
    /// predicates.
    #[test]
    fn the_chords_work_while_typing_and_the_letters_do_not() {
        use std::any::TypeId;

        let keymap = gpui::Keymap::new(bindings());
        let fires_in = |context: &[KeyContext], keystroke: &str, action: TypeId| {
            let (matched, _) =
                keymap.bindings_for_input(&[Keystroke::parse(keystroke).unwrap()], context);
            matched
                .iter()
                .any(|binding| binding.action().as_any().type_id() == action)
        };
        let typing = [
            KeyContext::parse(CONTEXT_BROWSE).unwrap(),
            KeyContext::parse("Input").unwrap(),
        ];
        let fires = |keystroke: &str, action: TypeId| fires_in(&typing, keystroke, action);
        let watching = [KeyContext::parse(CONTEXT_WATCH).unwrap()];
        assert!(
            fires_in(&watching, "secondary-f", TypeId::of::<FocusSearch>()),
            "the title bar's search box is on the watch page too"
        );
        assert!(fires("secondary-k", TypeId::of::<TogglePalette>()));
        assert!(fires("secondary-,", TypeId::of::<ToggleSettings>()));
        assert!(fires("secondary-r", TypeId::of::<Refresh>()));
        assert!(
            !fires("b", TypeId::of::<ToggleSidebar>()),
            "a letter typed into the box must reach the box"
        );
        assert!(
            !fires("escape", TypeId::of::<StepOut>()),
            "the box has its own escape"
        );

        // The same box on the watch page, where most of the bare keys are.
        // Each would act on the page behind a search being typed.
        let typing_while_watching = [
            KeyContext::parse(CONTEXT_WATCH).unwrap(),
            KeyContext::parse("Input").unwrap(),
        ];
        let fires_watching =
            |keystroke: &str, action: TypeId| fires_in(&typing_while_watching, keystroke, action);
        for (keystroke, action) in [
            ("m", TypeId::of::<ToggleMute>()),
            ("space", TypeId::of::<TogglePlayback>()),
            ("c", TypeId::of::<ToggleChat>()),
            ("f", TypeId::of::<ToggleFullscreen>()),
            ("b", TypeId::of::<ToggleSidebar>()),
            ("left", TypeId::of::<SeekBack>()),
            ("up", TypeId::of::<VolumeUp>()),
            ("1", TypeId::of::<ActivatePane>()),
            ("tab", TypeId::of::<NextPane>()),
            ("escape", TypeId::of::<GoBrowse>()),
        ] {
            assert!(
                !fires_watching(keystroke, action),
                "{keystroke} typed into the search box on the watch page must reach the box"
            );
        }
        assert!(fires_watching("secondary-k", TypeId::of::<TogglePalette>()));
        assert!(fires_watching(
            "secondary-,",
            TypeId::of::<ToggleSettings>()
        ));

        // Back and forward: on both pages, and on neither through a modal or
        // into a text box, where `alt-left` is the box's own.
        let browsing = [KeyContext::parse(CONTEXT_BROWSE).unwrap()];
        let modal = [KeyContext::parse(CONTEXT_MODAL).unwrap()];
        let sheet = [KeyContext::parse(CONTEXT_SHEET).unwrap()];
        for (keystroke, action) in [
            ("alt-left", TypeId::of::<NavigateBack>()),
            ("alt-right", TypeId::of::<NavigateForward>()),
        ] {
            assert!(
                fires_in(&watching, keystroke, action),
                "{keystroke} watching"
            );
            assert!(
                fires_in(&browsing, keystroke, action),
                "{keystroke} browsing"
            );
            assert!(
                !fires(keystroke, action),
                "{keystroke} typed into the box must reach the box"
            );
            assert!(
                !fires_watching(keystroke, action),
                "{keystroke} typed into the box on the watch page must reach the box"
            );
            assert!(
                !fires_in(&modal, keystroke, action),
                "{keystroke} must not navigate behind a modal"
            );
            assert!(
                !fires_in(&sheet, keystroke, action),
                "{keystroke} must not navigate behind the settings sheet"
            );
        }
    }

    /// The palette is drawn under the settings sheet, so `Ctrl+K` does
    /// nothing while the sheet is up — whether the keyboard is on the sheet
    /// or in one of its fields — rather than opening a box nobody can see
    /// with the cursor in it. It still closes the palette it opened, and the
    /// sheet's own two keys still answer over the sheet.
    #[test]
    fn the_palette_key_waits_for_the_settings_sheet_to_close() {
        use std::any::TypeId;

        let keymap = gpui::Keymap::new(bindings());
        let fires_in = |context: &[&str], keystroke: &str, action: TypeId| {
            let context: Vec<KeyContext> = context
                .iter()
                .map(|context| KeyContext::parse(context).unwrap())
                .collect();
            let (matched, _) =
                keymap.bindings_for_input(&[Keystroke::parse(keystroke).unwrap()], &context);
            matched
                .iter()
                .any(|binding| binding.action().as_any().type_id() == action)
        };
        let palette = TypeId::of::<TogglePalette>();

        assert!(!fires_in(&[CONTEXT_SHEET], "secondary-k", palette));
        assert!(
            !fires_in(&[CONTEXT_SHEET, "Input"], "secondary-k", palette),
            "with the cursor in a field on the sheet"
        );
        assert!(
            fires_in(&[CONTEXT_MODAL, "Input"], "secondary-k", palette),
            "the palette's own key closes it"
        );
        for page in [CONTEXT_WATCH, CONTEXT_BROWSE] {
            assert!(fires_in(&[page], "secondary-k", palette), "{page}");
        }

        let settings = TypeId::of::<ToggleSettings>();
        assert!(fires_in(&[CONTEXT_SHEET], "secondary-,", settings));
        assert!(fires_in(&[CONTEXT_SHEET], "escape", settings));
    }

    /// The predicates are assembled from the identifiers; the contexts are
    /// written out. Nothing else connects the two, so a rename on one side
    /// would leave every shortcut on that page silently dead.
    #[test]
    fn the_contexts_are_made_of_the_identifiers_the_predicates_test_for() {
        assert_eq!(CONTEXT_WATCH, format!("{APP} {WATCH}"));
        assert_eq!(CONTEXT_BROWSE, format!("{APP} {BROWSE}"));
        assert_eq!(CONTEXT_MODAL, format!("{APP} {MODAL}"));
        assert_eq!(CONTEXT_SHEET, format!("{APP} {MODAL} {SHEET}"));
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

    /// The other half of the sheet's claim: that the key it *draws* is the key
    /// it binds.
    ///
    /// [`every_documented_key_is_actually_bound`] normalises both sides through
    /// `Keystroke::parse`, which keeps the keystroke column honest and says
    /// nothing at all about the label beside it. That was survivable while the
    /// labels were fixed strings on one platform. It stopped being survivable
    /// when the modifier started depending on the target: a row reading `Ctrl+W`
    /// on a machine that binds `⌘W` is wrong in the one place a reader has no
    /// way to check.
    #[test]
    fn the_listing_names_the_modifier_it_binds() {
        const PREFIX: &str = secondary!();
        const ALT: &str = alt!();

        for (keystrokes, display, _) in SHORTCUTS {
            for keystroke in keystrokes {
                let parsed = Keystroke::parse(keystroke)
                    .unwrap_or_else(|e| panic!("{display}: {keystroke} does not parse: {e}"));
                assert_eq!(
                    parsed.modifiers.secondary(),
                    display.starts_with(PREFIX),
                    "{display} and {keystroke} disagree about the {PREFIX} modifier"
                );
                assert_eq!(
                    parsed.modifiers.alt,
                    display.starts_with(ALT),
                    "{display} and {keystroke} disagree about the {ALT} modifier"
                );
            }
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

    /// A tooltip's key is held to the same two claims as the sheet's — that it
    /// is listed, so the label it borrows exists, and that it is bound — and
    /// to one more: that what it is bound to is the control's own action.
    #[test]
    fn every_hint_names_a_listed_and_bound_key() {
        let bindings = bindings();

        for &hint in Hint::ALL {
            let label = hint.label().unwrap_or_default();
            assert!(
                !label.is_empty(),
                "{hint:?} names {}, which the sheet does not list",
                hint.keystroke()
            );
            let parsed = Keystroke::parse(hint.keystroke())
                .unwrap_or_else(|e| panic!("{hint:?}: {} does not parse: {e}", hint.keystroke()))
                .to_string();
            assert!(
                bindings.iter().any(|binding| {
                    binding.action().as_any().type_id() == hint.action()
                        && binding
                            .keystrokes()
                            .iter()
                            .any(|keystroke| keystroke.inner().to_string() == parsed)
                }),
                "{hint:?} names {}, which nothing binds to its action",
                hint.keystroke()
            );
            assert_eq!(hint.tooltip("Do it"), format!("Do it ({label})"));
        }
    }

    /// The README's keyboard table is the third place a key is listed, after
    /// the sheet and the tooltips, and the one a reader meets before the app
    /// is even built. Each label the sheet shows has to start a row there,
    /// exactly as the sheet writes it, in backticks — `↑ / ↓`, not `↑` `↓` —
    /// so a shortcut cannot be added here and left out there. It is the row
    /// that is looked for, `` | `Esc` | ``, not the label alone: most labels
    /// are in the README's prose as well, and a row dropped from the table
    /// would still find its key in a sentence further down.
    ///
    /// Not on macOS: the README spells the modifiers once, as `Ctrl` and
    /// `Alt`, and says that a Mac writes them `⌘` and `⌥`, so the labels a Mac
    /// build shows are deliberately not on the page. The `RUNNING` pages are
    /// shorter lists on purpose, and are kept in step by hand.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_readme_lists_every_shortcut() {
        const README: &str = include_str!("../../../README.md");

        for (_, display, _) in SHORTCUTS {
            assert!(
                README.contains(&format!("| `{display}` |")),
                "the sheet lists `{display}`, and the README's keyboard table does not"
            );
        }
    }
}
