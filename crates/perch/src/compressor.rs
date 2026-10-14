//! The audio compressor: More's `Audio compressor · Off` and `· On`, one
//! answer for every pane, which evens out how loud each stream is — a
//! streamer who runs hot comes down, one who mumbles comes up.
//!
//! One answer rather than one per pane, because what it is for is the
//! difference *between* streams: a compressor on the loud pane and not the
//! quiet one would just move the gap. It is remembered
//! (`Settings::compressor`), and it lives in one process-wide flag
//! ([`set`], [`on`]) that every player's render thread reads between frames,
//! the way it reads its volume. A flag rather than a value handed to each
//! player because players are started from several places — a pane, a
//! rendition resolved beside it, a quality started ahead, a pop-out, the
//! mini player — and one of them handed the old answer would play on
//! uncompressed with the row saying otherwise.
//!
//! What it does is FrankerFaceZ's player compressor as the browser runs it.
//! FFZ hands Chrome's `DynamicsCompressorNode` a threshold of −50 dB, a
//! 40 dB knee, 12:1, no attack and a 250 ms release, with nothing after it,
//! and Chrome's kernel (Blink's `dynamics_compressor.cc`) does the rest: it
//! listens to the louder channel's peak 6 ms ahead of what it plays, bends
//! from −50 dB over the 40 dB knee to 12:1 above −10, and adds a makeup gain
//! of its own, which for FFZ's numbers is +12.45 dB.
//!
//! Those numbers cannot be handed to ffmpeg as they are. Chrome's threshold
//! is where its knee starts; ffmpeg's is the middle of a knee at most ±9 dB
//! wide, so −50 dB there compresses far harder than FFZ does — and ffmpeg
//! adds no makeup, so whatever is set in its place is a guess. The chain is
//! fitted instead to what Chrome itself puts out (see `HANDOFF.md`, "The
//! compressor is fitted to the browser"). Measured in October 2026, played
//! through libmpv as a pane plays it, against Chrome 152 with FFZ's
//! settings, on eight clips — speech at five levels from about −50 to −12
//! LUFS, a quiet voice that shouts, a voice leaning in and out, speech over
//! a game with explosions, music loud and quiet, pink noise steps and a
//! rising tone — its loudness, taken every 20 ms, follows the browser's to
//! within half a decibel on average (0.41 to 0.54 dB a clip), and each
//! passage of speech or music comes out within 1 dB of the browser's level
//! (1.6 dB at worst, on noise hot enough to peak past full scale). Peaks
//! stay below −4 dBFS, so there is no limiter after it. Silence comes up by
//! the same 12.45 dB the browser's makeup lifts it, so a room's hum under a
//! quiet streamer is louder with it on, as it is in FFZ.
//!
//! The pane's volume goes in before the compressor, as Twitch's player's
//! volume goes in before FFZ's: the compressor hears each pane as loud as it
//! ends up ([`drive`]), and mpv's own volume, after it, then turns the sound
//! down to match. That matters more than any of its settings, because how
//! hard the browser's compressor squeezes depends on how loud the stream
//! reaches it: under a voice, music dips by about 9 dB at full volume and by
//! under 2 dB with the volume 20 dB down (46 on the slider, which mpv
//! cubes). With the volume after the compressor instead, every pane was
//! squeezed as FFZ squeezes at 100%, and the music could be heard dipping
//! under every sentence. As under FFZ, turning a loud pane down changes it
//! less near the top of the slider, where the compressor gives some back.
//!
//! Hearing a quiet pane loud means the chain's own output runs past full
//! scale until mpv's volume takes it down — the makeup is given before the
//! volume takes it away. That is safe because mpv turns the sound down in
//! floating point and only then clips (`ao.c`, `MUL_GAIN_f`), and Windows'
//! shared output is floating point (measured: mpv asks it for doubles and
//! gets floats). An output in whole numbers would clip it first.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// mpv's name for the chain, by which a playing chain is told its pane's
/// new volume ([`drive_command`]).
pub const LABEL: &str = "compressor";

