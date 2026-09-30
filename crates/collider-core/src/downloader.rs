//! The capture engine: parallel segment fetching, resume, live polling and muxing.
//!
//! Every track (video, alternate audio) is downloaded into its own work directory as
//! numbered segment files. A segment file only appears after it is fully written
//! (write to `.tmp`, then rename), so its presence means "done". That makes resuming
//! an interrupted VOD download a matter of rerunning the same command.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::{StreamExt, TryStreamExt};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::crypto::decrypt_aes128;
use crate::error::{Error, Result};
use crate::hls::Quality;
use crate::http::Client;
use crate::model::{InitSection, Protocol, Segment, StreamInfo};
use crate::mux;
use crate::source::{self, PlannedTrack, Selection};

#[derive(Debug, Clone)]
pub struct Options {
    /// Concurrent segment downloads per track.
    pub concurrency: usize,
    pub quality: Quality,
    /// Preferred audio language or rendition name for streams with separate audio.
    pub audio_lang: Option<String>,
    /// Stop after this much media time has been captured (useful for live streams).
    pub max_duration: Option<Duration>,
    /// Keep the per-segment work directory after a successful run.
    pub keep_parts: bool,
    /// Remux the result with ffmpeg. If false (or ffmpeg is missing) raw files are kept.
    pub remux: bool,
    /// Watch mode: keep polling at this interval until the stream exists and has segments.
    pub wait: Option<Duration>,
    /// Replace existing output files instead of refusing to start.
    pub overwrite: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            concurrency: 8,
            quality: Quality::Best,
            audio_lang: None,
            max_duration: None,
            keep_parts: false,
            remux: true,
            wait: None,
            overwrite: false,
        }
    }
}

/// Progress reports. Segment events carry `index`, the segment's position in the track's
/// discovery order (the same order `Discovered::total` counts).
#[derive(Debug, Clone)]
pub enum Event {
    /// The manifest was fetched and recognised.
    Detected {
        protocol: Protocol,
    },
    /// A track was chosen; `description` names the variant or rendition.
    TrackPlanned {
        track: String,
        description: String,
    },
    /// A track's playlist was read. `live` is true when the playlist has no end yet.
    TrackStarted {
        track: String,
        live: bool,
    },
    /// The number of known segments for a track changed.
    Discovered {
        track: String,
        total: usize,
    },
    /// A segment download started.
    SegmentStarted {
        track: String,
        index: usize,
    },
    /// A segment finished. `resumed` means it was already on disk from an earlier run.
    SegmentDone {
        track: String,
        index: usize,
        bytes: u64,
        resumed: bool,
    },
    /// A segment will not be in the output. `gap` means a live segment that could not be
    /// fetched; otherwise the manifest only implied it and the origin does not have it.
    SegmentSkipped {
        track: String,
        index: usize,
        gap: bool,
    },
    /// A live capture ended with gaps (segments that could not be fetched).
    Gaps {
        track: String,
        missing: usize,
    },
    /// Watch mode: the stream is not available yet; the next check is in `retry_in`.
    Waiting {
        reason: String,
        retry_in: Duration,
    },
    /// Every track is captured and ffmpeg is joining them into the output.
    Remuxing,
    Status(String),
}

pub type Reporter = Arc<dyn Fn(Event) + Send + Sync>;

pub struct Downloader {
    client: Client,
    opts: Options,
    report: Reporter,
    cancel: CancellationToken,
}

struct TrackOutput {
    name: String,
    /// One file per timeline, in order (see [`concat_track`]).
    pieces: Vec<PathBuf>,
}

/// Consecutive failed playlist refreshes after which a live capture is saved and ended.
/// A playlist that answers 404/410 (usually deleted at stream end) gets fewer tries.
const MAX_REFRESH_FAILURES: u32 = 5;
const MAX_GONE_REFRESHES: u32 = 2;

impl Downloader {
    pub fn new(client: Client, opts: Options, report: Reporter, cancel: CancellationToken) -> Self {
        Self {
            client,
            opts: Options {
                concurrency: opts.concurrency.max(1),
                ..opts
            },
            report,
            cancel,
        }
    }

    /// What `url` offers. For an HLS master this also reads one media playlist.
    pub async fn info(&self, url: &str) -> Result<StreamInfo> {
        let probe = source::probe(&self.client, &Url::parse(url)?).await?;
        source::inspect(&self.client, &probe).await
    }

