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
pub async fn remux(inputs: &[PathBuf], output: &Path) -> Result<()> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in inputs {
        cmd.arg("-i").arg(input);
    }
    for i in 0..inputs.len() {
        cmd.args(["-map", &format!("{i}:v?"), "-map", &format!("{i}:a?")]);
    }
    cmd.args(["-c", "copy"]);
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "mp4" | "m4a" | "m4v" | "mov") {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(output);
    let out = cmd.stdin(Stdio::null()).output().await?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Mux(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}
