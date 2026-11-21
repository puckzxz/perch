use gpui::{actions, App, KeyBinding, Menu, MenuItem, OsAction};
use gpui_component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};

use crate::keys::{self, ToggleSettings};

actions!(perch, [HidePerch, HideOthers, ShowAll, Quit]);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-h", HidePerch, Some(keys::APP)),
        KeyBinding::new("cmd-alt-h", HideOthers, Some(keys::APP)),
        KeyBinding::new("cmd-q", Quit, Some(keys::APP)),
    ]);
    cx.on_action(|_: &HidePerch, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Quit, cx| cx.quit());

    cx.set_menus(vec![
        Menu {
            name: "Perch".into(),
            items: vec![
                MenuItem::action("Settings…", ToggleSettings),
                MenuItem::separator(),
                MenuItem::action("Hide Perch", HidePerch),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Perch", Quit),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::separator(),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            ],
        },
    ]);
}