    /// Download `url` (HLS playlist or DASH MPD) to `output`. Returns the files written.
    pub async fn download(&self, url: &str, output: &Path) -> Result<Vec<PathBuf>> {
        let url = Url::parse(url)?;
        // Raw outputs (no remux) are checked in `finish`, once their extension is known.
        if !self.opts.overwrite
            && self.opts.remux
            && fs::try_exists(output).await.unwrap_or(false)
            && mux::ffmpeg_available().await
        {
            return Err(exists(output));
        }
        let tracks = loop {
            match self.plan(&url).await {
                Ok(tracks) => break tracks,
                Err(e) if self.opts.wait.is_some() && waitable(&e) => {
                    let interval = self.opts.wait.unwrap_or(Duration::from_secs(30));
                    (self.report)(Event::Waiting {
                        reason: e.to_string(),
                        retry_in: interval,
                    });
                    tokio::select! {
                        _ = tokio::time::sleep(interval) => {}
                        _ = self.cancel.cancelled() => return Err(Error::Cancelled),
                    }
                }
                Err(e) => return Err(e),
            }
        };

        let work = work_dir(output);
        for t in &tracks {
            claim_dir(&work.join(&t.name), &t.identity).await?;
        }
        let outputs = futures::future::try_join_all(tracks.into_iter().map(|t| {
            let dir = work.join(&t.name);
            self.run_track(t, dir)
        }))
        .await?;

        let files = self.finish(&outputs, output).await?;
        if !self.opts.keep_parts {
            fs::remove_dir_all(&work).await.ok();
        }
        Ok(files)
    }

    /// Probe, select tracks and, in watch mode, require that every track already has segments.
    async fn plan(&self, url: &Url) -> Result<Vec<PlannedTrack>> {
        let probe = source::probe(&self.client, url).await?;
        (self.report)(Event::Detected {
            protocol: probe.protocol,
        });
        let sel = Selection {
            quality: &self.opts.quality,
            audio_lang: self.opts.audio_lang.as_deref(),
        };
        let mut tracks = source::plan(&self.client, &probe, &sel)?;
        if self.opts.wait.is_some() {
            for t in tracks.iter_mut() {
                if t.source.refresh().await?.segments.is_empty() {
                    return Err(Error::NotLive(format!("{} has no segments", t.name)));
                }
            }
        }
        for t in &tracks {
            (self.report)(Event::TrackPlanned {
                track: t.name.clone(),
                description: t.description.clone(),
            });
        }
        Ok(tracks)
    }

