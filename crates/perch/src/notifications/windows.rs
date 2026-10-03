//! Desktop notifications on Windows: a toast through WinRT's
//! `ToastNotificationManager`, from a program with no package.
//!
//! A toast names the app it is from by an AppUserModelID, and Windows shows
//! nothing for an id it does not know. A packaged app has one from its
//! package; an unpackaged one has to say who it is, either through a Start
//! menu shortcut that carries the id or — what this does, since perch is a
//! loose `.exe` in a folder with no installer — a key of its own under
//! `HKCU\Software\Classes\AppUserModelId\<id>` with the name and the icon
//! the toast's header shows. The windows crate's own bindings make the
//! calls, at the version and on the features gpui already builds, so this
//! brings no new crate into the build; the registry is `windows-registry`,
//! which gpui builds too.
//!
//! Clicking a toast. The way a toast reaches the program it came from is COM:
//! a CLSID on the shortcut or the key, and a class factory the program
//! registers at startup, which an unpackaged app has to build itself. Without
//! one, the only activation a toast still has is a protocol's: it opens a
//! link, the way a click on a link anywhere would. So the toast opens
//! `perch://watch/<login>` (`target::app_link`), and the scheme is
//! registered to start this `.exe` with the link — which, with perch already
//! running, hands the link over to it (`instance`), and the window comes
//! forward with the foreground right the launch was given by the click. A
//! perch since closed starts on the channel instead.
//!
//! Both keys are written only once notifications are on and there is one to
//! show, once a session, and only where a value differs — another build of
//! perch elsewhere on disk takes the scheme over, the last one to show a
//! notification winning — and [`forget`] takes them back out when Desktop
//! notifications is turned Off. The README says what is written and how to
//! remove it by hand.

use std::error::Error;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread;

use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows_registry::{Key, CURRENT_USER};

use super::Notice;
use crate::target::{app_link, APP_SCHEME};
use crate::APP_NAME;

/// perch's AppUserModelID: the owner of the repository and the program, the
/// `Company.Product` shape Windows asks for. Never the process's own id —
/// perch does not set one, and doing so would part its taskbar button from
/// a pinned shortcut — only the name toasts are shown under.
const AUMID: &str = "puckzxz.perch";

/// The key that makes [`AUMID`] known, under `HKCU`.
fn aumid_key() -> String {
    format!(r"Software\Classes\AppUserModelId\{AUMID}")
}

/// The key that registers the `perch:` scheme, under `HKCU`.
fn scheme_key() -> String {
    format!(r"Software\Classes\{APP_SCHEME}")
}

/// The app's icon, which the build stamps on the `.exe` (`build.rs`). A
/// toast's header wants a picture file rather than a resource, so the 256px
/// PNG inside it is written out beside the log ([`icon_path`]).
const ICON: &[u8] = include_bytes!("../../assets/perch.ico");

/// Whether this session has registered perch, held across each register,
/// show and forget so the three never interleave: a toast shown while the
/// keys are being taken out would be shown under an id Windows no longer
/// knows.
static REGISTERED: Mutex<bool> = Mutex::new(false);

fn registered() -> MutexGuard<'static, bool> {
    REGISTERED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Show `notices` on a thread of their own, registering perch first if this
