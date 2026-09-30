//! Protocol-neutral description of a stream and its segments.

use std::time::Duration;

use serde::Serialize;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Hls,
    Dash,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Protocol::Hls => "HLS",
            Protocol::Dash => "DASH",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Video,
    Audio,
    Other,
}

/// Clear-key AES-128 (HLS only).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub url: Url,
    pub iv: [u8; 16],
}

/// Initialization section (`EXT-X-MAP` / DASH init segment), `byte_range` is `(offset, length)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InitSection {
    pub url: Url,
    pub byte_range: Option<(u64, u64)>,
    /// HLS only: the AES-128 key (with explicit IV) in force at the `EXT-X-MAP` tag.
    pub key: Option<Key>,
}

impl InitSection {
    /// Stable file name for the on-disk copy of this init section.
    pub fn file_name(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        format!("init-{:016x}.seg", h.finish())
    }
}

/// Segment seqs carry a period (timeline) ordinal above this bit; see [`Segment::period`].
pub const PERIOD_SHIFT: u32 = 40;

#[derive(Debug, Clone)]
pub struct Segment {
    /// Unique id within a track (used for resume and dedup). DASH packs the period ordinal
    /// into the bits above [`PERIOD_SHIFT`]; HLS seqs stay below it.
    pub seq: u64,
    pub url: Url,
    pub duration: f64,
    /// `(offset, length)`
    pub byte_range: Option<(u64, u64)>,
    pub key: Option<Key>,
    pub init: Option<InitSection>,
    /// The manifest only implies this segment may exist (DASH: the truncated last segment
    /// derived from a period duration), so a 404/410/416 means "end of period", not an error.
    pub optional: bool,
    /// HLS `EXT-X-DISCONTINUITY`: timestamps may restart at this segment.
    pub discontinuity: bool,
}

impl Segment {
    /// The DASH period this segment belongs to (always 0 for HLS).
    pub fn period(&self) -> u64 {
        self.seq >> PERIOD_SHIFT
    }
}

/// One view of a track's playlist. Live tracks return new segments on later refreshes.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub segments: Vec<Segment>,
    /// The track is complete: no further refresh will add segments.
    pub ended: bool,
    /// Suggested delay before the next refresh of a live track.
    pub refresh_after: Duration,
}

/// What a manifest offers, for `collider info` and track selection.
#[derive(Debug, Clone, Serialize)]
pub struct StreamInfo {
    pub protocol: Protocol,
    pub live: bool,
    pub variants: Vec<VariantInfo>,
    pub audio: Vec<AudioInfo>,
    /// Total media duration when known.
    pub duration: Option<f64>,
    /// Segment count when the manifest lists them directly.
    pub segments: Option<usize>,
    pub encrypted: bool,
    /// The manifest uses DRM; downloading is refused.
    pub drm: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantInfo {
    pub id: String,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
    pub codecs: Option<String>,
    pub frame_rate: Option<f64>,
    pub audio_group: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioInfo {
    pub id: String,
    pub group: String,
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
    pub bandwidth: Option<u64>,
    pub codecs: Option<String>,
}

/// Parse a DASH-style `"start-end"` byte range into `(offset, length)`.
pub fn parse_range(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.trim().split_once('-')?;
    let start: u64 = a.trim().parse().ok()?;
    let end: u64 = b.trim().parse().ok()?;
    (end >= start).then(|| (start, end - start + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("0-999"), Some((0, 1000)));
        assert_eq!(parse_range("1000-1000"), Some((1000, 1)));
        assert_eq!(parse_range("5-1"), None);
        assert_eq!(parse_range("x"), None);
    }

    #[test]
    fn init_file_names_are_stable_and_distinct() {
        let a = InitSection {
            url: Url::parse("https://x/init.mp4").unwrap(),
            byte_range: None,
            key: None,
        };
        let b = InitSection {
            url: Url::parse("https://x/init.mp4").unwrap(),
            byte_range: Some((0, 10)),
            key: None,
        };
        assert_eq!(a.file_name(), a.file_name());
        assert_ne!(a.file_name(), b.file_name());
    }
}
