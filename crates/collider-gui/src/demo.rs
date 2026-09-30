//! A demo scene that mirrors the design mockups, built by replaying engine events into
//! real job states, and a test that renders every page to PNG.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use collider_core::model::{AudioInfo, VariantInfo};
use collider_core::{Error, Event, Protocol, StreamInfo};

use crate::app::{App, Page, Storage};
use crate::history::{Outcome, Record};
use crate::jobs::{Ffmpeg, JobState};
use crate::settings::{Container, Header, QualityPref};
use crate::views::new_download::Draft;

fn ago(now: Instant, secs: f64) -> Instant {
    now.checked_sub(Duration::from_secs_f64(secs))
        .unwrap_or(now)
}

fn planned(
    st: &mut JobState,
    at: Instant,
    protocol: Protocol,
    tracks: &[(&str, &str, usize, bool)],
) {
    st.apply(Event::Detected { protocol }, at);
    for (track, description, _, _) in tracks {
        st.apply(
            Event::TrackPlanned {
                track: track.to_string(),
                description: description.to_string(),
            },
            at,
        );
    }
    for (track, _, total, live) in tracks {
        st.apply(
            Event::TrackStarted {
                track: track.to_string(),
                live: *live,
            },
            at,
        );
        st.apply(
            Event::Discovered {
                track: track.to_string(),
                total: *total,
            },
            at,
        );
    }
}

/// Segments `range` of `track` finish evenly over `from..to`, about `bytes` each.
fn done(
    st: &mut JobState,
    track: &str,
    range: std::ops::Range<usize>,
    bytes: u64,
    (from, to): (Instant, Instant),
    resumed: bool,
) {
    let n = range.len().max(1);
    let span = to.saturating_duration_since(from);
    for (k, index) in range.enumerate() {
        let at = from + span.mul_f64(k as f64 / n as f64);
        let wobble = 1.0 + 0.18 * ((k as f64) * 0.37).sin() + 0.07 * ((k as f64) * 1.9).cos();
        st.apply(
            Event::SegmentStarted {
                track: track.into(),
                index,
            },
            at,
        );
        st.apply(
            Event::SegmentDone {
                track: track.into(),
                index,
                bytes: (bytes as f64 * wobble) as u64,
                resumed,
            },
            at,
        );
    }
}

fn in_flight(st: &mut JobState, track: &str, range: std::ops::Range<usize>, at: Instant) {
    for index in range {
        st.apply(
            Event::SegmentStarted {
                track: track.into(),
                index,
            },
            at,
        );
    }
}

