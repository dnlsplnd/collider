//! End-to-end: drive the app like a user, against a local HLS stream made by ffmpeg.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::{App, Page, Storage};
use crate::history::Outcome;
use crate::jobs::Phase;

/// Serve the files under `root` over HTTP/1.1 on 127.0.0.1.
fn serve(root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let root = root.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let file = root.join(path.trim_start_matches('/'));
                match std::fs::read(&file) {
                    Ok(body) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(&body);
                    }
                    Err(_) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

/// A 3 s HLS VOD under `dir`; false when ffmpeg is not installed.
fn fixture(dir: &Path) -> bool {
    std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440", "-t", "3"])
        .args([
            "-c:v",
            "mpeg4",
            "-g",
            "25",
            "-bsf:v",
            "dump_extra",
            "-c:a",
            "aac",
        ])
        .args(["-f", "hls", "-hls_time", "1", "-hls_playlist_type", "vod"])
        .arg("-hls_segment_filename")
        .arg(dir.join("seg%02d.ts"))
        .arg(dir.join("index.m3u8"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct Setup {
    tmp: tempfile::TempDir,
    url: String,
    rt: tokio::runtime::Runtime,
}

fn setup() -> Option<Setup> {
    let tmp = tempfile::tempdir().unwrap();
    let show = tmp.path().join("www").join("show");
    std::fs::create_dir_all(&show).unwrap();
    if !fixture(&show) {
        eprintln!("skipping: ffmpeg is not available");
        return None;
    }
    let base = serve(tmp.path().join("www"));
    Some(Setup {
        url: format!("{base}/show/index.m3u8"),
        tmp,
        rt: tokio::runtime::Runtime::new().unwrap(),
    })
}

fn harness(s: &Setup) -> Harness<'static, App> {
    let storage = Storage {
        settings: Some(s.tmp.path().join("gui.toml")),
        history: Some(s.tmp.path().join("history.json")),
    };
    let handle = s.rt.handle().clone();
    let out = s.tmp.path().join("out");
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = App::new(&cc.egui_ctx, handle, storage);
            app.settings.output_dir = out;
            app
        });
    h.run_steps(2);
    h
}

/// Step the UI until `done` holds, letting real time pass for the download.
fn run_until(h: &mut Harness<'static, App>, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !done(h.state()) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn type_url(h: &mut Harness<'static, App>, url: &str) {
    h.get_by_role(Role::TextInput).focus();
    h.step();
    h.get_by_role(Role::TextInput).type_text(url);
    h.step();
    assert_eq!(h.state().url, url);
}

fn completed(app: &App) -> Option<Vec<PathBuf>> {
    app.jobs.first().and_then(|j| match &j.state().phase {
        Phase::Completed { files } => Some(files.clone()),
        Phase::Failed { message } => panic!("download failed: {message}"),
        _ => None,
    })
}

#[test]
fn probe_then_start_download_through_the_ui() {
    let Some(s) = setup() else { return };
    let mut h = harness(&s);
    type_url(&mut h, &s.url);

    h.get_by_label("Probe").click();
    h.step();
    assert_eq!(h.state().page, Page::NewDownload);
    run_until(&mut h, "the probe", |app| {
        app.draft.as_ref().is_some_and(|d| d.result.is_some())
    });
    let info = match &h.state().draft.as_ref().unwrap().result {
        Some(Ok((info, _))) => info.clone(),
        other => panic!("probe failed: {other:?}"),
    };
    assert!(!info.live && info.segments == Some(3), "{info:?}");

    h.run_steps(2);
    h.get_by_label("Start download").click();
    h.step();
    assert_eq!(h.state().page, Page::Downloads);
    run_until(&mut h, "the download", |app| completed(app).is_some());

    let files = completed(h.state()).unwrap();
    assert_eq!(files, [s.tmp.path().join("out").join("show.mp4")]);
    assert!(std::fs::metadata(&files[0]).unwrap().len() > 10_000);
    // The work directory is gone and the job is in the saved history.
    assert!(!s.tmp.path().join("out").join("show.mp4.parts").exists());
    h.step();
    let history = crate::history::History::load_from(&s.tmp.path().join("history.json"));
    assert_eq!(history.records.len(), 1);
    assert_eq!(history.records[0].outcome, Outcome::Completed);
    assert_eq!(history.records[0].name, "show");
}

#[test]
fn add_downloads_without_probing_and_never_overwrites() {
    let Some(s) = setup() else { return };
    let mut h = harness(&s);
    type_url(&mut h, &s.url);
    h.get_by_label("Add").click();
    h.step();
    run_until(&mut h, "the first download", |app| completed(app).is_some());

    // The same link again gets a new name instead of replacing the first file.
    type_url(&mut h, &s.url);
    h.get_by_label("Add").click();
    h.step();
    run_until(&mut h, "the second download", |app| {
        app.jobs.len() == 2 && matches!(app.jobs[1].state().phase, Phase::Completed { .. })
    });
    let out = s.tmp.path().join("out");
    assert!(out.join("show.mp4").exists());
    assert!(out.join("show-1.mp4").exists());

    // Settings changed in the app are saved to the given file, not the user's.
    // (Saving is debounced, so wait for the new value to arrive.)
    h.state_mut().settings.retries = 2;
    let path = s.tmp.path().join("gui.toml");
    let deadline = Instant::now() + Duration::from_secs(5);
    while crate::settings::Settings::load_from(&path).retries != 2 {
        assert!(Instant::now() < deadline, "settings were not saved");
        h.step();
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// ffmpeg publishing a live HLS stream in real time; killed when dropped.
struct LiveStream(std::process::Child);

impl Drop for LiveStream {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn record_live_and_stop_and_save() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("www").join("live");
    std::fs::create_dir_all(&dir).unwrap();
    let child = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-re"])
        .args(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25"])
        .args(["-f", "lavfi", "-i", "sine=frequency=550", "-t", "40"])
        .args([
            "-c:v",
            "mpeg4",
            "-g",
            "25",
            "-bsf:v",
            "dump_extra",
            "-c:a",
            "aac",
        ])
        .args(["-f", "hls", "-hls_time", "1", "-hls_list_size", "6"])
        .args(["-hls_flags", "delete_segments", "-hls_segment_filename"])
        .arg(dir.join("s%04d.ts"))
        .arg(dir.join("index.m3u8"))
        .stdin(std::process::Stdio::null())
        .spawn();
    let Ok(child) = child else {
        eprintln!("skipping: ffmpeg is not available");
        return;
    };
    let _live = LiveStream(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    while !dir.join("index.m3u8").exists() {
        assert!(
            Instant::now() < deadline,
            "ffmpeg did not start the live stream"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let s = Setup {
        url: format!("{}/live/index.m3u8", serve(tmp.path().join("www"))),
        tmp,
        rt: tokio::runtime::Runtime::new().unwrap(),
    };
    let mut h = harness(&s);
    type_url(&mut h, &s.url);
    h.get_by_label("Add").click();
    h.step();
    run_until(&mut h, "four recorded segments", |app| {
        let st = app.jobs[0].state();
        st.live && st.phase == Phase::Running && st.done() >= 4
    });
    h.run_steps(2);
    h.get_by_label("Stop and save").click();
    h.step();
    run_until(&mut h, "the saved recording", |app| {
        completed(app).is_some()
    });
    let files = completed(h.state()).unwrap();
    assert_eq!(files.len(), 1);
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(&files[0])
        .output()
        .unwrap();
    let secs: f64 = String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse()
        .unwrap();
    assert!(secs >= 3.0, "recorded only {secs} s");
    let st = h.state().jobs[0].state();
    assert!(st.live);
    assert_eq!(st.gaps(), 0);
}
