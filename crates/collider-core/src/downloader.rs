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
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// The manifest was fetched and recognised.
    Detected {
        protocol: Protocol,
    },
    /// A track was selected. `live` is true when the playlist has no end yet.
    TrackStarted {
        track: String,
        live: bool,
    },
    /// The number of known segments for a track changed.
    Discovered {
        track: String,
        total: usize,
    },
    /// A segment finished. `resumed` means it was already on disk from an earlier run.
    SegmentDone {
        track: String,
        bytes: u64,
        resumed: bool,
    },
    /// A live capture ended with gaps (segments that could not be fetched).
    Gaps {
        track: String,
        missing: usize,
    },
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
    file: PathBuf,
}

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

    pub async fn info(&self, url: &str) -> Result<StreamInfo> {
        let probe = source::probe(&self.client, &Url::parse(url)?).await?;
        source::describe(&probe)
    }

    /// Download `url` (HLS playlist or DASH MPD) to `output`. Returns the files written.
    pub async fn download(&self, url: &str, output: &Path) -> Result<Vec<PathBuf>> {
        let url = Url::parse(url)?;
        let tracks = loop {
            match self.plan(&url).await {
                Ok(tracks) => break tracks,
                Err(e) if self.opts.wait.is_some() && waitable(&e) => {
                    let interval = self.opts.wait.unwrap_or(Duration::from_secs(30));
                    (self.report)(Event::Status(format!(
                        "waiting ({e}); next check in {}s",
                        interval.as_secs()
                    )));
                    tokio::select! {
                        _ = tokio::time::sleep(interval) => {}
                        _ = self.cancel.cancelled() => return Err(Error::Cancelled),
                    }
                }
                Err(e) => return Err(e),
            }
        };

        let work = work_dir(output);
        fs::create_dir_all(&work).await?;
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
            (self.report)(Event::Status(format!("{}: {}", t.name, t.description)));
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

        loop {
            let snap = track.source.refresh().await?;
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
                        fetch_init(&self.client, &init, &dir).await?;
                    }
                }
                order.extend(fresh.iter().cloned());
                (self.report)(Event::Discovered {
                    track: name.clone(),
                    total: order.len(),
                });
                self.fetch_batch(&name, &fresh, &keys, &dir, live).await?;
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
                _ = tokio::time::sleep(wait.clamp(Duration::from_millis(500), Duration::from_secs(30))) => {}
                _ = self.cancel.cancelled() => {
                    (self.report)(Event::Status(format!("{name}: stopping, saving what was captured")));
                    break;
                }
            }
        }

        let (file, missing) = concat_track(&name, &order, &dir, live).await?;
        if missing > 0 {
            (self.report)(Event::Gaps {
                track: name.clone(),
                missing,
            });
        }
        Ok(TrackOutput { name, file })
    }

    async fn resolve_keys(
        &self,
        segs: &[Segment],
        keys: &mut HashMap<Url, [u8; 16]>,
    ) -> Result<()> {
        let wanted: HashSet<&Url> = segs
            .iter()
            .filter_map(|s| s.key.as_ref().map(|k| &k.url))
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

    async fn fetch_batch(
        &self,
        track: &str,
        segs: &[Segment],
        keys: &HashMap<Url, [u8; 16]>,
        dir: &Path,
        live: bool,
    ) -> Result<()> {
        let client = &self.client;
        let stream = futures::stream::iter(segs.iter())
            .map(|seg| async move {
                let key = seg.key.as_ref().map(|k| (keys[&k.url], k.iv));
                fetch_segment(client, seg, key, dir, live).await
            })
            .buffer_unordered(self.opts.concurrency)
            .take_until(self.cancel.cancelled());
        futures::pin_mut!(stream);

        while let Some(done) = stream.try_next().await? {
            if let Some((bytes, resumed)) = done {
                (self.report)(Event::SegmentDone {
                    track: track.to_string(),
                    bytes,
                    resumed,
                });
            }
        }
        Ok(())
    }

    async fn finish(&self, outputs: &[TrackOutput], output: &Path) -> Result<Vec<PathBuf>> {
        if self.opts.remux && mux::ffmpeg_available().await {
            (self.report)(Event::Status("remuxing with ffmpeg (stream copy)".into()));
            let inputs: Vec<PathBuf> = outputs.iter().map(|o| o.file.clone()).collect();
            mux::remux(&inputs, output).await?;
            return Ok(vec![output.to_path_buf()]);
        }
        if self.opts.remux {
            (self.report)(Event::Status(
                "ffmpeg not found; keeping raw stream files".into(),
            ));
        }
        let mut files = Vec::new();
        for o in outputs {
            let ext = o.file.extension().and_then(|e| e.to_str()).unwrap_or("ts");
            let dest = if outputs.len() == 1 {
                output.with_extension(ext)
            } else {
                output.with_extension(format!("{}.{ext}", o.name))
            };
            move_file(&o.file, &dest).await?;
            files.push(dest);
        }
        Ok(files)
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

/// Returns `Some((bytes, resumed))`, or `None` when a live segment was never published.
async fn fetch_segment(
    client: &Client,
    seg: &Segment,
    key: Option<([u8; 16], [u8; 16])>,
    dir: &Path,
    live: bool,
) -> Result<Option<(u64, bool)>> {
    let path = segment_path(dir, seg.seq);
    if let Ok(meta) = fs::metadata(&path).await {
        return Ok(Some((meta.len(), true)));
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
                status: 404 | 410,
                url,
            }) if live => {
                tracing::warn!(%url, "live segment is gone, skipping");
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
    };
    let data = match key {
        Some((k, iv)) => decrypt_aes128(&data, &k, &iv)?,
        None => data.to_vec(),
    };
    write_atomic(&path, &data).await?;
    Ok(Some((data.len() as u64, false)))
}

async fn fetch_init(client: &Client, init: &InitSection, dir: &Path) -> Result<()> {
    let path = dir.join(init.file_name());
    if fs::metadata(&path).await.is_ok() {
        return Ok(());
    }
    let data = client.get_bytes(&init.url, init.byte_range).await?;
    write_atomic(&path, &data).await
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

/// Join init sections + segments in playlist order into a single file.
/// Returns the file and the number of segments that were missing (live gaps).
async fn concat_track(
    name: &str,
    order: &[Segment],
    dir: &Path,
    allow_gaps: bool,
) -> Result<(PathBuf, usize)> {
    let out = dir.with_file_name(format!("{name}.{}", container_ext(order)));
    let mut f = fs::File::create(&out).await?;
    let mut current_init: Option<&InitSection> = None;
    let mut missing = 0usize;
    for seg in order {
        let data = match fs::read(segment_path(dir, seg.seq)).await {
            Ok(data) => data,
            Err(e) if allow_gaps && e.kind() == std::io::ErrorKind::NotFound => {
                missing += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if seg.init.as_ref() != current_init {
            if let Some(init) = &seg.init {
                f.write_all(&fs::read(dir.join(init.file_name())).await?)
                    .await?;
            }
            current_init = seg.init.as_ref();
        }
        f.write_all(&data).await?;
    }
    f.flush().await?;
    Ok((out, missing))
}

async fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).await.is_err() {
        fs::copy(from, to).await?;
        fs::remove_file(from).await?;
    }
    Ok(())
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
        });
        assert_eq!(container_ext(&[s]), "mp4");
    }

    #[tokio::test]
    async fn concat_orders_gaps_and_init_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("main");
        fs::create_dir_all(&dir).await.unwrap();
        let init_a = InitSection {
            url: Url::parse("https://x.test/ia").unwrap(),
            byte_range: None,
        };
        let init_b = InitSection {
            url: Url::parse("https://x.test/ib").unwrap(),
            byte_range: None,
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
        let (out, missing) = concat_track("main", &order, &dir, true).await.unwrap();
        assert_eq!(missing, 1);
        assert_eq!(fs::read(out).await.unwrap(), b"[IA]AB[IB]D");
    }
}
