use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use collider_core::{Client, Downloader, Event, Options, Quality, StreamInfo};
use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressStyle};
use tokio_util::sync::CancellationToken;

/// High-performance, resumable HLS stream downloader and live recorder.
#[derive(Parser)]
#[command(name = "collider", version, about, long_about = None)]
struct Cli {
    /// Increase log verbosity (-v, -vv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download a VOD stream or record a live stream
    Get(GetArgs),
    /// List the variants and audio renditions a stream offers (HLS or DASH)
    Info(InfoArgs),
}

#[derive(Args)]
struct NetArgs {
    /// URL of a master or media .m3u8 playlist
    url: String,
    /// Extra HTTP header, e.g. -H "Referer: https://example.com" (repeatable)
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
    /// Retries per request on network errors, 429 and 5xx
    #[arg(long, default_value_t = 5)]
    retries: u32,
    /// Give up on a request after this many seconds without receiving data
    #[arg(long, default_value_t = 30)]
    timeout: u64,
}

#[derive(Args)]
struct InfoArgs {
    #[command(flatten)]
    net: NetArgs,
    /// Print machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct GetArgs {
    #[command(flatten)]
    net: NetArgs,
    /// Output file; the container is chosen from the extension
    #[arg(short, long, default_value = "output.mp4")]
    output: PathBuf,
    /// Overwrite the output file if it already exists
    #[arg(short, long)]
    force: bool,
    /// Parallel segment downloads per track
    #[arg(short = 'j', long, default_value_t = 8)]
    concurrency: usize,
    /// best, worst, or a maximum height such as 720 or 720p
    #[arg(short, long, default_value = "best")]
    quality: String,
    /// Preferred audio language or rendition name (e.g. en, sv)
    #[arg(long)]
    audio_lang: Option<String>,
    /// Stop after this many seconds of media (handy for live streams)
    #[arg(long)]
    max_duration: Option<u64>,
    /// Skip the ffmpeg remux and keep raw stream files
    #[arg(long)]
    no_remux: bool,
    /// Keep the per-segment work directory after success
    #[arg(long)]
    keep_parts: bool,
    /// Watch mode: poll every SECS seconds until the stream is live, then record it
    #[arg(long, value_name = "SECS", num_args = 0..=1, default_missing_value = "30")]
    wait: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("collider_core={level}").into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match cli.cmd {
        Cmd::Info(args) => info(args).await,
        Cmd::Get(args) => get(args).await,
    }
}

fn client(net: &NetArgs) -> Result<Client> {
    let headers = net
        .headers
        .iter()
        .map(|h| match h.split_once(':') {
            Some((k, v)) => Ok((k.trim().to_string(), v.trim().to_string())),
            None => bail!("header must look like 'Name: value', got '{h}'"),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Client::new(
        &headers,
        net.retries,
        Duration::from_secs(net.timeout),
    )?)
}

async fn info(args: InfoArgs) -> Result<()> {
    let client = client(&args.net)?;
    let dl = Downloader::new(
        client,
        Options::default(),
        Arc::new(|_| {}),
        CancellationToken::new(),
    );
    let info = dl.info(&args.net.url).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }
    print_info(&info);
    Ok(())
}

fn print_info(info: &StreamInfo) {
    println!(
        "{}{}{}",
        info.protocol,
        if info.live { ", LIVE" } else { "" },
        info.duration
            .map(|d| format!(", {}", fmt_secs(d)))
            .unwrap_or_default()
    );
    if let Some(n) = info.segments {
        println!("  segments:   {n}");
        println!(
            "  encryption: {}",
            if info.encrypted {
                "AES-128 (clear key)"
            } else {
                "none"
            }
        );
    }
    if info.drm {
        println!("  DRM:        yes (ContentProtection present; collider will refuse to download)");
    }
    if !info.variants.is_empty() {
        let mut variants = info.variants.clone();
        variants.sort_by_key(|v| {
            std::cmp::Reverse((v.resolution.map(|r| r.1).unwrap_or(0), v.bandwidth))
        });
        println!(
            "\n{:<18} {:<11} {:>9} {:>6}  {:<26} AUDIO",
            "ID", "RESOLUTION", "BANDWIDTH", "FPS", "CODECS"
        );
        for v in &variants {
            println!(
                "{:<18} {:<11} {:>9} {:>6}  {:<26} {}",
                trunc(&v.id, 18),
                v.resolution
                    .map(|(w, h)| format!("{w}x{h}"))
                    .unwrap_or_else(|| "-".into()),
                format!("{}k", v.bandwidth / 1000),
                v.frame_rate
                    .map(|f| format!("{f:.2}"))
                    .unwrap_or_else(|| "-".into()),
                trunc(v.codecs.as_deref().unwrap_or("-"), 26),
                v.audio_group.as_deref().unwrap_or("-"),
            );
        }
    }
    if !info.audio.is_empty() {
        println!(
            "\n{:<18} {:<12} {:<6} {:>9}  {:<16} DEFAULT",
            "AUDIO", "GROUP", "LANG", "BANDWIDTH", "CODECS"
        );
        for a in &info.audio {
            println!(
                "{:<18} {:<12} {:<6} {:>9}  {:<16} {}",
                trunc(&a.name, 18),
                trunc(&a.group, 12),
                a.language.as_deref().unwrap_or("-"),
                a.bandwidth
                    .map(|b| format!("{}k", b / 1000))
                    .unwrap_or_else(|| "-".into()),
                trunc(a.codecs.as_deref().unwrap_or("-"), 16),
                if a.default { "yes" } else { "" }
            );
        }
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

async fn get(args: GetArgs) -> Result<()> {
    let client = client(&args.net)?;
    let quality: Quality = args
        .quality
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid --quality '{}'", args.quality))?;
    let opts = Options {
        concurrency: args.concurrency,
        quality,
        audio_lang: args.audio_lang,
        max_duration: args.max_duration.map(Duration::from_secs),
        keep_parts: args.keep_parts,
        remux: !args.no_remux,
        wait: args.wait.map(|s| Duration::from_secs(s.max(1))),
        overwrite: args.force,
    };

    let cancel = CancellationToken::new();
    tokio::spawn(handle_interrupts(cancel.clone()));

    let ui = Arc::new(Ui::new());
    let reporter = {
        let ui = ui.clone();
        Arc::new(move |e: Event| ui.handle(e))
    };
    let dl = Downloader::new(client, opts, reporter, cancel);
    let files = dl.download(&args.net.url, &args.output).await;
    ui.finish();
    for f in files? {
        println!("saved {}", f.display());
    }
    Ok(())
}

/// First Ctrl-C: stop gracefully (live captures are saved, VOD can be resumed).
/// Second Ctrl-C: exit immediately. A single signal stream is used so the first
/// signal cannot be observed twice.
async fn handle_interrupts(cancel: CancellationToken) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let Ok(mut sigint) = signal(SignalKind::interrupt()) else {
            return;
        };
        let Ok(mut sigterm) = signal(SignalKind::terminate()) else {
            return;
        };
        tokio::select! { _ = sigint.recv() => {}, _ = sigterm.recv() => {} }
        eprintln!("\ninterrupt: finishing in-flight segments (press Ctrl-C again to force quit)");
        cancel.cancel();
        tokio::select! { _ = sigint.recv() => {}, _ = sigterm.recv() => {} }
        std::process::exit(130);
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\ninterrupt: finishing in-flight segments");
            cancel.cancel();
        }
    }
}