/// session has not. WinRT needs no setup on the thread: the windows crate
/// joins the process's multithreaded apartment for a thread that has none.
pub fn show(notices: Vec<Notice>) {
    let spawned = thread::Builder::new()
        .name("notifications".into())
        .spawn(move || {
            let mut registered = registered();
            if !*registered {
                if let Err(e) = register() {
                    eprintln!("notifications: could not register perch with Windows: {e}");
                    return;
                }
                *registered = true;
            }
            for notice in &notices {
                if let Err(e) = toast(notice) {
                    eprintln!(
                        "notifications: could not show {}'s notification: {e}",
                        notice.login
                    );
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("notifications: could not start showing: {e}");
    }
}

/// Take both keys and the icon back out, on a thread of their own, if they
/// are this perch's to take: the scheme only when it starts a `perch.exe`,
/// since a `perch:` scheme that starts something else is not ours.
pub fn forget() {
    let spawned = thread::Builder::new()
        .name("notifications".into())
        .spawn(|| {
            let mut registered = registered();
            *registered = false;
            if let Err(e) = unregister() {
                eprintln!("notifications: could not take perch's registration out: {e}");
            }
        });
    if let Err(e) = spawned {
        eprintln!("notifications: could not start taking the registration out: {e}");
    }
}

/// Where the toast's header icon is written: beside the log, in
/// `%LOCALAPPDATA%\perch`.
fn icon_path() -> PathBuf {
    crate::diagnostics::log_path().with_file_name(format!("{APP_NAME}.png"))
}

/// Make perch known to Windows as the sender of its toasts and the opener of
/// its links, writing only what is not already so.
fn register() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;

    let aumid = CURRENT_USER.create(aumid_key())?;
    set(&aumid, "DisplayName", APP_NAME)?;
    // A toast without its icon is still a toast; the header falls back to a
    // blank square.
    match write_icon() {
        Ok(icon) => set(&aumid, "IconUri", &icon.to_string_lossy())?,
        Err(e) => eprintln!("notifications: no icon for the header: {e}"),
    }

    let scheme = CURRENT_USER.create(scheme_key())?;
    set(&scheme, "", &format!("URL:{APP_NAME}"))?;
    // Empty, and what makes the key a scheme rather than a file type.
    set(&scheme, "URL Protocol", "")?;
    let command = CURRENT_USER.create(format!(r"{}\shell\open\command", scheme_key()))?;
    set(&command, "", &open_command(&exe.to_string_lossy()))?;
    Ok(())
}

/// The command the scheme runs: this `.exe`, with the link as its one
/// argument. Quoted both, so a path with spaces is one argument and the link
/// is another; whatever a link might carry after a quote of its own, the
/// launch reads only the link (`launch`).
fn open_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

/// Set `name` to `value` on `key`, unless it already says so.
fn set(key: &Key, name: &str, value: &str) -> windows_registry::Result<()> {
    if key.get_string(name).is_ok_and(|current| current == value) {
        return Ok(());
    }
    key.set_string(name, value)
}

/// Write the icon's PNG out, unless it is already there as it is.
fn write_icon() -> std::io::Result<PathBuf> {
    let png = png_in_ico(ICON).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "the icon holds no PNG")
    })?;
    let path = icon_path();
    if std::fs::read(&path).is_ok_and(|on_disk| on_disk == png) {
        return Ok(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, png)?;
    Ok(path)
}

/// The PNG among an `.ico`'s images, if it has one: `make-icon.ps1` writes
/// the 256px entry as one.
///
/// The format: a six-byte header whose last two bytes count the images, then
/// sixteen bytes on each, with the image's length and where it starts as its
/// last two little-endian words.
fn png_in_ico(ico: &[u8]) -> Option<&[u8]> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let word = |at: usize| -> Option<usize> {
        let bytes = ico.get(at..at + 4)?;
        Some(u32::from_le_bytes(bytes.try_into().ok()?) as usize)
    };
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    (0..count).find_map(|index| {
        let entry = 6 + 16 * index;
        let (length, start) = (word(entry + 8)?, word(entry + 12)?);
        let image = ico.get(start..start.checked_add(length)?)?;
        image.starts_with(PNG).then_some(image)
    })
}

