//! Download jobs. The engine runs each job on the tokio runtime and reports [`Event`]s,
//! which are folded into a shared [`JobState`] that the UI reads every frame.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use collider_core::{
    Client, Downloader, Error, Event, Options, Protocol, Quality, Reporter, StreamInfo,
};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

/// Everything needed to (re)start a download.
#[derive(Debug, Clone)]
pub struct JobSpec {
    pub url: String,
    pub output: PathBuf,
    pub quality: Quality,
    pub audio_lang: Option<String>,
    pub concurrency: usize,
    pub max_duration: Option<Duration>,
    /// Watch mode: poll at this interval until the stream is live.
    pub wait: Option<Duration>,
    pub retries: u32,
    pub timeout: Duration,
    pub headers: Vec<(String, String)>,
}

/// Why the user stopped a running job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Intent {
    #[default]
    None,
    /// Stop a live recording and save what was captured.
    Save,
    /// Stop a download and keep its segments, so it can resume.
    Pause,
    /// Stop and delete everything this job wrote.
    Discard,
    /// Stop and start again at once (watch mode: check now).
    Restart,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Reading the manifest.
    Starting,
    /// Watch mode: the stream is not available yet.
    Waiting {
        reason: String,
        next_check: Instant,
        checks: u32,
    },
    /// Downloading segments, or recording a live stream.
    Running,
    /// A stop was requested; segments in flight are finishing.
    Stopping,
    Remuxing,
    Paused,
    /// Stopped in order to start again; the UI restarts it on the next frame.
    Restarting,
    Completed {
        files: Vec<PathBuf>,
    },
    Failed {
        message: String,
    },
    /// Refused: the stream uses DRM.
    Unsupported {
        message: String,
    },
    Cancelled,
}

impl Phase {
    /// The engine is working on the job.
    pub fn is_running(&self) -> bool {
        matches!(
            self,
            Phase::Starting
                | Phase::Waiting { .. }
                | Phase::Running
                | Phase::Stopping
                | Phase::Remuxing
                | Phase::Restarting
        )
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            Phase::Completed { .. }
                | Phase::Failed { .. }
                | Phase::Unsupported { .. }
                | Phase::Cancelled
        )
    }
}

/// The state of one segment, for the segment map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Pending,
    InFlight,
    Done,
    /// Already on disk from an earlier run.
    Resumed,
    /// A live segment that could not be fetched.
    Gap,
    /// Implied by the manifest but absent at the origin.
    Skipped,
}

#[derive(Debug, Clone, Default)]
pub struct Track {
    pub name: String,
    pub description: String,
    pub live: bool,
    pub cells: Vec<Cell>,
    /// Bytes on disk, resumed segments included.
    pub bytes: u64,
    /// Segments on disk (fetched or resumed).
    pub done: usize,
    pub resumed: usize,
    pub gaps: usize,
    pub skipped: usize,
    pub in_flight: usize,
}

impl Track {
    /// Segments that need no more work.
    pub fn settled(&self) -> usize {
        self.done + self.gaps + self.skipped
    }

