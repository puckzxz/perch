//! The design system's audit, as a test.
//!
//! `theme.rs` is the one source of colour, spacing, type and size, and
//! `controls.rs` of the one shadow. Nothing in the language says so: a
//! literal compiles as well as a token, and the audit that found them was a
//! list of greps in HANDOFF that somebody had to remember to run. This reads
//! the app's sources and fails on the first one, so a value written where
//! it is used does not get as far as a review.
//!
//! It reads source text rather than anything compiled, since the crate is a
//! binary with no library half for a test to link against, and since what
//! it guards is how the code is written. Comments are left out, so prose
//! may say `rgb(` freely; whitespace is left out, so formatting cannot hide
//! a call.
//!
//! Each check names the files allowed to hold what it looks for. That list
//! is the ledger: it only gets shorter, and a value comes off it by
//! becoming a token.
//!
//! Not checked here, and still worth a look after UI work (HANDOFF, "Design
//! system"): durations. A poll, a toast's life and a frame wait are real
//! timings and belong where they are used; only an animation's belongs in
//! `theme.rs`, and telling the two apart takes a reader.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.rs` file under the app's `src`, by its path relative to it, with
/// forward slashes, and its code: comments and whitespace out.
fn sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, found: &mut Vec<(String, String)>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|error| panic!("{dir:?}: {error}"));
        for entry in entries {
            let path: PathBuf = entry.expect("an unreadable entry").path();
            if path.is_dir() {
                walk(&path, root, found);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let text =
                    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
                let name = path
                    .strip_prefix(root)
                    .expect("a source outside src")
                    .to_string_lossy()
                    .replace('\\', "/");
                found.push((name, code_of(&text)));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    walk(&root, &root, &mut found);
    assert!(
        found.len() > 40,
        "only {} sources found under {root:?}: the walk is looking in the wrong place",
        found.len()
    );
    found.sort();
    found
}

/// `text` with its line comments and all its whitespace out. A `//` inside
/// a string literal — a URL — goes with what follows it on its line, which
/// costs nothing here: no check looks for anything a URL holds.
fn code_of(text: &str) -> String {
    text.lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .flat_map(str::split_whitespace)
        .collect()
}

/// Every place `needle` is in `code` with one of `after` straight after it
/// and, where `word` is set, no letter, digit or underscore straight
/// before, so `rgb(` is not found inside `to_rgb(`.
fn hits(code: &str, needle: &str, after: &str, word: bool) -> Vec<String> {
    code.match_indices(needle)
        .filter(|&(at, _)| {
            let next = code[at + needle.len()..].chars().next();
            let before = code[..at].chars().next_back();
            next.is_some_and(|next| after.contains(next))
                && !(word && before.is_some_and(|before| before.is_alphanumeric() || before == '_'))
        })
        .map(|(at, _)| {
            let end = (at + needle.len() + 12).min(code.len());
            // On a character boundary, whatever the source holds.
            let end = (end..=code.len())
                .find(|&end| code.is_char_boundary(end))
                .unwrap_or(code.len());
            code[at..end].to_string()
        })
        .collect()
}

/// Fail on every hit of `needles` (each with one of `after` straight after
/// it) in a file not among `allowed`, saying `rule`.
fn forbid(rule: &str, needles: &[&str], after: &str, word: bool, allowed: &[&str]) {
    let mut found = Vec::new();
    for (name, code) in sources() {
        if allowed.contains(&name.as_str()) {
            continue;
        }
        for needle in needles {
            for hit in hits(&code, needle, after, word) {
                found.push(format!("{name}: {hit}"));
            }
        }
    }
    assert!(found.is_empty(), "{rule}:\n  {}", found.join("\n  "));
}

const DIGITS: &str = "0123456789";
const NONZERO: &str = "123456789";

/// A colour is a token: nothing outside `theme.rs`, and the file that hands
/// the theme to the widget library, writes one out.
#[test]
fn colours_come_from_the_theme() {
    forbid(
        "a colour written where it is used; it wants a token in theme.rs",
        &["rgb(", "rgba(", "hsla(", "hsl("],
        "0123456789.-",
        true,
        &["theme.rs", "widget_theme.rs"],
    );
}

/// Spacing is named by role in `theme.rs`: no size-named helper
/// (`.gap_2()`), and no literal padding or gap.
#[test]
fn spacing_comes_from_the_theme() {
    let mut found = Vec::new();
    for (name, code) in sources() {
        if name == "theme.rs" {
            continue;
        }
        for helper in [".p_", ".px_", ".py_", ".gap_", ".gap_x_", ".gap_y_"] {
            for (at, _) in code.match_indices(helper) {
                let rest = &code[at + helper.len()..];
                let size: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == 'p')
                    .collect();
                if !size.is_empty() && rest[size.len()..].starts_with("()") {
                    found.push(format!("{name}: {helper}{size}()"));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "spacing by a size's name rather than a role's:\n  {}",
        found.join("\n  ")
    );
    forbid(
        "a literal padding or gap; it wants a token in theme.rs",
        &[
            ".p(px(",
            ".px(px(",
            ".py(px(",
            ".pt(px(",
            ".pb(px(",
            ".pl(px(",
            ".pr(px(",
            ".gap(px(",
            ".gap_x(px(",
            ".gap_y(px(",
        ],
        NONZERO,
        false,
        &["theme.rs"],
    );
}

/// Type is four roles and their weights, in `theme.rs`: no size by name
/// (`.text_sm()`), and no weight spelled out.
#[test]
fn type_comes_from_the_theme() {
    forbid(
        "a text size by name rather than by role",
        &[
            ".text_xs(",
            ".text_sm(",
            ".text_base(",
            ".text_lg(",
            ".text_xl(",
        ],
        ")",
        false,
        &["theme.rs"],
    );
    forbid(
        "a font weight spelled out; theme.rs has one for each role",
        &["FontWeight::"],
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        false,
        &["theme.rs"],
    );
}

/// A size is a token too. This was the ledger of the debt that was left —
/// three hairlines, two notices' widths and the settings sheet's — and it
/// is empty: nothing outside `theme.rs` sets a literal size or offset. A
/// zero is left out on purpose: the divider seams and the seek bar's time
/// label hang off zero-sized anchors by design.
#[test]
fn sizes_come_from_the_theme() {
    forbid(
        "a literal size or offset; it wants a token in theme.rs",
        &[
            ".w(px(",
            ".h(px(",
            ".min_w(px(",
            ".max_w(px(",
            ".min_h(px(",
            ".max_h(px(",
            ".size(px(",
            ".top(px(",
            ".left(px(",
            ".right(px(",
            ".bottom(px(",
        ],
        NONZERO,
        false,
        &["theme.rs"],
    );
    forbid(
        "a literal radius; theme.rs has RADIUS and RADIUS_LG",
        &[".rounded(px("],
        DIGITS,
        false,
        &["theme.rs"],
    );
}

/// One shadow, cast in one place: `controls::lifted`, which every raised
/// surface goes through (`controls::raised`). Three menus were written
/// without it, which is how it came to be one call.
#[test]
fn the_one_shadow_is_cast_in_controls() {
    forbid(
        "a shadow of its own; a raised surface goes through controls::raised or controls::lifted",
        &[
            ".shadow_sm(",
            ".shadow_md(",
            ".shadow_lg(",
            ".shadow_xl(",
            ".shadow_2xl(",
            ".shadow(",
        ],
        ")v",
        false,
        &["controls.rs"],
    );
}

/// Nothing is hidden with opacity alone: at zero a control is still there
/// to be pressed (HANDOFF, "Things not to redo"). Only `motion` fades one
/// there, on its way to `invisible()`.
#[test]
fn nothing_is_hidden_by_opacity_alone() {
    let mut found = Vec::new();
    for (name, code) in sources() {
        if name == "motion.rs" {
            continue;
        }
        for (at, call) in code.match_indices(".opacity(") {
            let rest = &code[at + call.len()..];
            let argument = rest.split(')').next().unwrap_or("");
            if is_zero(argument) {
                found.push(format!("{name}: .opacity({argument})"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "a control at zero opacity still takes a press; hide it with motion::Fade or invisible():
  {}",
        found.join(
            "
  "
        )
    );
}

/// Whether `argument`, as written between an `opacity`'s brackets, is a
/// literal nought: `0`, `0.`, `0.0`. Anything else — a variable, a half —
/// is somebody's sum, and not this audit's to judge.
fn is_zero(argument: &str) -> bool {
    !argument.is_empty()
        && argument.chars().all(|c| c == '0' || c == '.')
        && argument.starts_with('0')
}

/// The reading of the sources this audit rests on: a comment is not code,
/// whitespace does not hide a call, and a word inside another is not it.
#[test]
fn the_audit_reads_code_not_prose() {
    let code = code_of("let a = rgb( 0x1 ); // rgb(0x2) in prose\n    .gap_2 ()\n");
    assert_eq!(code, "leta=rgb(0x1);.gap_2()");
    assert_eq!(hits(&code, "rgb(", "0123456789", true).len(), 1);
    assert!(hits("to_rgb(0x1)", "rgb(", "0123456789", true).is_empty());
    assert!(hits(".w(px(0.))", ".w(px(", NONZERO, false).is_empty());
    assert_eq!(hits(".w(px(12.))", ".w(px(", NONZERO, false).len(), 1);
    for zero in ["0", "0.", "0.0"] {
        assert!(is_zero(zero), "{zero}");
    }
    for other in ["", "0.5", "delta", "1.0", ".5"] {
        assert!(!is_zero(other), "{other}");
    }
}