/// Take back what [`register`] wrote.
fn unregister() -> Result<(), Box<dyn Error>> {
    if CURRENT_USER.open(aumid_key()).is_ok() {
        CURRENT_USER.remove_tree(aumid_key())?;
    }
    let command = CURRENT_USER
        .open(format!(r"{}\shell\open\command", scheme_key()))
        .and_then(|key| key.get_string(""));
    if command.is_ok_and(|command| starts_perch(&command)) {
        CURRENT_USER.remove_tree(scheme_key())?;
    }
    match std::fs::remove_file(icon_path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Whether a scheme's `command` starts a `perch.exe`, wherever it is: the
/// shape [`open_command`] writes, the program quoted first.
fn starts_perch(command: &str) -> bool {
    let program = command
        .strip_prefix('"')
        .and_then(|rest| rest.split_once('"'))
        .map(|(program, _)| program)
        .unwrap_or_default();
    std::path::Path::new(program)
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(format!("{APP_NAME}.exe").as_str()))
}

/// Show one toast.
fn toast(notice: &Notice) -> windows::core::Result<()> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_xml(notice)))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    // A later go-live of the same channel replaces this one in the
    // notification centre rather than standing beside it.
    toast.SetTag(&HSTRING::from(notice.login.as_str()))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)
}

/// The toast's XML: the heading and up to two lines in Windows' generic
/// template, opening the channel's link when clicked. Everything Twitch
/// wrote is escaped: a title is anybody's text.
fn toast_xml(notice: &Notice) -> String {
    let texts: String = std::iter::once(&notice.heading)
        .chain(notice.lines.iter().take(2))
        .map(|text| format!("<text>{}</text>", escape(text)))
        .collect();
    format!(
        r#"<toast activationType="protocol" launch="{}"><visual><binding template="ToastGeneric">{texts}</binding></visual></toast>"#,
        escape(&app_link(&notice.login))
    )
}

/// `text` as XML character data or an attribute's value.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            // Characters XML 1.0 has no room for at all, even escaped.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(heading: &str, lines: &[&str]) -> Notice {
        Notice {
            login: "xqc".into(),
            heading: heading.into(),
            lines: lines.iter().map(|line| line.to_string()).collect(),
        }
    }

    /// The toast opens the channel's own link, says the heading then the
    /// lines, and carries anything Twitch wrote as text rather than markup.
    #[test]
    fn the_toast_says_the_notice_and_opens_the_channel() {
        let xml = toast_xml(&notice(
            "xQc went live",
            &["Just Chatting", "<b>react</b> & \"vibe\"\u{7}"],
        ));
        assert_eq!(
            xml,
            concat!(
                r#"<toast activationType="protocol" launch="perch://watch/xqc">"#,
                r#"<visual><binding template="ToastGeneric">"#,
                "<text>xQc went live</text>",
                "<text>Just Chatting</text>",
                "<text>&lt;b&gt;react&lt;/b&gt; &amp; &quot;vibe&quot;</text>",
                "</binding></visual></toast>",
            )
        );

        let bare = toast_xml(&notice("forsen went live", &[]));
        assert_eq!(bare.matches("<text>").count(), 1, "{bare}");
    }

    /// The scheme starts this `.exe` with the link as one argument, and only
    /// a scheme that starts a perch is taken back out.
    #[test]
    fn the_scheme_starts_perch_with_the_link() {
        let command = open_command(r"C:\Program Files\perch\perch.exe");
        assert_eq!(command, r#""C:\Program Files\perch\perch.exe" "%1""#);
        assert!(starts_perch(&command));
        assert!(starts_perch(r#""D:\x\PERCH.EXE" "%1""#));
        assert!(!starts_perch(r#""C:\Tools\other.exe" "%1""#));
        assert!(
            !starts_perch(r#"C:\perch.exe "%1""#),
            "not the shape perch writes"
        );
        assert!(!starts_perch(""));
    }

    /// The icon the build stamps on the `.exe` holds the PNG a toast's header
    /// is given; an `.ico` without one, or cut short, gives nothing.
    #[test]
    fn the_icon_has_a_png_for_the_header() {
        let png = png_in_ico(ICON).expect("make-icon.ps1 writes the 256px entry as a PNG");
        assert!(png.len() > 1000, "{} bytes", png.len());
        assert_eq!(png_in_ico(&ICON[..20]), None);
        assert_eq!(png_in_ico(&[0, 0, 1, 0, 0, 0]), None);
    }
}