/// The Downloads mockup's six jobs, a probed stream, custom headers and some history.
pub fn scene(app: &mut App) {
    let now = Instant::now();
    let home = dirs::home_dir().unwrap_or_default();
    app.settings.output_dir = home.join("Videos").join("collider");
    *app.ffmpeg.lock().unwrap() = Some(Ffmpeg {
        path: Some(PathBuf::from("/usr/bin/ffmpeg")),
        version: Some("7.1".into()),
    });

    // Inserted oldest first; the list shows newest first.
    let url = "https://tv.provider.example/premium-channel.m3u8";
    let spec = app.default_spec(url, "premium-channel");
    let job = app.insert_job("premium-channel", spec);
    {
        let mut st = job.state();
        st.apply(
            Event::Detected {
                protocol: Protocol::Hls,
            },
            ago(now, 600.0),
        );
        st.finish(
            Err(Error::Unsupported(
                "encryption method SampleAes: only clear-key AES-128 is supported (DRM is out \
                 of scope)"
                    .into(),
            )),
            ago(now, 600.0),
        );
    }

    let url = "https://cdn.openfilms.example/open-movie/master.m3u8";
    let mut spec = app.default_spec(url, "open-movie-1080p");
    spec.output = app.settings.output_dir.join("open-movie-1080p.mkv");
    let job = app.insert_job("open-movie-1080p", spec.clone());
    {
        let mut st = job.state();
        let t0 = ago(now, 900.0);
        st.backdate(t0);
        planned(
            &mut st,
            t0,
            Protocol::Hls,
            &[
                ("video", "1920x1080 @ 5000 kbps [avc1.640028]", 441, false),
                ("audio", "English (en)", 441, false),
            ],
        );
        let window = (t0, t0 + Duration::from_secs(34));
        done(&mut st, "video", 0..441, 5_700_000, window, false);
        done(&mut st, "audio", 0..441, 64_000, window, false);
        st.apply(Event::Remuxing, t0 + Duration::from_secs(35));
        st.finish(Ok(vec![spec.output.clone()]), t0 + Duration::from_secs(36));
    }

    let url = "https://live.devconf.example/keynote-2026.m3u8";
    let mut spec = app.default_spec(url, "keynote-2026");
    spec.wait = Some(Duration::from_secs(20));
    let job = app.insert_job("keynote-2026", spec);
    {
        let mut st = job.state();
        for k in (0..41).rev() {
            st.apply(
                Event::Waiting {
                    reason: format!("stream is not live yet: HTTP 404 for {url}"),
                    retry_in: Duration::from_secs(20),
                },
                ago(now, 7.0 + 20.0 * k as f64),
            );
        }
    }

    let url = "https://vod.university.example/cs/lec07/index.m3u8";
    let mut spec = app.default_spec(url, "lecture-series-ep07");
    spec.concurrency = 8;
    let job = app.insert_job("lecture-series-ep07", spec);
    {
        let mut st = job.state();
        let t0 = ago(now, 40.0);
        st.backdate(ago(now, 300.0));
        planned(
            &mut st,
            t0,
            Protocol::Hls,
            &[("main", "media playlist, 1204 segments", 1204, false)],
        );
        done(&mut st, "main", 0..812, 1_100_000, (t0, t0), true);
        done(
            &mut st,
            "main",
            812..1042,
            1_000_000,
            (ago(now, 17.0), now),
            false,
        );
        in_flight(&mut st, "main", 1042..1050, now);
    }

    let url = "https://media.blender-mirror.example/sintel/sintel-4k.mpd";
    let mut spec = app.default_spec(url, "sintel-4k");
    spec.concurrency = 12;
    let job = app.insert_job("sintel-4k", spec);
    {
        let mut st = job.state();
        let t0 = ago(now, 40.0);
        st.backdate(ago(now, 300.0));
        planned(
            &mut st,
            t0,
            Protocol::Dash,
            &[
                (
                    "video",
                    "v4k 3840x1636 @ 14000 kbps [hev1.2.4.L153]",
                    1462,
                    false,
                ),
                ("audio", "a-en @ 128 kbps [mp4a.40.2] (en)", 1462, false),
            ],
        );
        done(&mut st, "video", 0..921, 2_200_000, (t0, now), false);
        done(&mut st, "audio", 0..921, 20_000, (t0, now), false);
        in_flight(&mut st, "video", 921..927, now);
        in_flight(&mut st, "audio", 921..927, now);
    }

    let url = "https://stream.cityhall.example/council/live/master.m3u8";
    let spec = app.default_spec(url, "city-council-live");
    let job = app.insert_job("city-council-live", spec);
    {
        let mut st = job.state();
        let t0 = ago(now, 4364.0);
        st.backdate(ago(now, 300.0));
        planned(
            &mut st,
            t0,
            Protocol::Hls,
            &[
                (
                    "video",
                    "1920x1080 @ 5900 kbps [avc1.640028,mp4a.40.2]",
                    2182,
                    true,
                ),
                ("audio", "English (en)", 2182, true),
            ],
        );
        done(&mut st, "video", 0..2181, 1_516_000, (t0, now), false);
        done(&mut st, "audio", 0..2181, 32_000, (t0, now), false);
        in_flight(&mut st, "video", 2181..2182, now);
        in_flight(&mut st, "audio", 2181..2182, now);
    }

    // A probed stream on the New download page.
    let url = "https://cdn.openfilms.example/tears-of-steel/master.m3u8";
    let variant = |id: &str, w: u64, h: u64, mbit: f64, codecs: &str| VariantInfo {
        id: id.into(),
        bandwidth: (mbit * 1e6) as u64,
        resolution: Some((w, h)),
        codecs: Some(codecs.into()),
        frame_rate: Some(24.0),
        audio_group: Some("aud".into()),
    };
    let audio = |name: &str, lang: &str, default: bool| AudioInfo {
        id: name.into(),
        group: "aud".into(),
        name: name.into(),
        language: Some(lang.into()),
        default,
        bandwidth: None,
        codecs: None,
    };
    let info = StreamInfo {
        protocol: Protocol::Hls,
        live: false,
        variants: vec![
            variant("v0", 3840, 2160, 15.2, "avc1.640033,mp4a.40.2"),
            variant("v1", 2560, 1440, 9.8, "avc1.640032,mp4a.40.2"),
            variant("v2", 1920, 1080, 6.0, "avc1.640028,mp4a.40.2"),
            variant("v3", 1280, 720, 3.2, "avc1.64001f,mp4a.40.2"),
            variant("v4", 854, 480, 1.4, "avc1.64001e,mp4a.40.2"),
            variant("v5", 640, 360, 0.8, "avc1.42c01e,mp4a.40.2"),
        ],
        audio: vec![
            audio("English", "en", true),
            audio("Deutsch", "de", false),
            audio("Español", "es", false),
            audio("Svenska", "sv", false),
        ],
        duration: Some(734.0),
        segments: Some(184),
        encrypted: true,
        drm: false,
    };
    let mut draft = Draft::probing(url.into(), &app.settings);
    draft.set_result(Ok((info, Duration::from_millis(412))), &app.settings);
    draft.quality = QualityPref::MaxHeight(1080);
    draft.container = Container::Mkv;
    draft.stem = "tears-of-steel-1080p".into();
    app.draft = Some(draft);
    app.url = url.into();
    app.probed = Some((url.into(), Duration::from_millis(412)));

    app.settings.headers = vec![
        Header {
            enabled: true,
            name: "Referer".into(),
            value: "https://cityhall.example/".into(),
        },
        Header {
            enabled: true,
            name: "User-Agent".into(),
            value: "collider/0.1.0 (+https://github.com/dnlsplnd/collider)".into(),
        },
        Header {
            enabled: false,
            name: "Cookie".into(),
            value: "session=a1b2c3".into(),
        },
    ];

    let finished = chrono::Local::now().timestamp();
    app.history.records = vec![
        Record {
            name: "board-meeting-2026-09-12".into(),
            url: "https://stream.cityhall.example/council/archive/0912.m3u8".into(),
            outcome: Outcome::Completed,
            files: vec![app.settings.output_dir.join("board-meeting-2026-09-12.mp4")],
            bytes: 1_893_000_000,
            live: true,
            finished: finished - 86_400 * 3,
            detail: String::new(),
        },
        Record {
            name: "premium-channel".into(),
            url: "https://tv.provider.example/premium-channel.m3u8".into(),
            outcome: Outcome::Unsupported,
            files: vec![],
            bytes: 0,
            live: false,
            finished: finished - 3600,
            detail: "encryption method SampleAes: only clear-key AES-128 is supported".into(),
        },
        Record {
            name: "open-movie-1080p".into(),
            url: "https://cdn.openfilms.example/open-movie/master.m3u8".into(),
            outcome: Outcome::Completed,
            files: vec![app.settings.output_dir.join("open-movie-1080p.mkv")],
            bytes: 2_577_000_000,
            live: false,
            finished: finished - 600,
            detail: String::new(),
        },
    ];
}