    fn set(&mut self, index: usize, cell: Cell) {
        if index >= self.cells.len() {
            self.cells.resize(index + 1, Cell::Pending);
        }
        if std::mem::replace(&mut self.cells[index], cell) == Cell::InFlight {
            self.in_flight = self.in_flight.saturating_sub(1);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Ok,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub at: DateTime<Local>,
    pub level: Level,
    pub text: String,
}

pub const BUCKET_SECS: f64 = 2.0;
/// Five minutes of 2 s buckets.
pub const BUCKETS: usize = 150;

/// Network throughput in 2 s buckets.
#[derive(Debug, Clone)]
pub struct RateMeter {
    origin: Instant,
    first: u64,
    buckets: VecDeque<u64>,
}

impl RateMeter {
    fn new(now: Instant) -> Self {
        Self {
            origin: now,
            first: 0,
            buckets: VecDeque::new(),
        }
    }

    fn slot(&self, now: Instant) -> u64 {
        (now.saturating_duration_since(self.origin).as_secs_f64() / BUCKET_SECS) as u64
    }

    fn get(&self, slot: u64) -> u64 {
        slot.checked_sub(self.first)
            .and_then(|i| self.buckets.get(i as usize).copied())
            .unwrap_or(0)
    }

    fn add(&mut self, now: Instant, bytes: u64) {
        if now < self.origin {
            return;
        }
        let slot = self.slot(now);
        while self.first + (self.buckets.len() as u64) <= slot {
            self.buckets.push_back(0);
        }
        while self.buckets.len() > BUCKETS + 1 {
            self.buckets.pop_front();
            self.first += 1;
        }
        if let Some(b) = slot
            .checked_sub(self.first)
            .and_then(|i| self.buckets.get_mut(i as usize))
        {
            *b += bytes;
        }
    }

    /// Bytes per second over roughly the last ten seconds. Live segments arrive every
    /// few seconds, so a shorter window would swing between zero and double the rate.
    pub fn current(&self, now: Instant) -> f64 {
        const WINDOW: u64 = 5;
        let since = now.saturating_duration_since(self.origin).as_secs_f64();
        let slot = self.slot(now);
        let into_slot = since - slot as f64 * BUCKET_SECS;
        let first = slot.saturating_sub(WINDOW);
        let bytes: u64 = (first..=slot).map(|s| self.get(s)).sum();
        let span = ((slot - first) as f64 * BUCKET_SECS + into_slot).min(since);
        bytes as f64 / span.max(1.0)
    }

    /// Bytes per second of the last `n` complete buckets, oldest first.
    pub fn series(&self, now: Instant, n: usize) -> Vec<f64> {
        let slot = self.slot(now);
        (0..n)
            .map(|k| {
                let back = (n - k) as u64;
                slot.checked_sub(back)
                    .map_or(0.0, |s| self.get(s) as f64 / BUCKET_SECS)
            })
            .collect()
    }

    /// The busiest complete bucket, in bytes per second.
    pub fn peak(&self, now: Instant) -> f64 {
        self.series(now, BUCKETS).into_iter().fold(0.0, f64::max)
    }
}

#[derive(Debug)]
pub struct JobState {
    pub phase: Phase,
    pub intent: Intent,
    pub protocol: Option<Protocol>,
    pub live: bool,
    pub tracks: Vec<Track>,
    pub log: Vec<LogLine>,
    /// When the current run began capturing.
    pub capture_started: Option<(Instant, DateTime<Local>)>,
    pub finished_at: Option<Instant>,
    /// Wall-clock time the job ended.
    pub finished_wall: Option<DateTime<Local>>,
    pub rate: RateMeter,
    /// Bytes fetched over the network in this run (resumed segments excluded).
    pub fetched: u64,
    pub runs: u32,
}

const LOG_LIMIT: usize = 400;

impl JobState {
    fn new() -> Self {
        Self {
            phase: Phase::Starting,
            intent: Intent::None,
            protocol: None,
            live: false,
            tracks: Vec::new(),
            log: Vec::new(),
            capture_started: None,
            finished_at: None,
            finished_wall: None,
            rate: RateMeter::new(Instant::now()),
            fetched: 0,
            runs: 0,
        }
    }

    /// Start the throughput history at `origin` (demo scenes replay past events).
    #[cfg(test)]
    pub fn backdate(&mut self, origin: Instant) {
        self.rate = RateMeter::new(origin);
    }

    pub(crate) fn begin_run(&mut self) {
        self.phase = Phase::Starting;
        self.intent = Intent::None;
        self.tracks.clear();
        self.capture_started = None;
        self.finished_at = None;
        self.finished_wall = None;
        self.rate = RateMeter::new(Instant::now());
        self.fetched = 0;
        self.runs += 1;
    }

    /// What is being captured, compactly: `1920×1080 avc1 + en`.
    pub fn summary(&self) -> String {
        self.tracks
            .iter()
            .map(|t| compact_track(&t.description))
            .filter(|d| !d.is_empty())
            .collect::<Vec<_>>()
            .join(" + ")
    }

    pub fn bytes(&self) -> u64 {
        self.tracks.iter().map(|t| t.bytes).sum()
    }

    pub fn total(&self) -> usize {
        self.tracks.iter().map(|t| t.cells.len()).sum()
    }

    pub fn done(&self) -> usize {
        self.tracks.iter().map(|t| t.done).sum()
    }

    pub fn settled(&self) -> usize {
        self.tracks.iter().map(Track::settled).sum()
    }

    pub fn resumed(&self) -> usize {
        self.tracks.iter().map(|t| t.resumed).sum()
    }

    pub fn gaps(&self) -> usize {
        self.tracks.iter().map(|t| t.gaps).sum()
    }

    pub fn in_flight(&self) -> usize {
        self.tracks.iter().map(|t| t.in_flight).sum()
    }

    /// Fraction of segments settled, for a download whose length is known.
    pub fn progress(&self) -> Option<f32> {
        let total = self.total();
        (!self.live && total > 0).then(|| self.settled() as f32 / total as f32)
    }

    /// Seconds since capture began (frozen once the job ended).
    pub fn elapsed(&self, now: Instant) -> f64 {
        self.capture_started.map_or(0.0, |(t, _)| {
            self.finished_at
                .unwrap_or(now)
                .saturating_duration_since(t)
                .as_secs_f64()
        })
    }

    /// Estimated seconds left for a download, from the recent rate.
    pub fn eta(&self, now: Instant) -> Option<f64> {
        let (done, total) = (self.done(), self.total());
        let fetched_segs = done.saturating_sub(self.resumed());
        if self.live || fetched_segs == 0 || total <= done {
            return None;
        }
        let per_seg = self.fetched as f64 / fetched_segs as f64;
        let rate = self.rate.current(now);
        (rate > 0.0).then(|| (total - done) as f64 * per_seg / rate)
    }

    fn track(&mut self, name: &str) -> &mut Track {
        match self.tracks.iter().position(|t| t.name == name) {
            Some(i) => &mut self.tracks[i],
            None => {
                self.tracks.push(Track {
                    name: name.to_string(),
                    ..Track::default()
                });
                self.tracks.last_mut().expect("just pushed")
            }
        }
    }

    fn log(&mut self, level: Level, text: impl Into<String>) {
        self.log.push(LogLine {
            at: Local::now(),
            level,
            text: text.into(),
        });
        let excess = self.log.len().saturating_sub(LOG_LIMIT);
        self.log.drain(..excess);
    }

    pub fn apply(&mut self, event: Event, now: Instant) {
        match event {
            Event::Detected { protocol } => {
                self.protocol = Some(protocol);
                self.log(Level::Info, format!("Probed {protocol} stream"));
            }
            Event::TrackPlanned { track, description } => {
                self.log(Level::Info, format!("Selected {track}: {description}"));
                self.track(&track).description = description;
            }
            Event::TrackStarted { track, live } => {
                self.track(&track).live = live;
                self.live |= live;
                if self.capture_started.is_none() {
                    self.capture_started = Some((now, Local::now()));
                    self.log(
                        Level::Info,
                        if live {
                            "Recording started, following the live playlist"
                        } else {
                            "Download started"
                        },
                    );
                }
                if matches!(self.phase, Phase::Starting | Phase::Waiting { .. }) {
                    self.phase = Phase::Running;
                }
            }
            Event::Discovered { track, total } => {
                let t = self.track(&track);
                if t.cells.len() < total {
                    t.cells.resize(total, Cell::Pending);
                }
            }
            Event::SegmentStarted { track, index } => {
                let t = self.track(&track);
                t.set(index, Cell::InFlight);
                t.in_flight += 1;
            }
            Event::SegmentDone {
                track,
                index,
                bytes,
                resumed,
            } => {
                let t = self.track(&track);
                t.set(index, if resumed { Cell::Resumed } else { Cell::Done });
                t.done += 1;
                t.bytes += bytes;
                if resumed {
                    t.resumed += 1;
                } else {
                    self.fetched += bytes;
                    self.rate.add(now, bytes);
                }
            }
            Event::SegmentSkipped { track, index, gap } => {
                let t = self.track(&track);
                t.set(index, if gap { Cell::Gap } else { Cell::Skipped });
                if gap {
                    t.gaps += 1;
                    self.log(
                        Level::Warn,
                        format!(
                            "Segment {} of {track} could not be fetched; the recording has a gap",
                            index + 1
                        ),
                    );
                } else {
                    t.skipped += 1;
                }
            }
            Event::Gaps { track, missing } => {
                self.log(
                    Level::Warn,
                    format!("{track}: {missing} segment(s) missing from the recording"),
                );
            }
            Event::Waiting { reason, retry_in } => {
                let checks = match &self.phase {
                    Phase::Waiting { checks, .. } => checks + 1,
                    _ => {
                        self.log(
                            Level::Info,
                            format!(
                                "Not available yet ({reason}); checking every {} s",
                                retry_in.as_secs()
                            ),
                        );
                        1
                    }
                };
                self.phase = Phase::Waiting {
                    reason,
                    next_check: now + retry_in,
                    checks,
                };
            }
            Event::Remuxing => {
                self.phase = Phase::Remuxing;
                self.log(Level::Info, "Remuxing with ffmpeg (stream copy)");
            }
            Event::Status(text) => {
                let warn = text.contains("failed") || text.contains("unavailable");
                self.log(
                    if warn { Level::Warn } else { Level::Info },
                    capitalize(&text),
                );
            }
        }
    }

    pub fn finish(&mut self, result: Result<Vec<PathBuf>, Error>, now: Instant) {
        self.finished_at = Some(now);
        self.finished_wall = Some(Local::now());
        for t in &mut self.tracks {
            for c in &mut t.cells {
                if *c == Cell::InFlight {
                    *c = Cell::Pending;
                }
            }
            t.in_flight = 0;
        }
        self.phase = match result {
            Ok(files) => {
                for f in &files {
                    self.log(Level::Ok, format!("Saved {}", crate::format::tilde(f)));
                }
                Phase::Completed { files }
            }
            Err(Error::Cancelled) => match self.intent {
                Intent::Discard => {
                    self.log(Level::Info, "Cancelled; everything downloaded was deleted");
                    Phase::Cancelled
                }
                Intent::Restart => Phase::Restarting,
                _ => {
                    self.log(
                        Level::Info,
                        "Paused; finished segments are kept, so it resumes where it stopped",
                    );
                    Phase::Paused
                }
            },
            Err(Error::Unsupported(message)) => {
                self.log(Level::Warn, message.clone());
                Phase::Unsupported { message }
            }
            Err(e) => {
                let message = e.to_string();
                self.log(Level::Warn, message.clone());
                Phase::Failed { message }
            }
        };
    }
}

/// Shorten the engine's track description: `1920x1080 @ 5900 kbps [avc1.640028,mp4a.40.2]`
/// becomes `1920×1080 avc1`, `a-en @ 128 kbps [mp4a.40.2] (en)` becomes `en mp4a`, and
/// `English (en)` becomes `en`.
pub fn compact_track(desc: &str) -> String {
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let resolution = desc
        .split_whitespace()
        .find(|w| {
            w.split_once('x')
                .is_some_and(|(a, b)| digits(a) && digits(b))
        })
        .map(|w| w.replace('x', "×"));
    let codec = desc
        .split_once('[')
        .and_then(|(_, rest)| rest.split(']').next())
        .and_then(|c| c.split(',').next())
        .map(|c| c.split('.').next().unwrap_or(c).trim().to_string())
        .filter(|c| !c.is_empty());
    let lang = desc
        .trim_end()
        .strip_suffix(')')
        .and_then(|d| d.rsplit_once('('))
        .map(|(_, l)| l.to_string())
        .filter(|l| !l.is_empty() && l.len() <= 12);
    [resolution.or(lang), codec]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// collider's work directory for `output` (see `collider_core::downloader`).
pub fn parts_dir(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    output.with_file_name(format!("{name}.parts"))
}

fn lock(state: &Mutex<JobState>) -> MutexGuard<'_, JobState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Job {
    pub id: u64,
    pub name: String,
    pub spec: JobSpec,
    pub created: DateTime<Local>,
    state: Arc<Mutex<JobState>>,
    cancel: Option<CancellationToken>,
    /// Already written to the history.
    pub archived: bool,
}

impl Job {
    pub fn new(id: u64, name: String, spec: JobSpec) -> Self {
        Self {
            id,
            name,
            spec,
            created: Local::now(),
            state: Arc::new(Mutex::new(JobState::new())),
            cancel: None,
            archived: false,
        }
    }

    pub fn state(&self) -> MutexGuard<'_, JobState> {
        lock(&self.state)
    }

    /// Start (or restart, resuming from the parts on disk) on the runtime.
    pub fn start(&mut self, rt: &Handle, ctx: &egui::Context) {
        self.state().begin_run();
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        let spec = &self.spec;
        let client = match Client::new(&spec.headers, spec.retries, spec.timeout) {
            Ok(c) => c,
            Err(e) => {
                self.state().finish(Err(e), Instant::now());
                return;
            }
        };
        let reporter: Reporter = {
            let state = self.state.clone();
            let ctx = ctx.clone();
            Arc::new(move |e| {
                lock(&state).apply(e, Instant::now());
                ctx.request_repaint();
            })
        };
        let opts = Options {
            concurrency: spec.concurrency,
            quality: spec.quality.clone(),
            audio_lang: spec.audio_lang.clone(),
            max_duration: spec.max_duration,
            keep_parts: false,
            remux: true,
            wait: spec.wait,
            overwrite: false,
        };
        let dl = Downloader::new(client, opts, reporter, cancel);
        let (url, output) = (spec.url.clone(), spec.output.clone());
        let (state, ctx) = (self.state.clone(), ctx.clone());
        rt.spawn(async move {
            let result = match output.parent() {
                Some(dir) => match tokio::fs::create_dir_all(dir).await {
                    Ok(()) => dl.download(&url, &output).await,
                    Err(e) => Err(e.into()),
                },
                None => dl.download(&url, &output).await,
            };
            let discard = lock(&state).intent == Intent::Discard;
            if discard {
                // A stopped live capture is saved before the discard can take effect.
                if let Ok(files) = &result {
                    for f in files {
                        tokio::fs::remove_file(f).await.ok();
                    }
                }
                tokio::fs::remove_dir_all(parts_dir(&output)).await.ok();
            }
            let result = if discard {
                Err(Error::Cancelled)
            } else {
                result
            };
            lock(&state).finish(result, Instant::now());
            ctx.request_repaint();
        });
    }

    /// Ask a running job to stop; what happens next depends on `intent`.
    pub fn stop(&mut self, intent: Intent) {
        let mut st = self.state();
        if !st.phase.is_running() {
            return;
        }
        st.intent = intent;
        if !matches!(st.phase, Phase::Remuxing) {
            st.phase = Phase::Stopping;
        }
        drop(st);
        if let Some(c) = &self.cancel {
            c.cancel();
        }
    }

    /// Delete the parts of a job that is not running (paused or failed).
    pub fn delete_parts(&self, rt: &Handle) {
        let dir = parts_dir(&self.spec.output);
        rt.spawn(async move {
            tokio::fs::remove_dir_all(dir).await.ok();
        });
    }
}

/// A manifest probe running in the background.
pub struct Probe {
    pub url: String,
    result: Arc<Mutex<Option<ProbeResult>>>,
}

pub type ProbeResult = Result<(StreamInfo, Duration), String>;

impl Probe {
    pub fn start(
        rt: &Handle,
        ctx: &egui::Context,
        url: String,
        headers: &[(String, String)],
        retries: u32,
        timeout: Duration,
    ) -> Self {
        let result = Arc::new(Mutex::new(None));
        let started = Instant::now();
        let client = Client::new(headers, retries.min(2), timeout);
        let (slot, ctx, target) = (result.clone(), ctx.clone(), url.clone());
        rt.spawn(async move {
            let outcome = match client {
                Ok(client) => {
                    let dl = Downloader::new(
                        client,
                        Options::default(),
                        Arc::new(|_| {}),
                        CancellationToken::new(),
                    );
                    dl.info(&target)
                        .await
                        .map(|info| (info, started.elapsed()))
                        .map_err(|e| e.to_string())
                }
                Err(e) => Err(e.to_string()),
            };
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
            ctx.request_repaint();
        });
        Self { url, result }
    }