    async fn run_track(&self, mut track: PlannedTrack, dir: PathBuf) -> Result<TrackOutput> {
        fs::create_dir_all(&dir).await?;
        let name = track.name.clone();
        let mut order: Vec<Segment> = Vec::new();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut keys: HashMap<Url, [u8; 16]> = HashMap::new();
        let mut inits: HashSet<InitSection> = HashSet::new();
        let mut captured = 0.0f64;
        let mut first = true;
        let mut live = false;
        let mut failures = 0u32;
        let mut interval = Duration::from_secs(2);

        loop {
            let refreshed = tokio::select! {
                r = track.source.refresh() => r,
                _ = self.cancel.cancelled(), if live => {
                    (self.report)(Event::Status(format!("{name}: stopping, saving what was captured")));
                    break;
                }
            };
            let snap = match refreshed {
                Ok(snap) => {
                    failures = 0;
                    interval = snap.refresh_after;
                    snap
                }
                // A running live capture is never thrown away because its playlist went
                // missing or the network failed: retry a few times, then save what we have.
                Err(e) if live => {
                    failures += 1;
                    let gone = matches!(
                        e,
                        Error::Status {
                            status: 404 | 410,
                            ..
                        }
                    );
                    let limit = if gone {
                        MAX_GONE_REFRESHES
                    } else {
                        MAX_REFRESH_FAILURES
                    };
                    if failures >= limit {
                        (self.report)(Event::Status(format!(
                            "{name}: playlist unavailable ({e}); saving what was captured"
                        )));
                        break;
                    }
                    (self.report)(Event::Status(format!(
                        "{name}: playlist refresh failed ({e}); retrying"
                    )));
                    tokio::select! {
                        _ = tokio::time::sleep(clamp_refresh(interval)) => continue,
                        _ = self.cancel.cancelled() => {
                            (self.report)(Event::Status(format!("{name}: stopping, saving what was captured")));
                            break;
                        }
                    }
                }
                Err(e) => return Err(e),
            };
            if first {
                live = !snap.ended;
                (self.report)(Event::TrackStarted {
                    track: name.clone(),
                    live,
                });
                first = false;
            }

            let mut fresh = Vec::new();
            let mut limit_hit = false;
            for s in snap.segments.into_iter().filter(|s| seen.insert(s.seq)) {
                if let Some(max) = self.opts.max_duration {
                    if captured >= max.as_secs_f64() {
                        limit_hit = true;
                        break;
                    }
                }
                captured += s.duration;
                fresh.push(s);
            }

            if !fresh.is_empty() {
                self.resolve_keys(&fresh, &mut keys).await?;
                for init in fresh.iter().filter_map(|s| s.init.clone()) {
                    if inits.insert(init.clone()) {
                        let key = init.key.as_ref().map(|k| (keys[&k.url], k.iv));
                        fetch_init(&self.client, &init, key, &dir).await?;
                    }
                }
                let base = order.len();
                order.extend(fresh.iter().cloned());
                (self.report)(Event::Discovered {
                    track: name.clone(),
                    total: order.len(),
                });
                self.fetch_batch(&name, &fresh, base, &keys, &dir, live)
                    .await?;
            }

            if self.cancel.is_cancelled() {
                if live {
                    (self.report)(Event::Status(format!(
                        "{name}: stopping, saving what was captured"
                    )));
                    break;
                }
                return Err(Error::Cancelled);
            }
            if snap.ended || limit_hit {
                break;
            }

            // RFC 8216 §6.3.4 / DASH minimumUpdatePeriod: wait a full interval after changes,
            // half of it when nothing new appeared.
            let wait = if fresh.is_empty() {
                snap.refresh_after / 2
            } else {
                snap.refresh_after
            };
            tokio::select! {
                _ = tokio::time::sleep(clamp_refresh(wait)) => {}
                _ = self.cancel.cancelled() => {
                    (self.report)(Event::Status(format!("{name}: stopping, saving what was captured")));
                    break;
                }
            }
        }

        let (pieces, missing) = concat_track(&name, &order, &dir, live).await?;
        if missing > 0 {
            (self.report)(Event::Gaps {
                track: name.clone(),
                missing,
            });
        }
        Ok(TrackOutput { name, pieces })
    }

    async fn resolve_keys(
        &self,
        segs: &[Segment],
        keys: &mut HashMap<Url, [u8; 16]>,
    ) -> Result<()> {
        let wanted: HashSet<&Url> = segs
            .iter()
            .flat_map(|s| [s.key.as_ref(), s.init.as_ref().and_then(|i| i.key.as_ref())])
            .flatten()
            .map(|k| &k.url)
            .collect();
        for url in wanted {
            if keys.contains_key(url) {
                continue;
            }
            let bytes = self.client.get_bytes(url, None).await?;
            let key: [u8; 16] = bytes.as_ref().try_into().map_err(|_| {
                Error::Decrypt(format!(
                    "key at {url} is {} bytes, expected 16",
                    bytes.len()
                ))
            })?;
            keys.insert(url.clone(), key);
        }
        Ok(())
    }

    /// Fetch `segs`, whose first element is segment `base` of the track.
    async fn fetch_batch(
        &self,
        track: &str,
        segs: &[Segment],
        base: usize,
        keys: &HashMap<Url, [u8; 16]>,
        dir: &Path,
        live: bool,
    ) -> Result<()> {
        // The futures are built by a plain iterator and a named `async fn`: a closure
        // returning an async block inside `Stream::map` makes the whole download future
        // impossible to prove `Send` (rustc #102211), and the GUI spawns it on a runtime.
        let pending: Vec<_> = segs
            .iter()
            .enumerate()
            .map(|(i, seg)| self.fetch_one(track, seg, base + i, keys, dir, live))
            .collect();
        // On cancel no new downloads start, but the ones in flight finish: a hole before
        // already-completed later segments would otherwise become a gap in a live capture.
        let stream = futures::stream::iter(pending)
            .take_until(self.cancel.cancelled())
            .buffer_unordered(self.opts.concurrency);
        futures::pin_mut!(stream);

        while let Some((index, fetched)) = stream.try_next().await? {
            let track = track.to_string();
            (self.report)(match fetched {
                Fetched::Done { bytes, resumed } => Event::SegmentDone {
                    track,
                    index,
                    bytes,
                    resumed,
                },
                Fetched::Gap => Event::SegmentSkipped {
                    track,
                    index,
                    gap: true,
                },
                Fetched::Absent => Event::SegmentSkipped {
                    track,
                    index,
                    gap: false,
                },
            });
        }
        Ok(())
    }