/// The chain while it is on, for a pane at `volume`, as mpv's `af` option
/// takes it: one `lavfi` graph, labelled [`LABEL`], in ffmpeg's words.
///
/// - `asplit` and `adelay`: what is heard is held 6 ms behind the copy the
///   compressor listens to, the browser's lookahead, so the gain is already
///   down when a shout arrives. Six milliseconds is far under anything
///   heard as out of step with the picture.
/// - `sidechaincompress`: what it hears the pane at, `level_sc` ([`drive`]);
///   the browser's detection, the louder channel's peak
///   (`detection=peak:link=maximum`); its makeup, 12.45 dB, which the test
///   below works out the way Chrome does; FFZ's 250 ms release; 1 ms of
///   attack, Chrome's floor under FFZ's 0; and a threshold, ratio and knee
///   (−28.5 dB, 3:1, ffmpeg's widest) fitted so the output follows the
///   browser's, rather than FFZ's own, which mean something else here.
///
/// Every option here has been in FFmpeg since 4.3 (2020). That matters:
/// mpv starts a file whose chain it cannot build with no sound at all, so
/// an option newer than the libmpv it runs on would silence every pane.
pub fn chain(volume: u8) -> String {
    format!(
        "@{LABEL}:lavfi=[asplit[heard][listened];[heard]adelay=delays=6:all=1[late];\
         [late][listened]sidechaincompress=level_sc={}:{SHAPE}]",
        drive_words(volume)
    )
}

/// The compressor's settings but what it hears the pane at; see [`chain`].
const SHAPE: &str = "threshold=-28.5dB:ratio=3:knee=8:attack=1:release=250:\
    makeup=12.45dB:detection=peak:link=maximum";

/// How loud the compressor hears a pane at `volume`: as loud as mpv then
/// plays it, the cube of the slider (mpv's volume curve), so 50 is heard
/// 18 dB down. Never quieter than [`LEAST_DRIVE`], which a pane reaches
/// at 25 and where it barely compresses anyway.
pub fn drive(volume: u8) -> f64 {
    (f64::from(volume.min(100)) / 100.0)
        .powi(3)
        .max(LEAST_DRIVE)
}

/// The lowest `level_sc` `sidechaincompress` takes, 36 dB down.
const LEAST_DRIVE: f64 = 1.0 / 64.0;

fn drive_words(volume: u8) -> String {
    format!("{:.6}", drive(volume))
}

/// What tells a playing chain its pane is now at `volume`, without
/// building it again (`Player::audio_filter_command`): the chain's label,
/// the filter in it, the setting and its value.
pub fn drive_command(volume: u8) -> (&'static str, &'static str, &'static str, String) {
    (LABEL, "sidechaincompress", "level_sc", drive_words(volume))
}

/// How often a playing chain is reminded of its pane's volume while its
/// words carry another; see [`Applied`].
const REMIND: Duration = Duration::from_millis(250);

/// One player's chain as last given to mpv, which decides what each pass
/// between frames still has to do ([`Applied::step`]).
///
/// The chain's volume lives in two places. Its words, the `af` mpv keeps,
/// are what mpv builds it from — when it is turned on, and again whenever it
/// builds it anew: a new file, a change of format. A new volume goes to the
/// chain playing instead, by command, because new words would build it
/// again and the sound would stop for a moment. So the words fall behind,
/// and a chain built from words quieter than the pane plays at would squeeze
/// too little and run hot. Perch's own reload brings the words up first
/// ([`before_reload`](Self::before_reload)); a build mpv starts by itself
/// is caught by reminding the playing chain every [`REMIND`] while the
/// words are behind.
#[derive(Clone, Copy, Debug)]
pub struct Applied {
    on: bool,
    /// The volume the chain's words carry.
    words: u8,
    /// The volume last told to the playing chain, and when.
    told: u8,
    told_at: Instant,
}

/// What a player has to do for its chain to match the compressor and its
/// pane's volume.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Give mpv these words for the chain (`Player::set_audio_filters`).
    Words(String),
    /// Tell the playing chain the pane is at this volume ([`drive_command`]).
    Tell(u8),
}

impl Applied {
    /// A player opening with the compressor `on`, at `volume`, built from
    /// [`filters`]`(on, volume)`.
    pub fn new(on: bool, volume: u8, now: Instant) -> Self {
        Self {
            on,
            words: volume,
            told: volume,
            told_at: now,
        }
    }