    pub fn take(&self) -> Option<ProbeResult> {
        self.result.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// The ffmpeg that collider would run: the first `ffmpeg` on `PATH`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ffmpeg {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

pub fn detect_ffmpeg() -> Ffmpeg {
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(exe))
            .find(|p| p.is_file())
    });
    let version = path.as_ref().and_then(|p| {
        let out = std::process::Command::new(p)
            .arg("-version")
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        parse_ffmpeg_version(&String::from_utf8_lossy(&out.stdout))
    });
    Ffmpeg { path, version }
}

fn parse_ffmpeg_version(banner: &str) -> Option<String> {
    let token = banner
        .lines()
        .next()?
        .split_whitespace()
        .skip_while(|w| *w != "version")
        .nth(1)?;
    let token = token
        .strip_prefix('n')
        .filter(|t| t.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(token);
    // `7.1.1-2.fc44` → `7.1.1`; keep git builds (`N-1234-g…`) short.
    let clean = token.split(['-', '+', '~']).next().unwrap_or(token);
    Some(
        if clean.is_empty() || clean == "N" {
            token
        } else {
            clean
        }
        .chars()
        .take(12)
        .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev_done(track: &str, index: usize, bytes: u64, resumed: bool) -> Event {
        Event::SegmentDone {
            track: track.into(),
            index,
            bytes,
            resumed,
        }
    }

    #[test]
    fn events_build_tracks_cells_and_rates() {
        let t0 = Instant::now();
        let mut st = JobState::new();
        st.begin_run();
        st.apply(
            Event::TrackStarted {
                track: "video".into(),
                live: false,
            },
            t0,
        );
        assert_eq!(st.phase, Phase::Running);
        st.apply(
            Event::Discovered {
                track: "video".into(),
                total: 4,
            },
            t0,
        );
        for i in 0..3 {
            st.apply(
                Event::SegmentStarted {
                    track: "video".into(),
                    index: i,
                },
                t0,
            );
        }
        st.apply(ev_done("video", 0, 1000, true), t0);
        st.apply(
            ev_done("video", 1, 4000, false),
            t0 + Duration::from_secs(1),
        );
        st.apply(
            Event::SegmentSkipped {
                track: "video".into(),
                index: 2,
                gap: false,
            },
            t0,
        );
        let v = &st.tracks[0];
        assert_eq!(
            v.cells,
            [Cell::Resumed, Cell::Done, Cell::Skipped, Cell::Pending]
        );
        assert_eq!((v.done, v.resumed, v.skipped, v.in_flight), (2, 1, 1, 0));
        assert_eq!(st.bytes(), 5000);
        assert_eq!(st.fetched, 4000);
        assert_eq!(st.progress(), Some(0.75));
        // 4000 bytes landed in the first 2 s bucket.
        let series = st.rate.series(t0 + Duration::from_secs(3), 2);
        assert_eq!(series, [0.0, 2000.0]);
        assert!(st.rate.current(t0 + Duration::from_secs(1)) > 0.0);
    }

    #[test]
    fn stopping_maps_to_the_intended_outcome() {
        let mut st = JobState::new();
        st.intent = Intent::Pause;
        st.finish(Err(Error::Cancelled), Instant::now());
        assert_eq!(st.phase, Phase::Paused);
        st.intent = Intent::Restart;
        st.finish(Err(Error::Cancelled), Instant::now());
        assert_eq!(st.phase, Phase::Restarting);
        st.intent = Intent::Discard;
        st.finish(Err(Error::Cancelled), Instant::now());
        assert_eq!(st.phase, Phase::Cancelled);
        st.finish(Err(Error::Unsupported("DRM".into())), Instant::now());
        assert!(matches!(st.phase, Phase::Unsupported { .. }));
        st.finish(Ok(vec![PathBuf::from("/tmp/a.mp4")]), Instant::now());
        assert!(st.phase.is_finished());
    }

    #[test]
    fn waiting_counts_checks_and_logs_once() {
        let mut st = JobState::new();
        for _ in 0..3 {
            st.apply(
                Event::Waiting {
                    reason: "HTTP 404".into(),
                    retry_in: Duration::from_secs(20),
                },
                Instant::now(),
            );
        }
        assert!(matches!(st.phase, Phase::Waiting { checks: 3, .. }));
        assert_eq!(st.log.len(), 1);
    }

    #[test]
    fn ffmpeg_versions() {
        let v = |s: &str| parse_ffmpeg_version(s);
        assert_eq!(
            v("ffmpeg version 7.1.1 Copyright (c) 2000-2025").as_deref(),
            Some("7.1.1")
        );
        assert_eq!(v("ffmpeg version n6.1-3 Copyright").as_deref(), Some("6.1"));
        assert_eq!(
            v("ffmpeg version 7.1.1-2.fc44 Copyright").as_deref(),
            Some("7.1.1")
        );
        assert_eq!(
            v("ffmpeg version N-112233-gabcdef Copyright").as_deref(),
            Some("N-112233-gab")
        );
        assert_eq!(v("garbage"), None);
    }

    #[test]
    fn track_descriptions_are_compacted() {
        let c = compact_track;
        assert_eq!(
            c("1920x1080 @ 5900 kbps [avc1.640028,mp4a.40.2]"),
            "1920×1080 avc1"
        );
        assert_eq!(
            c("v4k 3840x1636 @ 14000 kbps [hev1.2.4.L153]"),
            "3840×1636 hev1"
        );
        assert_eq!(c("a-en @ 128 kbps [mp4a.40.2] (en)"), "en mp4a");
        assert_eq!(c("English (en)"), "en");
        assert_eq!(c("media playlist, 1204 segments"), "");
        assert_eq!(c("unknown resolution @ 800 kbps"), "");
    }

    #[test]
    fn rate_is_smoothed_over_ten_seconds() {
        let t0 = Instant::now();
        let mut m = RateMeter::new(t0);
        // A 2 MB live segment every 4 s is 0.5 MB/s, whenever you look.
        for k in 0..10 {
            m.add(t0 + Duration::from_secs(4 * k), 2_000_000);
        }
        for probe in [37.0, 38.5, 39.9] {
            let r = m.current(t0 + Duration::from_secs_f64(probe));
            assert!((350_000.0..650_000.0).contains(&r), "{probe}: {r}");
        }
        // Samples from before the meter started are ignored.
        let mut late = RateMeter::new(t0 + Duration::from_secs(10));
        late.add(t0, 1_000_000);
        assert_eq!(late.current(t0 + Duration::from_secs(12)), 0.0);
    }

    #[test]
    fn parts_dir_matches_the_engine() {
        assert_eq!(
            parts_dir(Path::new("/v/show.mp4")),
            PathBuf::from("/v/show.mp4.parts")
        );
    }
}