    async fn fetch_one(
        &self,
        track: &str,
        seg: &Segment,
        index: usize,
        keys: &HashMap<Url, [u8; 16]>,
        dir: &Path,
        live: bool,
    ) -> Result<(usize, Fetched)> {
        (self.report)(Event::SegmentStarted {
            track: track.to_string(),
            index,
        });
        let key = seg.key.as_ref().map(|k| (keys[&k.url], k.iv));
        let fetched = fetch_segment(&self.client, seg, key, dir, live).await?;
        Ok((index, fetched))
    }

    async fn finish(&self, outputs: &[TrackOutput], output: &Path) -> Result<Vec<PathBuf>> {
        let ffmpeg = self.opts.remux && mux::ffmpeg_available().await;
        let mut inputs = Vec::with_capacity(outputs.len());
        for o in outputs {
            inputs.push(self.join_pieces(o, ffmpeg).await?);
        }
        if ffmpeg {
            (self.report)(Event::Remuxing);
            if let Err(e) = mux::remux(&inputs, output).await {
                // Do not leave a broken file behind; the raw capture stays in the work dir.
                fs::remove_file(output).await.ok();
                return Err(Error::Mux(format!("{e}; the raw capture is kept at {}", {
                    let kept: Vec<String> =
                        inputs.iter().map(|p| p.display().to_string()).collect();
                    kept.join(", ")
                })));
            }
            return Ok(vec![output.to_path_buf()]);
        }
        if self.opts.remux {
            (self.report)(Event::Status(
                "ffmpeg not found; keeping raw stream files".into(),
            ));
        }
        let mut dests = Vec::new();
        for (o, file) in outputs.iter().zip(&inputs) {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("ts");
            let dest = if outputs.len() == 1 {
                output.with_extension(ext)
            } else {
                output.with_extension(format!("{}.{ext}", o.name))
            };
            if !self.opts.overwrite && fs::try_exists(&dest).await.unwrap_or(false) {
                return Err(exists(&dest));
            }
            dests.push(dest);
        }
        for (file, dest) in inputs.iter().zip(&dests) {
            move_file(file, dest).await?;
        }
        Ok(dests)
    }

    /// Turn a track's timeline pieces into one file: with ffmpeg, re-timed by the concat
    /// demuxer; without it, appended raw (timestamps then restart at each boundary).
    async fn join_pieces(&self, o: &TrackOutput, ffmpeg: bool) -> Result<PathBuf> {
        let (first, rest) = o
            .pieces
            .split_first()
            .expect("concat_track returns at least one piece");
        if rest.is_empty() {
            return Ok(first.clone());
        }
        let ext = first.extension().and_then(|e| e.to_str()).unwrap_or("ts");
        let joined = first.with_file_name(format!("{}.{ext}", o.name));
        let n = o.pieces.len();
        if ffmpeg {
            (self.report)(Event::Status(format!(
                "{}: joining {n} timelines (DASH periods or HLS discontinuities)",
                o.name
            )));
            mux::concat(&o.pieces, &joined).await?;
        } else {
            (self.report)(Event::Status(format!(
                "{}: {n} timelines appended raw; timestamps restart at each boundary",
                o.name
            )));
            let mut f = fs::File::create(&joined).await?;
            for p in &o.pieces {
                f.write_all(&fs::read(p).await?).await?;
            }
            f.flush().await?;
        }
        Ok(joined)
    }
}

fn exists(path: &Path) -> Error {
    Error::Conflict(format!(
        "{} already exists; use --force to overwrite it",
        path.display()
    ))
}

fn clamp_refresh(wait: Duration) -> Duration {
    wait.clamp(Duration::from_millis(500), Duration::from_secs(30))
}

