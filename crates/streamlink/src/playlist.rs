//! A recording's HLS playlist, read by the app rather than left to the player.
//!
//! The player's HLS demuxer cannot seek a fragmented-MP4 playlist in the
//! libmpv builds people actually have: after a seek it waits for a keyframe
//! that never comes, and an initial offset fails the same way. Twitch keeps
//! that kind of playlist — `init-0.mp4` and `.mp4` segments rather than `.ts`
//! — for every channel on its newer encoding path, which is most of the large
//! ones. Transport-stream playlists seek fine, but rather than two paths the
//! app positions every recording the same way: it reads the playlist once,
//! and to reach a moment it writes a playlist that *starts* at the segment
//! holding that moment and opens the player on that, with a start offset
//! inside the first segment, which the demuxer does handle. The parsing and
//! the rewriting are here, pure and tested; the file on disk and the player
//! are the app's business, in `perch::vod`.
//!
//! A broadcast still being recorded has a playlist with no end marker that
//! grows by a segment every ten seconds. The app keeps its local copy growing
//! the same way, by appending — see [`Playlist::tail`] — and the demuxer,
//! which re-reads a playlist without an end marker whenever it runs out of
//! segments, picks the new ones up. Once Twitch writes the end marker the app
//! appends it too, and the player reaches the end the ordinary way.

use std::time::Duration;

/// One segment of a recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub duration: f64,
    /// Absolute, whatever the playlist wrote: the copy the app writes lives
    /// somewhere else entirely, so nothing in it may be relative.
    pub url: String,
    /// A cut in the recording before this segment. A highlight stitched from
    /// ranges of a broadcast has them; an archive does not. Kept so the
    /// rewritten playlist says what the original said.
    pub discontinuity: bool,
}

/// A recording's playlist: where its pieces are and how long each is.
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    /// Where this came from, so it can be asked for again while it grows.
    pub url: String,
    pub version: u32,
    pub target_duration: u32,
    /// The initialisation segment of a fragmented-MP4 recording, absolute.
    /// `None` for a transport stream, which needs none.
    pub init: Option<String>,
    pub segments: Vec<Segment>,
    /// Whether the recording is complete. Without this the playlist is still
    /// growing, and so is the broadcast.
    pub ended: bool,
}

/// Fetching a playlist is one request for a few hundred kilobytes, and it
/// runs on a worker before the player opens — or every ten seconds on a
/// thread of its own while a broadcast is still going.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Read the playlist at `url`.
pub fn fetch(url: &str) -> Result<Playlist, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if status >= 400 {
        return Err(format!("the playlist answered HTTP {status}"));
    }
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("could not read the playlist: {e}"))?;
    parse(url, &text)
}

/// Parse the playlist at `url` from its text. Segment and init URLs come out
/// absolute, resolved against `url`.
pub fn parse(url: &str, text: &str) -> Result<Playlist, String> {
    if !text.trim_start().starts_with("#EXTM3U") {
        return Err("not an HLS playlist".to_string());
    }
    let mut playlist = Playlist {
        url: url.to_string(),
        version: 3,
        target_duration: 10,
        init: None,
        segments: Vec::new(),
        ended: false,
    };
    let mut duration: Option<f64> = None;
    let mut discontinuity = false;

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(rest) = line.strip_prefix("#EXT-X-VERSION:") {
            playlist.version = rest.trim().parse().unwrap_or(3);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            playlist.target_duration = rest.trim().parse().unwrap_or(10);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            if let Some(uri) = attribute(rest, "URI") {
                playlist.init = Some(absolute(url, &uri));
            }
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            duration = rest.split(',').next().and_then(|d| d.trim().parse().ok());
        } else if line == "#EXT-X-DISCONTINUITY" {
            discontinuity = true;
        } else if line == "#EXT-X-ENDLIST" {
            playlist.ended = true;
        } else if line.starts_with('#') {
            // Program dates, Twitch's own tags, comments: nothing the player
            // needs to be told again.
        } else if let Some(duration) = duration.take() {
            playlist.segments.push(Segment {
                duration,
                url: absolute(url, line),
                discontinuity,
            });
            discontinuity = false;
        }
    }

    if playlist.segments.is_empty() {
        return Err("the playlist lists no segments".to_string());
    }
    Ok(playlist)
}

/// The value of `KEY="..."` in an attribute list.
fn attribute(list: &str, key: &str) -> Option<String> {
    let start = list.find(&format!("{key}=\""))? + key.len() + 2;
    let end = list[start..].find('"')? + start;
    Some(list[start..end].to_string())
}

