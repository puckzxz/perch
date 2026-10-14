//! Which streamlink is new enough for the arguments perch passes it.
//!
//! An older one does not fail in a way that names itself: argparse refuses
//! `--twitch-supported-codecs` on stderr, which nothing reads during the
//! probe, and the pane said `streamlink returned unreadable JSON`. The
//! `--version` that finding the binary already runs says which one it is,
//! so it is read here instead of thrown away.

/// The oldest streamlink perch runs, as (major, minor).
///
/// 8.0.0 (11 November 2025) added `--twitch-supported-codecs`, which every
/// run passes ([`crate::twitch_args`]), and it fixed the access token after
/// Twitch retired the persisted query older releases asked for.
pub(crate) const MINIMUM: (u32, u32) = (8, 0);

/// The (major, minor) of what `streamlink --version` printed, which is
/// `streamlink 8.6.2`, or a build's longer form of it
/// (`streamlink 8.6.2+5.g1234abc`). `None` for anything else.
pub(crate) fn parse(output: &str) -> Option<(u32, u32)> {
    let version = output.split_whitespace().nth(1)?;
    let mut parts = version.split('.');
    let major = leading_number(parts.next()?)?;
    let minor = parts.next().and_then(leading_number).unwrap_or(0);
    Some((major, minor))
}

fn leading_number(part: &str) -> Option<u32> {
    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Why the streamlink that printed `output` will not do, if it will not:
/// the sentence a pane shows. One whose version cannot be read is let
/// through, since refusing a working build over the shape of its version
/// string would be the worse mistake.
pub(crate) fn too_old(output: &str) -> Option<String> {
    let found = parse(output)?;
    (found < MINIMUM).then(|| {
        let shown = output.split_whitespace().nth(1).unwrap_or("found");
        format!(
            "streamlink {shown} is too old. Update it to {}.{} or later.",
            MINIMUM.0, MINIMUM.1
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_reads_as_major_and_minor() {
        assert_eq!(parse("streamlink 8.6.2\n"), Some((8, 6)));
        assert_eq!(parse("streamlink 8.6.2+5.g1234abc"), Some((8, 6)));
        assert_eq!(parse("streamlink 10.0.0.dev3"), Some((10, 0)));
        assert_eq!(parse("streamlink 9"), Some((9, 0)));
        assert_eq!(parse("streamlink"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("streamlink unknown"), None);
    }

    #[test]
    fn only_a_version_known_to_be_old_is_refused() {
        assert_eq!(
            too_old("streamlink 7.5.0").as_deref(),
            Some("streamlink 7.5.0 is too old. Update it to 8.0 or later.")
        );
        assert_eq!(too_old("streamlink 8.0.0"), None);
        assert_eq!(too_old("streamlink 8.6.2"), None);
        assert_eq!(too_old("streamlink 12.1.0"), None);
        // Unreadable is not old.
        assert_eq!(too_old("something else entirely"), None);
        assert_eq!(too_old(""), None);
    }
}