    /// What this pass has to do, now the compressor is `on` and the pane at
    /// `volume`: new words when it is turned on or off, at the volume now;
    /// the volume, when it has changed; the volume again, every
    /// [`REMIND`], while the words are behind it.
    pub fn step(&mut self, on: bool, volume: u8, now: Instant) -> Option<Step> {
        if on != self.on {
            *self = Self::new(on, volume, now);
            return Some(Step::Words(filters(on, volume)));
        }
        if !on {
            return None;
        }
        let behind = self.words != volume && now.duration_since(self.told_at) >= REMIND;
        if volume != self.told || behind {
            self.told = volume;
            self.told_at = now;
            return Some(Step::Tell(volume));
        }
        None
    }

    /// Words to give mpv before it opens another file in this player, which
    /// it builds the chain for from them: none while they already carry the
    /// pane's `volume`.
    pub fn before_reload(&mut self, volume: u8) -> Option<String> {
        if !self.on || self.words == volume {
            return None;
        }
        self.words = volume;
        self.told = volume;
        Some(chain(volume))
    }
}

/// Whether the compressor is on, for every player. Off until the root has
/// read the settings, which it does before any player starts.
static ON: AtomicBool = AtomicBool::new(false);

/// Turn it on or off for every player, from the settings at startup and
/// from More's row. Each player picks it up between frames.
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// Whether it is on: what a player should be filtering with, and what
/// More's row says.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// mpv's `af` for `on`, for a pane at `volume`: the chain, or nothing,
/// which clears it.
pub fn filters(on: bool, volume: u8) -> String {
    if on {
        chain(volume)
    } else {
        String::new()
    }
}

