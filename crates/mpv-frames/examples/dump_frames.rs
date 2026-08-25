//! Render frames from any mpv-playable source to PNG, and report what came back.
//!
//! This exists to make the frame pipeline verifiable on its own, with no UI in
//! the loop. The printed stats are the fast check; the PNGs are the real one.
//!
//!     cargo run -p mpv-frames --example dump_frames -- <url-or-path> [options]
//!
//!     --size WxH     render size, default 1280x720
//!     --frames N     how many PNGs to write, default 3
//!     --skip N       frames to discard first, default 2 (early ones are often blank)
//!     --out DIR      output directory, default ./out
//!     --audio        enable audio output
//!     --hwdec        let mpv hardware-decode and copy frames back
//!     --opt K=V      pass an arbitrary mpv option (repeatable)
//!     --timeout SECS give up waiting after this long, default 20
//!     --bench SECS   skip PNGs; measure render cost for this long instead

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mpv_frames::{Config, Event, Player};

struct Args {
    source: String,
    width: u32,
    height: u32,
    frames: usize,
    skip: usize,
    out: PathBuf,
    audio: bool,
    hwdec: bool,
    opts: Vec<(String, String)>,
    timeout: Duration,
    bench: Option<Duration>,
}

fn parse_args() -> Result<Args, String> {
    let mut raw = std::env::args().skip(1);
    let source = raw.next().ok_or("missing <url-or-path>")?;

    let mut args = Args {
        source,
        width: 1280,
        height: 720,
        frames: 3,
        skip: 2,
        out: PathBuf::from("out"),
        audio: false,
        hwdec: false,
        opts: Vec::new(),
        timeout: Duration::from_secs(20),
        bench: None,
    };

    while let Some(flag) = raw.next() {
        let mut value = || raw.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--size must look like 1280x720")?;
                args.width = w.parse().map_err(|_| "bad width")?;
                args.height = h.parse().map_err(|_| "bad height")?;
            }
            "--frames" => args.frames = value()?.parse().map_err(|_| "bad --frames")?,
            "--skip" => args.skip = value()?.parse().map_err(|_| "bad --skip")?,
            "--out" => args.out = PathBuf::from(value()?),
            "--audio" => args.audio = true,
            "--hwdec" => args.hwdec = true,
            "--opt" => {
                let v = value()?;
                let (k, val) = v.split_once('=').ok_or("--opt must look like key=value")?;
                args.opts.push((k.to_string(), val.to_string()));
            }
            "--bench" => {
                let secs: u64 = value()?.parse().map_err(|_| "bad --bench")?;
                args.bench = Some(Duration::from_secs(secs));
            }
            "--timeout" => {
                let secs: u64 = value()?.parse().map_err(|_| "bad --timeout")?;
                args.timeout = Duration::from_secs(secs);
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(args)
}

/// What a rendered buffer actually contains. Cheap to compute, and enough to
/// catch every failure mode this pipeline has: all-black, transparent, and
/// channel-swapped output.
struct FrameStats {
    mean_b: f64,
    mean_g: f64,
    mean_r: f64,
    min_alpha: u8,
    nonblack_pct: f64,
    distinct_sample: usize,
}

fn analyse(bgra: &[u8]) -> FrameStats {
    let (mut sum_b, mut sum_g, mut sum_r) = (0u64, 0u64, 0u64);
    let mut min_alpha = u8::MAX;
    let mut nonblack = 0u64;
    let mut seen = std::collections::HashSet::new();

    let pixels = bgra.chunks_exact(4);
    let total = pixels.len() as u64;

    for (i, px) in bgra.chunks_exact(4).enumerate() {
        sum_b += px[0] as u64;
        sum_g += px[1] as u64;
        sum_r += px[2] as u64;
        min_alpha = min_alpha.min(px[3]);
        if px[0] > 8 || px[1] > 8 || px[2] > 8 {
            nonblack += 1;
        }
        // Sampling keeps this O(1)-ish on 1080p while still distinguishing a
        // real image from a flat fill.
        if i % 997 == 0 {
            seen.insert(u32::from_le_bytes([px[0], px[1], px[2], px[3]]));
        }
    }

    let n = total.max(1) as f64;
    FrameStats {
        mean_b: sum_b as f64 / n,
        mean_g: sum_g as f64 / n,
        mean_r: sum_r as f64 / n,
        min_alpha,
        nonblack_pct: nonblack as f64 / n * 100.0,
        distinct_sample: seen.len(),
    }
}

/// PNG wants RGBA; our buffer is BGRA. Swapping here rather than in the library
/// keeps the library's contract honest - if the PNG colours look wrong, the bug
/// is upstream in `render_bgra`, not in this converter.
fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = bgra.to_vec();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    rgba
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n");
            eprintln!("usage: dump_frames <url-or-path> [--size WxH] [--frames N]");
            eprintln!("                    [--skip N] [--out DIR] [--audio] [--timeout SECS]");
            std::process::exit(2);
        }
    };

    std::fs::create_dir_all(&args.out)?;

    let config = Config {
        audio: args.audio,
        hwdec: args.hwdec,
        extra: args.opts.clone(),
    };

    let started = Instant::now();
    let player = Player::open_with(&args.source, config)?;
    println!("libmpv:  {}", player.library_path().display());
    println!("source:  {}", args.source);
    println!("render:  {}x{} BGRA   hwdec {}", args.width, args.height, if args.hwdec { "auto-copy" } else { "no" });
    println!();

    let mut buf = vec![0u8; args.width as usize * args.height as usize * 4];

    if let Some(duration) = args.bench {
        return run_bench(&player, &args, &mut buf, duration);
    }

    let mut rendered = 0usize;
    let mut saved = 0usize;
    let deadline = Instant::now() + args.timeout;
    let mut reported_source = false;

    while saved < args.frames {
        if Instant::now() > deadline {
            eprintln!("\ntimed out after {:?} with {saved} frame(s) saved", args.timeout);
            if rendered == 0 {
                eprintln!("no frames rendered at all - mpv never produced video.");
                std::process::exit(1);
            }
            break;
        }

        for event in player.poll_events() {
            match event {
                Event::EndFile => {
                    eprintln!("mpv reported end-of-file (bad URL, or the stream ended)");
                    if saved == 0 {
                        std::process::exit(1);
                    }
                    return Ok(());
                }
                Event::Shutdown => return Ok(()),
                _ => {}
            }
        }

        if !player.wait_for_frame(Duration::from_millis(250)) {
            continue;
        }

        player.render_bgra(args.width, args.height, &mut buf)?;
        rendered += 1;

        if !reported_source {
            let get = |k: &str| player.property(k).unwrap_or_else(|| "?".into());
            println!(
                "decoded: {}x{} {} @ {} fps  (first frame after {:?})",
                get("width"),
                get("height"),
                get("video-codec"),
                get("container-fps"),
                started.elapsed()
            );
            println!();
            reported_source = true;
        }

        if rendered <= args.skip {
            continue;
        }

        let stats = analyse(&buf);
        let path = args.out.join(format!("frame_{saved:03}.png"));
        image::save_buffer(
            &path,
            &bgra_to_rgba(&buf),
            args.width,
            args.height,
            image::ColorType::Rgba8,
        )?;

        println!(
            "{}  mean B/G/R {:5.1}/{:5.1}/{:5.1}   alpha min {:3}   non-black {:5.1}%   distinct {:4}",
            path.display(),
            stats.mean_b,
            stats.mean_g,
            stats.mean_r,
            stats.min_alpha,
            stats.nonblack_pct,
            stats.distinct_sample,
        );

        if stats.min_alpha != 0xFF {
            eprintln!("  WARNING alpha is not fully opaque - the bgr0 padding byte leaked through");
        }
        if stats.nonblack_pct < 0.5 {
            eprintln!("  WARNING frame is essentially black");
        }
        if stats.distinct_sample <= 1 {
            eprintln!("  WARNING frame is a flat fill, not a decoded image");
        }

        saved += 1;
    }

    println!("\nrendered {rendered} frame(s), saved {saved} PNG(s) to {}", args.out.display());
    Ok(())
}

