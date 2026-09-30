//! Lossless remuxing via ffmpeg (stream copy, never re-encodes).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::error::{Error, Result};

pub async fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Mux one or more elementary/container inputs into `output` with `-c copy`.
/// Only video and audio streams are mapped, which drops timed-ID3 data tracks
/// that MP4 cannot hold.
///
/// ffmpeg rebases every input to start at zero, which would throw away the offset between
/// separately captured tracks. Each input is therefore shifted back by its start time
/// relative to the earliest one, when all of them carry timestamps.
pub async fn remux(inputs: &[PathBuf], output: &Path) -> Result<()> {
    let offsets = if inputs.len() > 1 {
        let mut starts = Vec::with_capacity(inputs.len());
        for input in inputs {
            starts.push(start_time(input).await);
        }
        alignment(&starts)
    } else {
        None
    };
    if let Some(o) = &offsets {
        tracing::debug!(?o, "aligning inputs by start time");
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for (i, input) in inputs.iter().enumerate() {
        if let Some(offset) = offsets.as_ref().map(|o| o[i]).filter(|o| *o > 0.0) {
            cmd.args(["-itsoffset", &format!("{offset:.6}")]);
        }
        cmd.arg("-i").arg(input);
    }
    for i in 0..inputs.len() {
        cmd.args(["-map", &format!("{i}:v?"), "-map", &format!("{i}:a?")]);
    }
    cmd.args(["-c", "copy"]);
    let ext = extension(output);
    if matches!(ext.as_str(), "mp4" | "m4a" | "m4v" | "mov") {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(output);
    run(cmd).await
}

/// Join the pieces of one track, each with its own timeline (DASH periods, HLS
/// discontinuities), into `output` with the concat demuxer, which re-times every piece to
/// follow the previous one. The pieces must share a directory. Stream copy only.
pub async fn concat(pieces: &[PathBuf], output: &Path) -> Result<()> {
    let Some(first) = pieces.first() else {
        return Err(Error::Mux("nothing to concatenate".into()));
    };
    let list = first.with_extension("concat");
    let mut text = String::from("ffconcat version 1.0\n");
    for p in pieces {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        // Paths in the list resolve against the list's directory; quote for ffconcat.
        text.push_str(&format!("file '{}'\n", name.replace('\'', r"'\''")));
    }
    tokio::fs::write(&list, text).await?;
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "concat", "-safe", "0", "-i"])
        .arg(&list)
        .args(["-map", "0:v?", "-map", "0:a?", "-c", "copy"])
        .arg(output);
    let result = run(cmd).await;
    tokio::fs::remove_file(&list).await.ok();
    result
}

async fn run(mut cmd: Command) -> Result<()> {
    let out = cmd.stdin(Stdio::null()).output().await?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Mux(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The first timestamp of `input` in seconds, or `None` when it is unknown: ffprobe is
/// missing, or the input is an elementary stream (ADTS, MP3, AC-3) whose reported start of
/// zero says nothing about how it lines up with the other tracks.
async fn start_time(input: &Path) -> Option<f64> {
    if matches!(extension(input).as_str(), "aac" | "mp3" | "ac3" | "eac3") {
        return None;
    }
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=start_time"])
        .args(["-of", "csv=p=0"])
        .arg(input)
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|t| t.is_finite())
}

/// Per-input offsets from the earliest start, or `None` unless every start is known.
fn alignment(starts: &[Option<f64>]) -> Option<Vec<f64>> {
    let starts: Vec<f64> = starts.iter().copied().collect::<Option<_>>()?;
    let min = starts.iter().copied().fold(f64::INFINITY, f64::min);
    Some(starts.iter().map(|s| s - min).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_is_relative_to_the_earliest_input() {
        assert_eq!(alignment(&[Some(10.0), Some(12.5)]), Some(vec![0.0, 2.5]));
        assert_eq!(alignment(&[Some(3.0), Some(1.0)]), Some(vec![2.0, 0.0]));
        assert_eq!(alignment(&[Some(10.0), None]), None);
    }

    fn ffmpeg(args: &[&str]) -> bool {
        std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn stream_start(file: &Path, stream: &str) -> f64 {
        let out = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", stream])
            .args(["-show_entries", "stream=start_time", "-of", "csv=p=0"])
            .arg(file)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    /// Video captured from t=10 s and audio from t=12 s must stay 2 s apart after the remux.
    #[tokio::test]
    async fn separately_captured_tracks_keep_their_offset() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path().join("video.ts");
        let a = tmp.path().join("audio.ts");
        let out = tmp.path().join("out.mkv");
        let video = "testsrc2=size=160x90:rate=10:duration=4";
        let built = ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            video,
            "-c:v",
            "mpeg4",
            "-output_ts_offset",
            "10",
            v.to_str().unwrap(),
        ]) && ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "sine=duration=2",
            "-c:a",
            "aac",
            "-output_ts_offset",
            "12",
            a.to_str().unwrap(),
        ]);
        if !built {
            eprintln!("skipping: ffmpeg cannot build the fixtures");
            return;
        }
        remux(&[v, a], &out).await.unwrap();
        let gap = stream_start(&out, "a:0") - stream_start(&out, "v:0");
        assert!(
            (gap - 2.0).abs() < 0.1,
            "audio starts {gap:.3} s after video"
        );
    }
}