/// More's row while the compressor is `on`: its id and its words. The id
/// follows the words, as every control's whose words follow a state does
/// (`loudness::HearOnly::row`), so the row that says the other thing is a
/// new element rather than the old one relabelled under the pointer.
pub fn row(on: bool) -> (&'static str, &'static str) {
    if on {
        ("more-compressor-on", "Audio compressor · On")
    } else {
        ("more-compressor-off", "Audio compressor · Off")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(linear: f64) -> f64 {
        20.0 * linear.log10()
    }

    fn linear(db: f64) -> f64 {
        10f64.powf(db / 20.0)
    }

    /// A number a full-volume pane's chain gives `sidechaincompress`,
    /// without its `dB`.
    fn param(name: &str) -> f64 {
        const FILTER: &str = "sidechaincompress=";
        let chain = chain(100);
        let options = &chain[chain.find(FILTER).unwrap() + FILTER.len()..];
        options
            .trim_end_matches(']')
            .split(':')
            .find_map(|option| option.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("no {name} in {chain}"))
            .trim_end_matches("dB")
            .parse()
            .unwrap()
    }

    /// The browser's compressor with FrankerFaceZ's settings, as Blink's
    /// `DynamicsCompressor` computes it (`KneeCurve`, `Saturate`,
    /// `KAtSlope`): where a steady peak at `x` dBFS comes out, makeup
    /// included, and the makeup itself.
    struct Browser {
        threshold: f64,
        k: f64,
        knee_end_db: f64,
        knee_end_out_db: f64,
        slope: f64,
    }

    impl Browser {
        fn ffz() -> Self {
            let (threshold_db, knee_db, ratio) = (-50.0, 40.0, 12.0);
            let threshold = linear(threshold_db);
            let curve = |x: f64, k: f64| threshold + (1.0 - (-k * (x - threshold)).exp()) / k;
            // The knee's sharpness, found as Blink finds it: fifteen halvings
            // (in log k) toward the slope that meets the ratio where the knee
            // ends.
            let end = linear(threshold_db + knee_db);
            let (mut low, mut high, mut k) = (0.1f64, 10000.0f64, 5.0f64);
            for _ in 0..15 {
                let slope =
                    (db(curve(end * 1.001, k)) - db(curve(end, k))) / (db(end * 1.001) - db(end));
                if slope < 1.0 / ratio {
                    high = k;
                } else {
                    low = k;
                }
                k = (low * high).sqrt();
            }
            Browser {
                threshold,
                k,
                knee_end_db: threshold_db + knee_db,
                knee_end_out_db: db(curve(end, k)),
                slope: 1.0 / ratio,
            }
        }

        /// The curve alone, before the makeup.
        fn curve_db(&self, x_db: f64) -> f64 {
            let x = linear(x_db);
            if x < self.threshold {
                x_db
            } else if x_db < self.knee_end_db {
                db(self.threshold + (1.0 - (-self.k * (x - self.threshold)).exp()) / self.k)
            } else {
                self.knee_end_out_db + self.slope * (x_db - self.knee_end_db)
            }
        }

        /// `pow(1 / Saturate(1), 0.6)`: what full scale is turned down
        /// by, given back at six tenths.
        fn makeup_db(&self) -> f64 {
            -0.6 * self.curve_db(0.0)
        }

        fn out_db(&self, x_db: f64) -> f64 {
            self.curve_db(x_db) + self.makeup_db()
        }
    }

    /// Where the chain puts a steady peak at `x` dBFS: ffmpeg's
    /// `output_gain`, downward, with its Hermite knee, then the makeup.
    fn chain_out_db(x_db: f64) -> f64 {
        let (threshold, ratio, knee) = (param("threshold"), param("ratio"), param("knee"));
        let half_knee = db(knee.sqrt());
        let (start, stop) = (threshold - half_knee, threshold + half_knee);
        let compressed = |x: f64| threshold + (x - threshold) / ratio;
        let out = if x_db <= start {
            x_db
        } else if x_db < stop {
            let width = stop - start;
            let t = (x_db - start) / width;
            let (p0, p1, m0, m1) = (start, compressed(stop), width, width / ratio);
            (2.0 * p0 + m0 - 2.0 * p1 + m1) * t.powi(3)
                + (-3.0 * p0 - 2.0 * m0 + 3.0 * p1 - m1) * t.powi(2)
                + m0 * t
                + p0
        } else {
            compressed(x_db)
        };
        out + param("makeup")
    }

    #[test]
    fn off_clears_the_chain() {
        assert_eq!(filters(false, 42), "");
        assert_eq!(filters(true, 42), chain(42));
    }

    /// The chain is one labelled lavfi graph with no whitespace in it: the
    /// string's line breaks must not leave spaces inside the graph, which
    /// ffmpeg would read as part of an option's value. Its brackets pair
    /// up, which is how mpv finds where the graph ends.
    #[test]
    fn the_chain_is_one_graph_with_no_spaces() {
        for volume in [0, 42, 100] {
            let chain = chain(volume);
            let graph = chain
                .strip_prefix(&format!("@{LABEL}:lavfi="))
                .unwrap_or_else(|| panic!("{chain}"));
            assert!(graph.starts_with('[') && graph.ends_with(']'), "{chain}");
            assert!(!chain.contains(char::is_whitespace), "{chain}");
            let mut depth = 0;
            for (at, c) in graph.char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
                assert!(
                    depth > 0 || at == graph.len() - 1,
                    "the graph closes early at {at}: {chain}"
                );
            }
            assert_eq!(depth, 0, "{chain}");
        }
    }

    /// What the browser does that a static curve cannot show: it listens
    /// 6 ms ahead of what it plays, to the louder channel's peak.
    #[test]
    fn the_chain_listens_ahead_as_the_browser_does() {
        let chain = chain(100);
        assert!(chain.contains("adelay=delays=6:all=1"), "{chain}");
        assert!(chain.contains("detection=peak:link=maximum"), "{chain}");
    }

    /// The compressor hears a pane as loud as mpv then plays it, as the
    /// player's volume reaches the browser's compressor first under FFZ:
    /// all of it at 100, the cube of the slider below, never under
    /// `sidechaincompress`'s floor. And the command a new volume sends
    /// names what the chain a pane starts with says, so the two cannot
    /// drift apart.
    #[test]
    fn the_compressor_hears_a_pane_as_loud_as_it_plays() {
        assert_eq!(drive(100), 1.0);
        assert_eq!(drive(50), 0.125);
        assert_eq!(drive(25), LEAST_DRIVE);
        assert_eq!(drive(10), LEAST_DRIVE);
        assert_eq!(drive(0), LEAST_DRIVE);
        assert!((1..=100).all(|v| drive(v) >= drive(v - 1)));
        let (label, filter, setting, value) = drive_command(50);
        assert_eq!(value, "0.125000");
        let chain = chain(50);
        assert!(chain.starts_with(&format!("@{label}:")), "{chain}");
        assert!(
            chain.contains(&format!("{filter}={setting}={value}:")),
            "{chain}"
        );
    }

    /// The chain against the browser running FFZ's settings: the same
    /// makeup, which is also how far silence comes up, and a curve within
    /// 2 dB of the browser's from −60 dBFS to full scale. Not closer: the
    /// threshold, ratio and knee were fitted to what the browser puts out
    /// from real sound, where its peak detector and gain smoothing differ
    /// from ffmpeg's, and that fit is what lands within 2 dB here rather
    /// than within a fraction of one.
    #[test]
    fn the_chain_follows_ffz_in_the_browser() {
        assert_eq!(param("level_sc"), 1.0, "measured at full volume");
        let browser = Browser::ffz();
        assert!(
            (browser.makeup_db() - 12.45).abs() < 0.01,
            "the browser's makeup for FFZ's settings is {:.3} dB",
            browser.makeup_db()
        );
        assert!(
            (param("makeup") - browser.makeup_db()).abs() < 0.01,
            "the chain's makeup is {} dB, the browser's {:.2}",
            param("makeup"),
            browser.makeup_db()
        );
        for x in -60..=0 {
            let x = f64::from(x);
            let (chain, browser) = (chain_out_db(x), browser.out_db(x));
            assert!(
                (chain - browser).abs() <= 2.0,
                "a peak at {x} dBFS comes out at {chain:.2} here, {browser:.2} in the browser"
            );
        }
    }

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Turned on or off, the chain is built at the volume the pane is at
    /// then; nothing else happens while it is off.
    #[test]
    fn turning_it_on_or_off_gives_words_at_the_volume_now() {
        let t0 = Instant::now();
        let mut applied = Applied::new(false, 42, t0);
        assert_eq!(applied.step(false, 80, ms(t0, 10)), None);
        assert_eq!(applied.before_reload(80), None);
        assert_eq!(
            applied.step(true, 60, ms(t0, 20)),
            Some(Step::Words(chain(60)))
        );
        assert_eq!(applied.step(true, 60, ms(t0, 5000)), None);
        assert_eq!(
            applied.step(false, 60, ms(t0, 5010)),
            Some(Step::Words(String::new()))
        );
    }

    /// A new volume is told to the chain playing, never by new words, which
    /// would build it again; and told again while the words are behind it,
    /// in case mpv has built the chain anew from them.
    #[test]
    fn a_new_volume_is_told_and_retold_while_the_words_are_behind() {
        let t0 = Instant::now();
        let mut applied = Applied::new(true, 42, t0);
        assert_eq!(applied.step(true, 42, ms(t0, 5000)), None);
        assert_eq!(applied.step(true, 60, ms(t0, 5010)), Some(Step::Tell(60)));
        assert_eq!(applied.step(true, 60, ms(t0, 5100)), None);
        assert_eq!(applied.step(true, 60, ms(t0, 5260)), Some(Step::Tell(60)));
        // Back where the words are: nothing left to remind.
        assert_eq!(applied.step(true, 42, ms(t0, 5270)), Some(Step::Tell(42)));
        assert_eq!(applied.step(true, 42, ms(t0, 9000)), None);
    }

    /// Before another file opens, words behind the volume are brought up to
    /// it, once, so the new file's chain is built at the volume it plays at.
    #[test]
    fn a_reload_brings_the_words_up_to_the_volume() {
        let t0 = Instant::now();
        let mut applied = Applied::new(true, 42, t0);
        assert_eq!(applied.before_reload(42), None);
        assert_eq!(applied.step(true, 70, ms(t0, 10)), Some(Step::Tell(70)));
        assert_eq!(applied.before_reload(70), Some(chain(70)));
        assert_eq!(applied.before_reload(70), None);
        assert_eq!(applied.step(true, 70, ms(t0, 5000)), None);
    }

    #[test]
    fn the_row_says_what_it_is_and_changes_id_with_it() {
        let (off_id, off_words) = row(false);
        let (on_id, on_words) = row(true);
        assert_eq!(off_words, "Audio compressor · Off");
        assert_eq!(on_words, "Audio compressor · On");
        assert_ne!(off_id, on_id);
    }
}