/// Record which track `dir` holds segments of, or refuse to resume into a directory that
/// was filled by another stream, variant or audio rendition: its segments would be spliced
/// into this recording.
async fn claim_dir(dir: &Path, identity: &str) -> Result<()> {
    fs::create_dir_all(dir).await?;
    let file = dir.join("source");
    match fs::read_to_string(&file).await {
        Ok(found) if found.trim() == identity => Ok(()),
        Ok(found) => Err(Error::Conflict(format!(
            "{} holds segments of another download ({}); delete it or choose another -o",
            dir.parent().unwrap_or(dir).display(),
            found.trim()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write_atomic(&file, identity.as_bytes()).await
        }
        Err(e) => Err(e.into()),
    }
}

/// Errors that mean "not live yet" rather than "broken": worth retrying in watch mode.
fn waitable(e: &Error) -> bool {
    matches!(
        e,
        Error::NotLive(_) | Error::Http(_) | Error::Status { .. } | Error::Parse(_)
    )
}

fn work_dir(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    output.with_file_name(format!("{name}.parts"))
}

fn segment_path(dir: &Path, seq: u64) -> PathBuf {
    dir.join(format!("{seq:016x}.seg"))
}

async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp).await?;
    f.write_all(data).await?;
    f.flush().await?;
    drop(f);
    fs::rename(&tmp, path).await?;
    Ok(())
}

/// What became of one segment.
#[derive(Debug, PartialEq, Eq)]
enum Fetched {
    /// On disk; `resumed` means it already was before this run.
    Done { bytes: u64, resumed: bool },
    /// A live segment that was never published or could not be fetched.
    Gap,
    /// An optional segment that the origin does not have.
    Absent,
}

async fn fetch_segment(
    client: &Client,
    seg: &Segment,
    key: Option<([u8; 16], [u8; 16])>,
    dir: &Path,
    live: bool,
) -> Result<Fetched> {
    let path = segment_path(dir, seg.seq);
    if let Ok(meta) = fs::metadata(&path).await {
        return Ok(Fetched::Done {
            bytes: meta.len(),
            resumed: true,
        });
    }
    // Live segments can be announced slightly before they are available: retry a 404 briefly.
    let mut attempts = 0;
    let data = loop {
        match client.get_bytes(&seg.url, seg.byte_range).await {
            Ok(d) => break d,
            Err(Error::Status { status: 404, .. }) if live && attempts < 4 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(500 * attempts)).await;
            }
            Err(Error::Status {
                status: 404 | 410 | 416,
                url,
            }) if seg.optional => {
                tracing::debug!(%url, "optional trailing segment does not exist");
                return Ok(Fetched::Absent);
            }
            Err(Error::Status {
                status: 404 | 410,
                url,
            }) if live => {
                tracing::warn!(%url, "live segment is gone, skipping");
                return Ok(Fetched::Gap);
            }
            // The client already retried; one lost live segment is a gap, not the end.
            Err(e @ (Error::Http(_) | Error::Status { .. })) if live => {
                tracing::warn!(url = %seg.url, error = %e, "live segment failed, skipping");
                return Ok(Fetched::Gap);
            }
            Err(e) => return Err(e),
        }
    };
    let data = match key {
        Some((k, iv)) => decrypt_aes128(&data, &k, &iv)?,
        None => data.to_vec(),
    };
    write_atomic(&path, &data).await?;
    Ok(Fetched::Done {
        bytes: data.len() as u64,
        resumed: false,
    })
}

async fn fetch_init(
    client: &Client,
    init: &InitSection,
    key: Option<([u8; 16], [u8; 16])>,
    dir: &Path,
) -> Result<()> {
    let path = dir.join(init.file_name());
    if fs::metadata(&path).await.is_ok() {
        return Ok(());
    }
    let data = client.get_bytes(&init.url, init.byte_range).await?;
    let data = match key {
        // The playlist cannot tell a KEY-then-MAP (encrypted init) from a MAP-then-KEY
        // (plaintext init) apart, so only decrypt what does not already look like media.
        Some((k, iv)) if !looks_like_plaintext_init(&data) => decrypt_aes128(&data, &k, &iv)
            .map_err(|e| Error::Decrypt(format!("init section {}: {e}", init.url)))?,
        _ => data.to_vec(),
    };
    write_atomic(&path, &data).await
}

/// True when `data` starts like an ISO-BMFF file or is a run of MPEG-TS packets, or cannot
/// be AES-CBC ciphertext at all (not a whole number of blocks).
fn looks_like_plaintext_init(data: &[u8]) -> bool {
    const BOXES: [&[u8; 4]; 11] = [
        b"ftyp", b"styp", b"moov", b"sidx", b"free", b"skip", b"moof", b"mdat", b"emsg", b"prft",
        b"uuid",
    ];
    let isobmff = data.len() >= 8 && BOXES.iter().any(|b| &data[4..8] == *b);
    let ts = !data.is_empty() && data.len() % 188 == 0 && data.chunks(188).all(|p| p[0] == 0x47);
    isobmff || ts || data.len() % 16 != 0
}