/// `reference` as written in a playlist at `base`, made absolute.
fn absolute(base: &str, reference: &str) -> String {
    if reference.contains("://") {
        return reference.to_string();
    }
    if let Some(path) = reference.strip_prefix('/') {
        // Scheme and host of the base, then the absolute path.
        let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
        let host_end = base[scheme_end..]
            .find('/')
            .map(|i| i + scheme_end)
            .unwrap_or(base.len());
        return format!("{}/{}", &base[..host_end], path);
    }
    match base.rfind('/') {
        Some(slash) => format!("{}/{}", &base[..slash], reference),
        None => reference.to_string(),
    }
}

impl Playlist {
    /// How long the recording is, as far as this playlist knows.
    pub fn length(&self) -> f64 {
        self.segments.iter().map(|segment| segment.duration).sum()
    }

    /// The segment holding `secs`, and how many seconds come before it. A
    /// time past the end lands in the last segment, so a seek to the end of
    /// a finished recording finishes it rather than failing.
    pub fn segment_at(&self, secs: f64) -> (usize, f64) {
        let mut before = 0.0;
        for (index, segment) in self.segments.iter().enumerate() {
            if secs < before + segment.duration {
                return (index, before);
            }
            before += segment.duration;
        }
        let last = self.segments.len().saturating_sub(1);
        let last_duration = self.segments.last().map_or(0.0, |s| s.duration);
        (last, (before - last_duration).max(0.0))
    }