/// Render every page of the demo scene to PNG, at the mockup size and at the minimum.
/// Needs a GPU (wgpu), so it is ignored by default:
/// `COLLIDER_GUI_SHOTS=dir cargo test -p collider-gui render_pages -- --ignored`
#[test]
#[ignore = "renders with wgpu; run with --ignored where a GPU is available"]
fn render_pages() {
    use egui_kittest::Harness;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = std::env::var_os("COLLIDER_GUI_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/shots")
        });
    std::fs::create_dir_all(&dir).unwrap();
    for (w, h, suffix) in [(1440.0, 900.0, ""), (1100.0, 700.0, "-min")] {
        let handle = rt.handle().clone();
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(w, h))
            .with_pixels_per_point(1.0)
            .build_eframe(|cc| {
                let mut app = App::new(&cc.egui_ctx, handle, Storage::default());
                scene(&mut app);
                app
            });
        let live = harness
            .state()
            .jobs
            .iter()
            .find(|j| j.name == "city-council-live")
            .map(|j| j.id)
            .unwrap();
        for (page, name) in [
            (Page::Downloads, "downloads"),
            (Page::NewDownload, "new-download"),
            (Page::Job(live), "details"),
            (Page::Settings, "settings"),
            (Page::History, "history"),
            (Page::Watch, "watch"),
        ] {
            harness.state_mut().page = page;
            harness.run_steps(4);
            let image = harness.render().expect("render");
            image.save(dir.join(format!("{name}{suffix}.png"))).unwrap();
        }
    }
}