fn container_ext(order: &[Segment]) -> &'static str {
    let ext = order
        .first()
        .and_then(|s| s.url.path().rsplit('.').next())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("webm" | "mkv") => "webm",
        Some("aac") => "aac",
        Some("mp3") => "mp3",
        Some("ac3") => "ac3",
        Some("m4s" | "mp4" | "m4a" | "m4v") => "mp4",
        Some("ts") => "ts",
        _ if order.iter().any(|s| s.init.is_some()) => "mp4",
        _ => "ts",
    }
}

/// Join init sections + segments in playlist order, one file per timeline: a new piece
/// starts wherever timestamps may restart (a new DASH period, an HLS discontinuity or a
/// new init section), because raw-appending those gives non-monotonic timestamps.
/// Returns the pieces (at least one) and the number of segments missing (live gaps).
async fn concat_track(
    name: &str,
    order: &[Segment],
    dir: &Path,
    allow_gaps: bool,
) -> Result<(Vec<PathBuf>, usize)> {
    let ext = container_ext(order);
    let piece_path = |i: usize| dir.with_file_name(format!("{name}.{i:03}.{ext}"));
    let mut pieces = Vec::new();
    let mut f: Option<fs::File> = None;
    let mut missing = 0usize;
    let mut boundary = false;
    for (i, seg) in order.iter().enumerate() {
        // A boundary carried by a missing segment still applies to the next one written.
        boundary |= i > 0 && {
            let prev = &order[i - 1];
            seg.discontinuity || seg.period() != prev.period() || seg.init != prev.init
        };
        let data = match fs::read(segment_path(dir, seg.seq)).await {
            Ok(data) => data,
            // An optional segment that the origin does not have is not a gap.
            Err(e) if seg.optional && e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) if allow_gaps && e.kind() == std::io::ErrorKind::NotFound => {
                missing += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let file = match f.as_mut() {
            Some(file) if !boundary => file,
            _ => {
                if let Some(mut done) = f.take() {
                    done.flush().await?;
                }
                pieces.push(piece_path(pieces.len()));
                let mut file = fs::File::create(&pieces[pieces.len() - 1]).await?;
                if let Some(init) = &seg.init {
                    file.write_all(&fs::read(dir.join(init.file_name())).await?)
                        .await?;
                }
                boundary = false;
                f.insert(file)
            }
        };
        file.write_all(&data).await?;
    }
    match f {
        Some(mut file) => file.flush().await?,
        None => {
            pieces.push(piece_path(0));
            fs::File::create(&pieces[0]).await?;
        }
    }
    if let [only] = pieces.as_mut_slice() {
        let single = dir.with_file_name(format!("{name}.{ext}"));
        fs::rename(&*only, &single).await?;
        *only = single;
    }
    Ok((pieces, missing))
}

async fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).await.is_err() {
        fs::copy(from, to).await?;
        fs::remove_file(from).await?;
    }
    Ok(())
}