/// Measure what one `render_bgra` call actually costs at this size.
///
/// The number that matters is render time against the frame budget: at 60 fps
/// there are 16.7 ms per frame, and everything else - decode, UI, GPU upload -
/// has to fit in there too.
fn run_bench(
    player: &Player,
    args: &Args,
    buf: &mut [u8],
    duration: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    // Measure CPU cost, not mpv's deliberate wait for the frame's display time.
    player.set_block_for_target_time(false);

    // Let playback settle so we measure steady state, not HLS startup.
    let warmup = Instant::now() + Duration::from_secs(2);
    while Instant::now() < warmup {
        if player.wait_for_frame(Duration::from_millis(250)) {
            player.render_bgra(args.width, args.height, buf)?;
        }
    }

    let mut samples: Vec<Duration> = Vec::new();
    let mut missed = 0usize;
    let deadline = Instant::now() + duration;
    let wall = Instant::now();

    while Instant::now() < deadline {
        if !player.wait_for_frame(Duration::from_millis(250)) {
            missed += 1;
            continue;
        }
        let t0 = Instant::now();
        player.render_bgra(args.width, args.height, buf)?;
        samples.push(t0.elapsed());
    }

    if samples.is_empty() {
        eprintln!("no frames rendered during the benchmark window");
        std::process::exit(1);
    }

    let elapsed = wall.elapsed();
    samples.sort_unstable();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let total: Duration = samples.iter().sum();
    let mean = ms(total) / samples.len() as f64;
    let p50 = ms(samples[samples.len() / 2]);
    let p99 = ms(samples[samples.len() * 99 / 100]);
    let worst = ms(*samples.last().unwrap());
    let bytes = args.width as f64 * args.height as f64 * 4.0;
    let fps = samples.len() as f64 / elapsed.as_secs_f64();

    println!("benchmark  {}x{}  over {:.1}s", args.width, args.height, elapsed.as_secs_f64());
    println!();
    println!("  frames rendered   {}", samples.len());
    println!("  delivered fps     {fps:.1}");
    println!("  render mean       {mean:.2} ms");
    println!("  render p50 / p99  {p50:.2} ms / {p99:.2} ms");
    println!("  render worst      {worst:.2} ms");
    println!("  frame size        {:.2} MB", bytes / 1_048_576.0);
    println!("  upload rate       {:.0} MB/s at {fps:.0} fps", bytes * fps / 1_048_576.0);
    println!("  waits with no frame {missed}");
    println!();
    println!("  budget at 60 fps  {:.1}% of 16.7 ms used by render", mean / 16.667 * 100.0);
    Ok(())
}
