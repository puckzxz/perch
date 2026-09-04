//! The time, written the way this machine writes it.
//!
//! Chat stamps every minute, and a stamp is read a few hundred times an
//! evening, so it should look like the clock in the corner of the screen
//! rather than like a log file. That means the system's short time format —
//! `11:41 PM` on a US machine, `23:41` on a German one, `午後11:41` on a
//! Japanese one — and the operating system is the only thing that knows which.
//!
//! Each platform is asked for its *pattern* rather than for a formatted string,
//! so the rendering is one small formatter with tests rather than a call into
//! the OS per row. The two grammars in play — Windows' `h:mm tt` and ICU's
//! `h:mm a` on macOS — overlap enough that one walker reads both.

use std::sync::OnceLock;

use chrono::Timelike;

/// A short time format, as the platform describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// The field pattern: `h:mm tt`, `HH:mm`, `h:mm a`.
    pattern: String,
    /// The two day-period designators, in the user's language.
    am: String,
    pm: String,
}

/// What everything falls back to: what chat used before this module existed,
/// and what most of the world's locales want anyway.
const TWENTY_FOUR_HOUR: &str = "HH:mm";

impl Pattern {
    fn new(pattern: impl Into<String>, am: impl Into<String>, pm: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            am: am.into(),
            pm: pm.into(),
        }
    }

    fn twenty_four_hour() -> Self {
        Self::new(TWENTY_FOUR_HOUR, "AM", "PM")
    }

    /// Render one time of day.
    ///
    /// Fields, from both grammars: `h`/`hh` twelve-hour, `H`/`HH` twenty-four,
    /// `K`/`k` ICU's zero- and one-based variants, `m`/`mm` minutes, `s`/`ss`
    /// seconds, `t`/`tt` Windows' one- and two-character day period, `a` ICU's.
    /// Anything in single quotes is literal, `''` is a quote, and any other
    /// character stands for itself.
    fn render(&self, hour: u32, minute: u32, second: u32) -> String {
        let mut out = String::new();
        let chars: Vec<char> = self.pattern.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' {
                // A quoted literal, or the escaped quote `''`.
                if chars.get(i + 1) == Some(&'\'') {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                let mut j = i + 1;
                while j < chars.len() && chars[j] != '\'' {
                    out.push(chars[j]);
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            if !c.is_ascii_alphabetic() {
                out.push(c);
                i += 1;
                continue;
            }
            let mut run = 1;
            while chars.get(i + run) == Some(&c) {
                run += 1;
            }
            let padded = run >= 2;
            let period = if hour < 12 { &self.am } else { &self.pm };
            match c {
                'h' => push_number(&mut out, twelve_hour(hour), padded),
                'H' => push_number(&mut out, hour, padded),
                'K' => push_number(&mut out, hour % 12, padded),
                'k' => push_number(&mut out, if hour == 0 { 24 } else { hour }, padded),
                'm' => push_number(&mut out, minute, padded),
                's' => push_number(&mut out, second, padded),
                'a' => out.push_str(period),
                't' => {
                    if padded {
                        out.push_str(period);
                    } else {
                        out.extend(period.chars().next());
                    }
                }
                // A field this walker does not know. Leaving the letters in
                // place is wrong, but visibly so; dropping them would hide a
                // pattern this code should learn about.
                other => out.extend(std::iter::repeat_n(other, run)),
            }
            i += run;
        }
        out
    }
}

fn twelve_hour(hour: u32) -> u32 {
    match hour % 12 {
        0 => 12,
        h => h,
    }
}

fn push_number(out: &mut String, value: u32, padded: bool) {
    if padded {
        out.push_str(&format!("{value:02}"));
    } else {
        out.push_str(&value.to_string());
    }
}

/// The system's pattern, asked for once.
fn system() -> &'static Pattern {
    static PATTERN: OnceLock<Pattern> = OnceLock::new();
    PATTERN.get_or_init(|| platform::pattern().unwrap_or_else(Pattern::twenty_four_hour))
}