    /// How often to ask for the playlist again while it is growing: once per
    /// segment, which is what the demuxer does too.
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(self.target_duration.clamp(2, 30) as u64)
    }

    /// The playlist as the player should read it, starting at segment
    /// `first`: absolute URLs, the init segment, and the end marker if the
    /// recording has one.
    pub fn render_from(&self, first: usize) -> String {
        let mut out = format!(
            "#EXTM3U\n#EXT-X-VERSION:{}\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{first}\n",
            self.version, self.target_duration
        );
        // The type is what makes a playlist with no end marker seekable at
        // all: ffmpeg treats an unfinished playlist as unseekable unless it is
        // an EVENT one, and then a start offset inside the first segment is
        // silently dropped and playback begins at the segment boundary.
        // Twitch writes EVENT on every recording; so does this.
        out.push_str(if self.ended {
            "#EXT-X-PLAYLIST-TYPE:VOD\n"
        } else {
            "#EXT-X-PLAYLIST-TYPE:EVENT\n"
        });
        if let Some(init) = &self.init {
            out.push_str(&format!("#EXT-X-MAP:URI=\"{init}\"\n"));
        }
        out.push_str(&self.tail(first));
        out
    }

    /// The segment lines from `from` on, and the end marker if the recording
    /// has one: what gets appended to a local copy that was written up to
    /// `from` while the broadcast was still going.
    pub fn tail(&self, from: usize) -> String {
        let mut out = String::new();
        for segment in self.segments.iter().skip(from) {
            if segment.discontinuity {
                out.push_str("#EXT-X-DISCONTINUITY\n");
            }
            out.push_str(&format!(
                "#EXTINF:{:.3},\n{}\n",
                segment.duration, segment.url
            ));
        }
        if self.ended {
            out.push_str("#EXT-X-ENDLIST\n");
        }
        out
    }

    /// Take a newer reading of the same playlist: any segments beyond what
    /// this one has, and the end marker once it appears. Segments already
    /// held are kept as they are — a broadcast never rewrites its past.
    pub fn absorb(&mut self, fresh: Playlist) {
        if fresh.segments.len() > self.segments.len() {
            let known = self.segments.len();
            self.segments.extend(fresh.segments.into_iter().skip(known));
        }
        self.ended |= fresh.ended;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://cdn.test/abc_someone_1_2/720p60/index-muted-X.m3u8";

    /// A fragmented-MP4 playlist as Twitch writes it, cut short.
    const FMP4: &str = "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:10\n#ID3-EQUIV-TDTG:2026-09-08T08:58:33\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-TWITCH-ELAPSED-SECS:0.000\n#EXT-X-TWITCH-TOTAL-SECS:25.500\n#EXT-X-MAP:URI=\"init-0.mp4\"\n#EXTINF:10.000,\n0.mp4\n#EXTINF:10.000,\n1.mp4\n#EXTINF:5.500,\n2-muted.mp4\n#EXT-X-ENDLIST\n";

    /// A transport-stream playlist, still being recorded.
    const TS: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:12\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PROGRAM-DATE-TIME:2026-09-08T13:01:54.435Z\n#EXTINF:10.000,\n0.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:10.000,\n1.ts\n";

    #[test]
    fn a_fragmented_playlist_parses_with_its_init_segment_and_end() {
        let playlist = parse(BASE, FMP4).unwrap();
        assert_eq!(playlist.version, 6);
        assert_eq!(playlist.target_duration, 10);
        assert_eq!(
            playlist.init.as_deref(),
            Some("https://cdn.test/abc_someone_1_2/720p60/init-0.mp4")
        );
        assert_eq!(playlist.segments.len(), 3);
        assert_eq!(
            playlist.segments[2].url,
            "https://cdn.test/abc_someone_1_2/720p60/2-muted.mp4"
        );
        assert_eq!(playlist.segments[2].duration, 5.5);
        assert!(playlist.ended);
        assert_eq!(playlist.length(), 25.5);
    }

    #[test]
    fn a_transport_stream_playlist_parses_and_is_still_growing() {
        let playlist = parse(BASE, TS).unwrap();
        assert_eq!(playlist.init, None);
        assert!(!playlist.ended);
        assert_eq!(playlist.segments.len(), 2);
        assert!(playlist.segments[1].discontinuity);
        assert!(!playlist.segments[0].discontinuity);
        assert_eq!(playlist.refresh_interval(), Duration::from_secs(12));
    }

    #[test]
    fn what_is_not_a_playlist_is_refused() {
        assert!(parse(BASE, "<html>nope</html>").is_err());
        assert!(
            parse(BASE, "#EXTM3U\n#EXT-X-VERSION:3\n").is_err(),
            "no segments"
        );
    }

    #[test]
    fn references_come_out_absolute_whatever_form_they_took() {
        assert_eq!(
            absolute(BASE, "5.mp4"),
            "https://cdn.test/abc_someone_1_2/720p60/5.mp4"
        );
        assert_eq!(
            absolute(BASE, "/other/init.mp4"),
            "https://cdn.test/other/init.mp4"
        );
        assert_eq!(
            absolute(BASE, "https://elsewhere.test/x.ts"),
            "https://elsewhere.test/x.ts"
        );
    }

    /// The whole point: which segment a second lives in, and what comes
    /// before it, including past the end.
    #[test]
    fn a_second_is_found_in_its_segment() {
        let playlist = parse(BASE, FMP4).unwrap();
        assert_eq!(playlist.segment_at(0.0), (0, 0.0));
        assert_eq!(playlist.segment_at(9.999), (0, 0.0));
        assert_eq!(playlist.segment_at(10.0), (1, 10.0));
        assert_eq!(playlist.segment_at(23.0), (2, 20.0));
        assert_eq!(
            playlist.segment_at(99.0),
            (2, 20.0),
            "past the end lands in the last"
        );
    }

    #[test]
    fn a_rewritten_playlist_starts_where_asked_and_keeps_what_the_player_needs() {
        let playlist = parse(BASE, FMP4).unwrap();
        let text = playlist.render_from(1);
        assert_eq!(
            text,
            "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:1\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"https://cdn.test/abc_someone_1_2/720p60/init-0.mp4\"\n#EXTINF:10.000,\nhttps://cdn.test/abc_someone_1_2/720p60/1.mp4\n#EXTINF:5.500,\nhttps://cdn.test/abc_someone_1_2/720p60/2-muted.mp4\n#EXT-X-ENDLIST\n"
        );

        let growing = parse(BASE, TS).unwrap();
        let text = growing.render_from(0);
        assert!(!text.contains("ENDLIST"), "a growing playlist has no end");
        assert!(
            text.contains("#EXT-X-PLAYLIST-TYPE:EVENT\n"),
            "and is an EVENT playlist, which is what keeps it seekable"
        );
        assert!(text.contains(
            "#EXT-X-DISCONTINUITY\n#EXTINF:10.000,\nhttps://cdn.test/abc_someone_1_2/720p60/1.ts\n"
        ));
    }

    /// Growth is appended, never rewritten: the lines after what was written
    /// so far, and the end marker on its own once that is all that is new.
    #[test]
    fn a_growing_playlist_is_absorbed_and_its_new_tail_appended() {
        let mut held = parse(BASE, TS).unwrap();
        let fresh = parse(BASE, &format!("{TS}#EXTINF:8.000,\n2.ts\n#EXT-X-ENDLIST\n")).unwrap();
        held.absorb(fresh);
        assert_eq!(held.segments.len(), 3);
        assert!(held.ended);
        assert_eq!(
            held.tail(2),
            "#EXTINF:8.000,\nhttps://cdn.test/abc_someone_1_2/720p60/2.ts\n#EXT-X-ENDLIST\n"
        );
        assert_eq!(held.tail(3), "#EXT-X-ENDLIST\n");

        // A reading with nothing new changes nothing.
        let same = held.clone();
        held.absorb(same);
        assert_eq!(held.segments.len(), 3);
    }
}