struct TrackBar {
    bar: ProgressBar,
    segments: u64,
    bytes: u64,
    resumed: u64,
    live: bool,
}

struct Ui {
    multi: MultiProgress,
    tracks: Mutex<HashMap<String, TrackBar>>,
}

impl Ui {
    fn new() -> Self {
        Self {
            multi: MultiProgress::new(),
            tracks: Mutex::new(HashMap::new()),
        }
    }

    /// Print a message line above the bars. `MultiProgress::println` drops it when stderr
    /// is not a terminal, and unattended recordings need these lines most.
    fn line(&self, msg: String) {
        if self.multi.is_hidden() {
            eprintln!("{msg}");
        } else {
            self.multi.println(msg).ok();
        }
    }

    fn handle(&self, e: Event) {
        let mut tracks = self.tracks.lock().unwrap();
        match e {
            Event::Status(s) => self.line(format!("» {s}")),
            Event::Detected { protocol } => self.line(format!("» {protocol} stream")),
            Event::Gaps { track, missing } => self.line(format!(
                "! {track}: {missing} segment(s) could not be fetched; the recording has gaps"
            )),
            Event::TrackStarted { track, live, .. } => {
                let bar = self.multi.add(if live {
                    ProgressBar::no_length()
                } else {
                    ProgressBar::new(0)
                });
                let template = if live {
                    "{prefix:>6} {spinner} LIVE {pos} segs  {msg}  {elapsed_precise}"
                } else {
                    "{prefix:>6} [{bar:36.cyan/blue}] {pos}/{len} segs  {msg}  eta {eta}"
                };
                bar.set_style(
                    ProgressStyle::with_template(template)
                        .unwrap()
                        .progress_chars("=> "),
                );
                bar.set_prefix(track.clone());
                bar.enable_steady_tick(Duration::from_millis(200));
                tracks.insert(
                    track,
                    TrackBar {
                        bar,
                        segments: 0,
                        bytes: 0,
                        resumed: 0,
                        live,
                    },
                );
            }
            Event::Discovered { track, total } => {
                if let Some(t) = tracks.get(&track) {
                    if !t.live {
                        t.bar.set_length(total as u64);
                    }
                }
            }
            Event::SegmentDone {
                track,
                bytes,
                resumed,
            } => {
                if let Some(t) = tracks.get_mut(&track) {
                    t.segments += 1;
                    t.bytes += bytes;
                    t.resumed += resumed as u64;
                    t.bar.inc(1);
                    let rate = t.bytes as f64 / t.bar.elapsed().as_secs_f64().max(0.001);
                    let resumed = if t.resumed > 0 {
                        format!("  ({} resumed)", t.resumed)
                    } else {
                        String::new()
                    };
                    t.bar.set_message(format!(
                        "{}  {}/s{resumed}",
                        HumanBytes(t.bytes),
                        HumanBytes(rate as u64)
                    ));
                }
            }
        }
    }

    fn finish(&self) {
        let tracks = self.tracks.lock().unwrap();
        let mut names: Vec<_> = tracks.keys().collect();
        names.sort();
        for name in names {
            let t = &tracks[name];
            t.bar.finish_and_clear();
            eprintln!(
                "{name}: {} segments, {}, {} resumed, {:.1}s",
                t.segments,
                HumanBytes(t.bytes),
                t.resumed,
                t.bar.elapsed().as_secs_f64()
            );
        }
    }
}

fn fmt_secs(s: f64) -> String {
    let s = s.round() as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}