/// `when`, as a short time in the system's format.
pub fn stamp(when: chrono::DateTime<chrono::Local>) -> String {
    system().render(when.hour(), when.minute(), when.second())
}

#[cfg(windows)]
mod platform {
    //! `GetLocaleInfoEx` with the user's default locale, which is the Region
    //! page in Settings — including a short time format the user has edited by
    //! hand, which is exactly the thing worth honouring.

    use super::Pattern;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocaleInfoEx(
            locale_name: *const u16,
            lc_type: u32,
            data: *mut u16,
            data_len: i32,
        ) -> i32;
    }

    /// From winnls.h.
    const LOCALE_SSHORTTIME: u32 = 0x0079;
    const LOCALE_S1159: u32 = 0x0028;
    const LOCALE_S2359: u32 = 0x0029;

    fn locale_info(kind: u32) -> Option<String> {
        let mut buffer = [0u16; 128];
        // SAFETY: a null locale name means the user default; the buffer and
        // its length are passed together, and the call writes at most that
        // many UTF-16 units including the terminator.
        let written = unsafe {
            GetLocaleInfoEx(
                std::ptr::null(),
                kind,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        if written <= 0 {
            return None;
        }
        // `written` counts the terminating NUL.
        let units = &buffer[..(written as usize).saturating_sub(1)];
        Some(String::from_utf16_lossy(units))
    }

    pub fn pattern() -> Option<Pattern> {
        let pattern = locale_info(LOCALE_SSHORTTIME).filter(|p| !p.trim().is_empty())?;
        // The designators may be empty on a locale that never uses them,
        // which is fine: such a pattern has no `t` in it.
        let am = locale_info(LOCALE_S1159).unwrap_or_default();
        let pm = locale_info(LOCALE_S2359).unwrap_or_default();
        Some(Pattern::new(pattern, am, pm))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    //! A `CFDateFormatter` with the short time style for the current locale.
    //! Its format is an ICU pattern, and its AM/PM symbols are already in the
    //! user's language.

    use std::ffi::{c_char, c_void};

    use super::Pattern;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyCurrent() -> CFTypeRef;
        fn CFDateFormatterCreate(
            allocator: CFTypeRef,
            locale: CFTypeRef,
            date_style: CFIndex,
            time_style: CFIndex,
        ) -> CFTypeRef;
        fn CFDateFormatterGetFormat(formatter: CFTypeRef) -> CFStringRef;
        fn CFDateFormatterCopyProperty(formatter: CFTypeRef, key: CFStringRef) -> CFTypeRef;
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFRelease(cf: CFTypeRef);
        static kCFDateFormatterAMSymbol: CFStringRef;
        static kCFDateFormatterPMSymbol: CFStringRef;
    }

    const NO_STYLE: CFIndex = 0;
    const SHORT_STYLE: CFIndex = 1;
    const UTF8: u32 = 0x0800_0100;

    /// Copy a CFString out. `None` for a null reference.
    ///
    /// # Safety
    /// `string` must be null or a valid CFString.
    unsafe fn text(string: CFStringRef) -> Option<String> {
        if string.is_null() {
            return None;
        }
        let mut buffer = [0 as c_char; 256];
        let ok = unsafe {
            CFStringGetCString(string, buffer.as_mut_ptr(), buffer.len() as CFIndex, UTF8)
        };
        if ok == 0 {
            return None;
        }
        let bytes: Vec<u8> = buffer
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8(bytes).ok()
    }

    pub fn pattern() -> Option<Pattern> {
        // SAFETY: every Create/Copy result is released exactly once below, the
        // Get result is borrowed from the formatter for as long as it lives,
        // and the symbol keys are process-wide constants.
        unsafe {
            let locale = CFLocaleCopyCurrent();
            if locale.is_null() {
                return None;
            }
            let formatter = CFDateFormatterCreate(std::ptr::null(), locale, NO_STYLE, SHORT_STYLE);
            CFRelease(locale);
            if formatter.is_null() {
                return None;
            }
            let pattern = text(CFDateFormatterGetFormat(formatter));
            let am_symbol = CFDateFormatterCopyProperty(formatter, kCFDateFormatterAMSymbol);
            let pm_symbol = CFDateFormatterCopyProperty(formatter, kCFDateFormatterPMSymbol);
            let am = text(am_symbol).unwrap_or_else(|| "AM".into());
            let pm = text(pm_symbol).unwrap_or_else(|| "PM".into());
            if !am_symbol.is_null() {
                CFRelease(am_symbol);
            }
            if !pm_symbol.is_null() {
                CFRelease(pm_symbol);
            }
            CFRelease(formatter);
            pattern
                .filter(|p| !p.trim().is_empty())
                .map(|p| Pattern::new(p, am, pm))
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    //! No platform API is asked here. The locale environment names the
    //! territory, and the handful of territories whose clocks say AM and PM
    //! are few enough to list.

    use super::Pattern;

    const TWELVE_HOUR: [&str; 8] = [
        "en_US", "en_CA", "en_AU", "en_NZ", "en_IN", "en_PH", "es_US", "hi_IN",
    ];

    pub fn pattern() -> Option<Pattern> {
        let locale = ["LC_ALL", "LC_TIME", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())?;
        let territory = locale.split(['.', '@']).next().unwrap_or(&locale);
        TWELVE_HOUR
            .contains(&territory)
            .then(|| Pattern::new("h:mm tt", "AM", "PM"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us() -> Pattern {
        Pattern::new("h:mm tt", "AM", "PM")
    }

    #[test]
    fn a_windows_twelve_hour_pattern_reads_like_the_taskbar() {
        assert_eq!(us().render(23, 41, 0), "11:41 PM");
        assert_eq!(us().render(0, 5, 0), "12:05 AM");
        assert_eq!(us().render(12, 0, 0), "12:00 PM");
        assert_eq!(us().render(9, 7, 0), "9:07 AM");
    }

    #[test]
    fn a_twenty_four_hour_pattern_pads_the_hour() {
        let de = Pattern::new("HH:mm", "", "");
        assert_eq!(de.render(9, 7, 0), "09:07");
        assert_eq!(de.render(23, 41, 0), "23:41");
        let fr = Pattern::new("H:mm", "", "");
        assert_eq!(fr.render(9, 7, 0), "9:07");
    }

    #[test]
    fn an_icu_pattern_uses_the_locale_symbols() {
        let ja = Pattern::new("aK:mm", "午前", "午後");
        assert_eq!(ja.render(23, 41, 0), "午後11:41");
        assert_eq!(ja.render(0, 5, 0), "午前0:05");
        let us_icu = Pattern::new("h:mm a", "AM", "PM");
        assert_eq!(us_icu.render(13, 30, 0), "1:30 PM");
    }

    #[test]
    fn a_single_t_is_the_first_letter_of_the_designator() {
        let short = Pattern::new("h:mm t", "AM", "PM");
        assert_eq!(short.render(13, 30, 0), "1:30 P");
    }

    #[test]
    fn quoted_text_is_literal_and_seconds_are_available() {
        let odd = Pattern::new("HH'h'mm''ss", "", "");
        assert_eq!(odd.render(9, 7, 3), "09h07'03");
        let at = Pattern::new("'at 'h:mm a", "am", "pm");
        assert_eq!(at.render(9, 7, 0), "at 9:07 am");
        let k = Pattern::new("kk:mm", "", "");
        assert_eq!(k.render(0, 1, 0), "24:01");
    }

    #[test]
    fn the_fallback_is_what_chat_always_showed() {
        assert_eq!(Pattern::twenty_four_hour().render(23, 41, 0), "23:41");
    }

    /// Whatever this machine says, it has to be something a person would
    /// recognise as a time: digits, a separator, and nothing else surprising.
    #[test]
    fn the_system_pattern_renders_a_plausible_time() {
        let text = system().render(15, 4, 0);
        assert!(text.contains("04"), "no minutes in {text:?}");
        assert!(
            text.contains("15") || text.contains('3'),
            "no hour in {text:?}"
        );
    }
}