/// The download future must stay `Send`: the GUI runs it on a multi-threaded runtime.
#[allow(dead_code)]
fn assert_download_is_send(d: &Downloader) {
    fn send<T: Send>(_: T) {}
    send(d.download("", Path::new("")));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(seq: u64, path: &str) -> Segment {
        Segment {
            seq,
            url: Url::parse(&format!("https://x.test/{path}")).unwrap(),
            duration: 4.0,
            byte_range: None,
            key: None,
            init: None,
            optional: false,
            discontinuity: false,
        }
    }

    #[test]
    fn work_dir_is_next_to_output() {
        assert_eq!(
            work_dir(Path::new("/tmp/out.mp4")),
            PathBuf::from("/tmp/out.mp4.parts")
        );
    }

    #[test]
    fn extension_detection() {
        assert_eq!(container_ext(&[seg(1, "a.ts?token=1")]), "ts");
        assert_eq!(container_ext(&[seg(1, "a.aac")]), "aac");
        assert_eq!(container_ext(&[seg(1, "chunk.webm")]), "webm");
        let mut s = seg(1, "a.m4s");
        s.init = Some(InitSection {
            url: s.url.clone(),
            byte_range: None,
            key: None,
        });
        assert_eq!(container_ext(&[s]), "mp4");
    }

    #[tokio::test]
    async fn encrypted_init_sections_are_decrypted() {
        use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        let key = [9u8; 16];
        let iv = crate::hls::parse_iv("0x01").unwrap();
        let plain = b"\0\0\0\x14ftypisom\0\0\0\0isommoov".to_vec();
        let enc = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        let (root, _log) = crate::source::tests::serve(vec![
            ("/enc.mp4", 200, "", enc),
            ("/plain.mp4", 200, "", plain.clone()),
        ]);
        let client = Client::new(&[], 0, Duration::from_secs(5)).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        for name in ["enc.mp4", "plain.mp4"] {
            let init = InitSection {
                url: root.join(name).unwrap(),
                byte_range: None,
                key: Some(crate::model::Key {
                    url: root.join("key.bin").unwrap(),
                    iv,
                }),
            };
            fetch_init(&client, &init, Some((key, iv)), tmp.path())
                .await
                .unwrap();
            let got = fs::read(tmp.path().join(init.file_name())).await.unwrap();
            assert_eq!(got, plain, "{name}");
        }
        assert!(looks_like_plaintext_init(&[0x47; 188 * 2]));
        assert!(!looks_like_plaintext_init(&[0x11; 32]));
    }

    #[tokio::test]
    async fn missing_optional_segment_ends_the_period() {
        let (root, _log) = crate::source::tests::serve(vec![("/1.m4s", 200, "", b"A".to_vec())]);
        let client = Client::new(&[], 0, Duration::from_secs(5)).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut a = seg(1, "1.m4s");
        a.url = root.join("1.m4s").unwrap();
        let mut b = seg(2, "2.m4s");
        b.url = root.join("2.m4s").unwrap();
        // A 404 on a required VOD segment is fatal ...
        assert!(fetch_segment(&client, &b, None, dir, false).await.is_err());
        // ... but on an optional one it just means the period ended earlier.
        b.optional = true;
        assert_eq!(
            fetch_segment(&client, &a, None, dir, false).await.unwrap(),
            Fetched::Done {
                bytes: 1,
                resumed: false
            }
        );
        assert_eq!(
            fetch_segment(&client, &b, None, dir, false).await.unwrap(),
            Fetched::Absent
        );
        let (pieces, missing) = concat_track("main", &[a, b], dir, false).await.unwrap();
        assert_eq!(missing, 0);
        assert_eq!(fs::read(&pieces[0]).await.unwrap(), b"A");
    }

    #[tokio::test]
    async fn concat_orders_gaps_and_init_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("main");
        fs::create_dir_all(&dir).await.unwrap();
        let init_a = InitSection {
            url: Url::parse("https://x.test/ia").unwrap(),
            byte_range: None,
            key: None,
        };
        let init_b = InitSection {
            url: Url::parse("https://x.test/ib").unwrap(),
            byte_range: None,
            key: None,
        };
        write_atomic(&dir.join(init_a.file_name()), b"[IA]")
            .await
            .unwrap();
        write_atomic(&dir.join(init_b.file_name()), b"[IB]")
            .await
            .unwrap();
        write_atomic(&segment_path(&dir, 2), b"B").await.unwrap();
        write_atomic(&segment_path(&dir, 1), b"A").await.unwrap();
        write_atomic(&segment_path(&dir, 4), b"D").await.unwrap();
        let mut order = [
            seg(1, "a.m4s"),
            seg(2, "b.m4s"),
            seg(3, "c.m4s"),
            seg(4, "d.m4s"),
        ];
        order[0].init = Some(init_a.clone());
        order[1].init = Some(init_a.clone());
        order[2].init = Some(init_b.clone());
        order[3].init = Some(init_b.clone());
        assert!(concat_track("main", &order, &dir, false).await.is_err());
        let (pieces, missing) = concat_track("main", &order, &dir, true).await.unwrap();
        assert_eq!(missing, 1);
        // A new init section starts a new timeline piece, even when its first segment is
        // missing.
        assert_eq!(pieces.len(), 2);
        assert_eq!(fs::read(&pieces[0]).await.unwrap(), b"[IA]AB");
        assert_eq!(fs::read(&pieces[1]).await.unwrap(), b"[IB]D");
    }

    #[tokio::test]
    async fn periods_and_discontinuities_start_new_pieces() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("main");
        fs::create_dir_all(&dir).await.unwrap();
        let p1 = 1 << crate::model::PERIOD_SHIFT;
        let mut order = vec![seg(1, "a.ts"), seg(2, "b.ts"), seg(p1, "c.ts")];
        order.push(seg(p1 + 1, "d.ts"));
        order.push(seg(p1 + 2, "e.ts"));
        order[4].discontinuity = true;
        for (s, body) in order.iter().zip(["A", "B", "C", "D", "E"]) {
            write_atomic(&segment_path(&dir, s.seq), body.as_bytes())
                .await
                .unwrap();
        }
        let (pieces, _) = concat_track("main", &order, &dir, false).await.unwrap();
        let mut got = Vec::new();
        for p in &pieces {
            got.push(String::from_utf8(fs::read(p).await.unwrap()).unwrap());
        }
        assert_eq!(got, ["AB", "CD", "E"]);
        // One timeline keeps the plain track file name.
        let (pieces, _) = concat_track("main", &order[..2], &dir, false)
            .await
            .unwrap();
        assert_eq!(pieces, [tmp.path().join("main.ts")]);
    }

    #[tokio::test]
    async fn parts_of_another_download_are_not_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("out.mp4.parts").join("video");
        claim_dir(&dir, "https://a.test/v.m3u8").await.unwrap();
        claim_dir(&dir, "https://a.test/v.m3u8").await.unwrap();
        let err = claim_dir(&dir, "https://a.test/other.m3u8")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err}");
    }

    /// A live capture survives a failed segment (a gap) and ends, saved, when its playlist
    /// disappears, instead of failing and discarding what was recorded.
    #[tokio::test]
    async fn live_capture_is_saved_when_the_playlist_disappears() {
        let playlist = b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n\
#EXTINF:1,\na.ts\n#EXTINF:1,\nb.ts\n#EXTINF:1,\nc.ts\n"
            .to_vec();
        let (root, _log) = crate::source::tests::serve(vec![
            ("/live.m3u8", 200, "", playlist),
            ("/live.m3u8", 404, "", Vec::new()),
            ("/a.ts", 200, "", b"A".to_vec()),
            ("/b.ts", 500, "", Vec::new()),
            ("/c.ts", 200, "", b"C".to_vec()),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("rec.ts");
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        let dl = Downloader::new(
            Client::new(&[], 0, Duration::from_secs(5)).unwrap(),
            Options {
                remux: false,
                ..Options::default()
            },
            Arc::new(move |e| sink.lock().unwrap().push(e)),
            CancellationToken::new(),
        );
        let files = dl
            .download(root.join("live.m3u8").unwrap().as_str(), &output)
            .await
            .unwrap();
        assert_eq!(files, std::slice::from_ref(&output));
        assert_eq!(fs::read(&output).await.unwrap(), b"AC");
        let events = events.lock().unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Gaps { missing: 1, .. })));
        // Segment events are indexed by discovery order: a.ts, b.ts (the gap), c.ts.
        let mut done: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                Event::SegmentDone { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        done.sort();
        assert_eq!(done, [0, 2]);
        assert!(events.iter().any(|e| matches!(
            e,
            Event::SegmentSkipped {
                index: 1,
                gap: true,
                ..
            }
        )));
        let started = events
            .iter()
            .filter(|e| matches!(e, Event::SegmentStarted { .. }))
            .count();
        assert_eq!(started, 3);
    }

    #[tokio::test]
    async fn existing_output_is_kept_without_overwrite() {
        let playlist = b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST\n";
        let (root, _log) = crate::source::tests::serve(vec![
            ("/vod.m3u8", 200, "", playlist.to_vec()),
            ("/a.ts", 200, "", b"A".to_vec()),
        ]);
        let url = root.join("vod.m3u8").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("vod.ts");
        fs::write(&output, b"precious").await.unwrap();
        let dl = |overwrite| {
            Downloader::new(
                Client::new(&[], 0, Duration::from_secs(5)).unwrap(),
                Options {
                    remux: false,
                    overwrite,
                    ..Options::default()
                },
                Arc::new(|_| {}),
                CancellationToken::new(),
            )
        };
        let err = dl(false).download(url.as_str(), &output).await.unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err}");
        assert_eq!(fs::read(&output).await.unwrap(), b"precious");
        // The capture is kept, so the rerun with --force resumes instead of refetching.
        dl(true).download(url.as_str(), &output).await.unwrap();
        assert_eq!(fs::read(&output).await.unwrap(), b"A");
    }
}
