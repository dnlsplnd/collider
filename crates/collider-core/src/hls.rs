//! HLS playlist model: parsing, URL resolution, variant selection and key/IV handling.
//!
//! Only clear-key AES-128 (part of the HLS spec, keys served over plain HTTP) is supported.
//! DRM schemes (SAMPLE-AES with FairPlay/Widevine/PlayReady key formats) are deliberately
//! rejected: collider does not circumvent technical protection measures.

use std::str::FromStr;

use m3u8_rs::{AlternativeMediaType, KeyMethod, Playlist};
use url::Url;

use crate::error::{Error, Result};
use crate::http::Client;
use crate::model::{
    AudioInfo, InitSection, Key, Protocol, Segment, Snapshot, StreamInfo, VariantInfo,
};

#[derive(Debug, Clone)]
pub struct Variant {
    pub url: Url,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
    pub codecs: Option<String>,
    pub frame_rate: Option<f64>,
    pub audio_group: Option<String>,
}

impl Variant {
    pub fn height(&self) -> u64 {
        self.resolution.map(|(_, h)| h).unwrap_or(0)
    }

    /// An audio-only variant (e.g. `CODECS="mp4a.40.2"` without `RESOLUTION`), as Apple
    /// recommends offering for poor connections.
    pub fn is_audio_only(&self) -> bool {
        const VIDEO: [&str; 14] = [
            "avc1", "avc3", "hvc1", "hev1", "av01", "vp08", "vp09", "vp8", "vp9", "dvh1", "dvhe",
            "dva1", "dvav", "mp4v",
        ];
        self.resolution.is_none()
            && self.codecs.as_deref().is_some_and(|c| {
                !c.split(',').any(|codec| {
                    let fourcc = codec.trim().split('.').next().unwrap_or_default();
                    VIDEO.iter().any(|v| fourcc.eq_ignore_ascii_case(v))
                })
            })
    }
}

#[derive(Debug, Clone)]
pub struct AudioRendition {
    pub url: Url,
    pub group_id: String,
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
}

#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub segments: Vec<Segment>,
    pub target_duration: f64,
    /// `#EXT-X-ENDLIST` present: the playlist is complete (VOD or finished live event).
    pub ended: bool,
    /// `#EXT-X-MEDIA-SEQUENCE` of the first segment, as served.
    pub media_sequence: u64,
}

#[derive(Debug, Clone)]
pub enum Manifest {
    Master {
        variants: Vec<Variant>,
        audio: Vec<AudioRendition>,
    },
    Media(MediaInfo),
}

pub fn parse(base: &Url, bytes: &[u8]) -> Result<Manifest> {
    let text = normalize(bytes);
    match m3u8_rs::parse_playlist_res(&text) {
        Ok(Playlist::MasterPlaylist(m)) => {
            // m3u8-rs demotes an EXT-X-STREAM-INF it cannot parse to an unknown tag and then
            // assigns the following URI to the *previous* variant: the table would be shifted.
            if let Some(t) = m.unknown_tags.iter().find(|t| t.tag == "X-STREAM-INF") {
                return Err(Error::Parse(format!(
                    "unparseable EXT-X-STREAM-INF: {}",
                    t.rest.as_deref().unwrap_or_default()
                )));
            }
            let variants = m
                .variants
                .into_iter()
                .filter(|v| !v.is_i_frame)
                .map(|v| {
                    Ok(Variant {
                        url: base.join(&v.uri)?,
                        bandwidth: v.bandwidth,
                        resolution: v.resolution.map(|r| (r.width, r.height)),
                        codecs: v.codecs,
                        frame_rate: v.frame_rate,
                        audio_group: v.audio,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let audio = m
                .alternatives
                .into_iter()
                .filter(|a| a.media_type == AlternativeMediaType::Audio)
                .filter_map(|a| {
                    let uri = a.uri?;
                    Some(base.join(&uri).map(|url| AudioRendition {
                        url,
                        group_id: a.group_id,
                        name: a.name,
                        language: a.language,
                        default: a.default,
                    }))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(Manifest::Master { variants, audio })
        }
        Ok(Playlist::MediaPlaylist(p)) => Ok(Manifest::Media(convert_media(base, p)?)),
        Err(e) => Err(Error::Parse(format!("{e:?}").chars().take(200).collect())),
    }
}

/// Rewrite the playlist text so m3u8-rs accepts common, harmless deviations instead of
/// silently mis-parsing them:
/// - a leading UTF-8 BOM and trailing whitespace on each line are dropped;
/// - a fractional `EXT-X-TARGETDURATION` is rounded up (m3u8-rs reads only the integer part
///   and turns the leftover `.0` into a phantom segment, shifting every sequence number);
/// - `EXT-X-KEY` attribute lists are re-emitted with the quoting m3u8-rs requires
///   (`METHOD`/`IV` unquoted, `URI`/`KEYFORMAT`/`KEYFORMATVERSIONS` quoted), and the valid
///   `METHOD=NONE` without an IV gets a dummy IV that m3u8-rs 6 wrongly insists on.
///   Otherwise such keys become unknown tags and lose their position in the playlist.
fn normalize(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = String::with_capacity(text.len() + 16);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line = line.trim_end();
        if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            if let Some(n) = v
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n >= 0.0)
            {
                out.push_str(&format!("#EXT-X-TARGETDURATION:{}", n.ceil() as u64));
                continue;
            }
        }
        if let Some(attrs) = line.strip_prefix("#EXT-X-KEY:") {
            if let Some(attrs) = normalize_key_attributes(attrs) {
                out.push_str("#EXT-X-KEY:");
                out.push_str(&attrs);
                continue;
            }
        }
        out.push_str(line);
    }
    out.into_bytes()
}

/// Parse an attribute list leniently (either quoting for any value) into `(name, value, quoted)`.
fn attribute_list(s: &str) -> Option<Vec<(&str, &str, bool)>> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let (name, after) = rest.split_once('=')?;
        let after = after.trim_start();
        let (value, quoted, tail) = match after.strip_prefix('"') {
            Some(q) => {
                let end = q.find('"')?;
                (&q[..end], true, &q[end + 1..])
            }
            None => {
                let end = after.find(',').unwrap_or(after.len());
                (after[..end].trim(), false, &after[end..])
            }
        };
        let tail = tail.trim_start();
        rest = match tail.strip_prefix(',') {
            Some(t) => t.trim_start(),
            None if tail.is_empty() => tail,
            None => return None,
        };
        out.push((name.trim(), value, quoted));
    }
    Some(out)
}

fn normalize_key_attributes(attrs: &str) -> Option<String> {
    let list = attribute_list(attrs)?;
    let mut out = Vec::with_capacity(list.len() + 1);
    for (name, value, quoted) in &list {
        out.push(match *name {
            "METHOD" | "IV" => format!("{name}={}", value.trim()),
            "URI" | "KEYFORMAT" | "KEYFORMATVERSIONS" => format!("{name}=\"{value}\""),
            _ if *quoted => format!("{name}=\"{value}\""),
            _ => format!("{name}={value}"),
        });
    }
    let none = list
        .iter()
        .any(|(n, v, _)| *n == "METHOD" && v.trim() == "NONE");
    if none && !list.iter().any(|(n, _, _)| *n == "IV") {
        out.push("IV=0x0".into());
    }
    Some(out.join(","))
}

fn convert_media(base: &Url, pl: m3u8_rs::MediaPlaylist) -> Result<MediaInfo> {
    // EXT-X-KEY and EXT-X-MAP apply to every following segment until replaced;
    // m3u8-rs only attaches them to the segment they precede, so carry them forward.
    let mut key: Option<m3u8_rs::Key> = None;
    let mut init: Option<InitSection> = None;
    let mut prev_range_end: Option<(Url, u64)> = None;
    let mut segments = Vec::with_capacity(pl.segments.len());

    for (i, s) in pl.segments.into_iter().enumerate() {
        let seq = pl.media_sequence + i as u64;
        // Tags m3u8-rs could not parse (even after `normalize`) must not be ignored: a lost
        // EXT-X-KEY would write ciphertext as output, a lost EXT-X-MAP an unplayable file.
        if let Some(t) = s
            .unknown_tags
            .iter()
            .find(|t| t.tag == "X-KEY" || t.tag == "X-MAP")
        {
            return Err(Error::Parse(format!(
                "unparseable EXT-{}: {}",
                t.tag,
                t.rest.as_deref().unwrap_or_default()
            )));
        }
        if let Some(k) = s.key {
            key = if matches!(k.method, KeyMethod::None) {
                None
            } else {
                Some(k)
            };
        }
        let url = base.join(&s.uri)?;
        let byte_range = s.byte_range.map(|r| {
            let offset = r.offset.unwrap_or_else(|| match &prev_range_end {
                Some((u, end)) if *u == url => *end,
                _ => 0,
            });
            prev_range_end = Some((url.clone(), offset + r.length));
            (offset, r.length)
        });
        let seg_key = match &key {
            None => None,
            Some(k) => match &k.method {
                KeyMethod::AES128 => {
                    let uri = k.uri.as_deref().ok_or_else(|| {
                        Error::Parse("EXT-X-KEY AES-128 without URI".into())
                    })?;
                    let iv = match &k.iv {
                        Some(iv) => parse_iv(iv)?,
                        None => iv_from_sequence(seq),
                    };
                    Some(Key { url: base.join(uri)?, iv })
                }
                other => {
                    return Err(Error::Unsupported(format!(
                        "encryption method {other:?}: only clear-key AES-128 is supported (DRM is out of scope)"
                    )))
                }
            },
        };
        if let Some(m) = s.map {
            // RFC 8216 §4.3.2.5: an init section may be encrypted by the EXT-X-KEY in force,
            // which must then carry an explicit IV. m3u8-rs does not record whether the KEY
            // came before or after the MAP, so without an IV the init is taken as plaintext,
            // and the downloader only decrypts it when it does not already look plaintext.
            let init_key = match &key {
                Some(k) if k.iv.is_some() => seg_key.clone(),
                _ => None,
            };
            init = Some(InitSection {
                url: base.join(&m.uri)?,
                byte_range: m.byte_range.map(|r| (r.offset.unwrap_or(0), r.length)),
                key: init_key,
            });
        }
        segments.push(Segment {
            seq,
            url,
            duration: s.duration as f64,
            byte_range,
            key: seg_key,
            init: init.clone(),
            optional: false,
            discontinuity: s.discontinuity,
        });
    }

    Ok(MediaInfo {
        segments,
        target_duration: pl.target_duration as f64,
        ended: pl.end_list,
        media_sequence: pl.media_sequence,
    })
}

/// Parse a `0x...` hex IV into 16 bytes (left-padded if shorter).
pub fn parse_iv(s: &str) -> Result<[u8; 16]> {
    let hex_str = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    let padded = format!("{hex_str:0>32}");
    let bytes = hex::decode(&padded).map_err(|e| Error::Parse(format!("bad IV '{s}': {e}")))?;
    bytes
        .try_into()
        .map_err(|_| Error::Parse(format!("IV '{s}' is longer than 128 bits")))
}

/// Default IV per RFC 8216 §5.2: the media sequence number as a big-endian 128-bit integer.
pub fn iv_from_sequence(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quality {
    Best,
    Worst,
    /// Best variant whose height is at most this many pixels.
    MaxHeight(u64),
}

impl FromStr for Quality {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "best" => Ok(Self::Best),
            "worst" => Ok(Self::Worst),
            other => other
                .trim_end_matches('p')
                .parse()
                .map(Self::MaxHeight)
                .map_err(|_| Error::NoVariant(s.to_string())),
        }
    }
}

pub fn select_variant<'a>(variants: &'a [Variant], q: &Quality) -> Result<&'a Variant> {
    let rank = |v: &&Variant| (v.height(), v.bandwidth);
    // Audio-only variants have no height and would otherwise rank lowest: never pick one
    // as the video track when the master offers any video.
    let has_video = variants.iter().any(|v| !v.is_audio_only());
    let candidates = || {
        variants
            .iter()
            .filter(move |v| !has_video || !v.is_audio_only())
    };
    let pick = match q {
        Quality::Best => candidates().max_by_key(rank),
        Quality::Worst => candidates().min_by_key(rank),
        Quality::MaxHeight(h) => candidates().filter(|v| v.height() <= *h).max_by_key(rank),
    };
    pick.ok_or_else(|| Error::NoVariant(format!("{q:?}")))
}

pub fn select_audio<'a>(
    renditions: &'a [AudioRendition],
    group: &str,
    lang: Option<&str>,
) -> Option<&'a AudioRendition> {
    let in_group: Vec<_> = renditions.iter().filter(|a| a.group_id == group).collect();
    lang.and_then(|l| {
        in_group.iter().copied().find(|a| {
            a.language
                .as_deref()
                .is_some_and(|x| x.eq_ignore_ascii_case(l))
                || a.name.eq_ignore_ascii_case(l)
        })
    })
    .or_else(|| in_group.iter().copied().find(|a| a.default))
    .or_else(|| in_group.first().copied())
}

pub fn describe(m: &Manifest) -> StreamInfo {
    match m {
        Manifest::Master { variants, audio } => StreamInfo {
            protocol: Protocol::Hls,
            live: false,
            variants: variants
                .iter()
                .enumerate()
                .map(|(i, v)| VariantInfo {
                    id: v
                        .url
                        .path_segments()
                        .and_then(|mut p| p.next_back())
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| i.to_string()),
                    bandwidth: v.bandwidth,
                    resolution: v.resolution,
                    codecs: v.codecs.clone(),
                    frame_rate: v.frame_rate,
                    audio_group: v.audio_group.clone(),
                })
                .collect(),
            audio: audio
                .iter()
                .map(|a| AudioInfo {
                    id: a.name.clone(),
                    group: a.group_id.clone(),
                    name: a.name.clone(),
                    language: a.language.clone(),
                    default: a.default,
                    bandwidth: None,
                    codecs: None,
                })
                .collect(),
            duration: None,
            segments: None,
            encrypted: false,
            drm: false,
        },
        Manifest::Media(m) => StreamInfo {
            protocol: Protocol::Hls,
            live: !m.ended,
            variants: Vec::new(),
            audio: Vec::new(),
            duration: Some(m.segments.iter().map(|s| s.duration).sum()),
            segments: Some(m.segments.len()),
            encrypted: m.segments.iter().any(|s| s.key.is_some()),
            drm: false,
        },
    }
}

/// Segment source for one HLS media playlist (VOD or live).
pub struct HlsSource {
    client: Client,
    url: Url,
    first: Option<MediaInfo>,
    epoch: Epoch,
}

/// Keeps `Segment::seq` unique across a media-sequence reset (encoder restart).
///
/// Sequence numbers must never decrease (RFC 8216 §6.2.1), but restarted encoders commonly
/// start again at 0. Without renumbering, the downloader would take the new segments for
/// ones it already has and silently drop them.
#[derive(Default)]
struct Epoch {
    /// Added to every served sequence number.
    offset: u64,
    /// Media sequence of the last accepted playlist, as served.
    last_media_sequence: Option<u64>,
    /// Served sequence number -> URL path of the last accepted playlist.
    last_window: std::collections::HashMap<u64, String>,
    /// Highest (renumbered) `seq` returned so far.
    highest: Option<u64>,
}

impl Epoch {
    fn renumber(&mut self, url: &Url, media: &mut MediaInfo) {
        let ms = media.media_sequence;
        let mut stale = false;
        if let Some(last) = self.last_media_sequence.filter(|last| ms < *last) {
            // A lagging origin behind a load balancer serves an older copy of the *same*
            // timeline: its segments match what we saw at the same sequence numbers.
            stale = media
                .segments
                .iter()
                .any(|s| self.last_window.get(&s.seq).map(String::as_str) == Some(s.url.path()));
            if !stale {
                let next = self.highest.map_or(0, |h| h + 1);
                self.offset = next.saturating_sub(ms);
                tracing::warn!(
                    %url,
                    "media sequence went backwards ({last} -> {ms}); treating it as a restarted stream"
                );
            }
        }
        if !stale {
            self.last_media_sequence = Some(ms);
            self.last_window = media
                .segments
                .iter()
                .map(|s| (s.seq, s.url.path().to_string()))
                .collect();
        }
        for s in &mut media.segments {
            // Only the id changes: default AES IVs were already derived from the served number.
            s.seq += self.offset;
            self.highest = Some(self.highest.map_or(s.seq, |h| h.max(s.seq)));
        }
    }
}

impl HlsSource {
    pub fn new(client: Client, url: Url, first: Option<MediaInfo>) -> Self {
        Self {
            client,
            url,
            first,
            epoch: Epoch::default(),
        }
    }

    pub async fn refresh(&mut self) -> Result<Snapshot> {
        let mut media = match self.first.take() {
            Some(m) => m,
            None => {
                // Re-request the original URL (redirecting edges may reissue tokens), but
                // resolve relative URIs against where the playlist was actually served from.
                let (base, body) = self.client.get_bytes_with_url(&self.url, None).await?;
                match parse(&base, &body)? {
                    Manifest::Media(m) => m,
                    Manifest::Master { .. } => {
                        return Err(Error::Parse(format!(
                            "{} is a master playlist, expected media",
                            self.url
                        )))
                    }
                }
            }
        };
        self.epoch.renumber(&self.url, &mut media);
        Ok(Snapshot {
            refresh_after: std::time::Duration::from_secs_f64(
                media.target_duration.clamp(1.0, 30.0),
            ),
            ended: media.ended,
            segments: media.segments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.example.com/show/index.m3u8").unwrap()
    }

    const MASTER: &str = "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"audio/en.m3u8\"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Svenska\",LANGUAGE=\"sv\",DEFAULT=NO,URI=\"audio/sv.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,AUDIO=\"aud\"
360/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,AUDIO=\"aud\"
1080/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,AUDIO=\"aud\"
https://other.example.com/720.m3u8
";

    const MEDIA: &str = "#EXTM3U
#EXT-X-VERSION:4
#EXT-X-TARGETDURATION:6
#EXT-X-MEDIA-SEQUENCE:100
#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"
#EXTINF:6.0,
a.ts
#EXTINF:6.0,
b.ts
#EXT-X-KEY:METHOD=AES-128,URI=\"key2.bin\",IV=0x0000000000000000000000000000ABCD
#EXTINF:6.0,
c.ts
#EXT-X-KEY:METHOD=NONE
#EXT-X-BYTERANGE:1000@0
#EXTINF:4.0,
all.ts
#EXT-X-BYTERANGE:500
#EXTINF:4.0,
all.ts
#EXT-X-ENDLIST
";

    #[test]
    fn master_variants_and_selection() {
        let Manifest::Master { variants, audio } = parse(&base(), MASTER.as_bytes()).unwrap()
        else {
            panic!("expected master");
        };
        assert_eq!(variants.len(), 3);
        assert_eq!(audio.len(), 2);
        assert_eq!(
            variants[0].url.as_str(),
            "https://cdn.example.com/show/360/index.m3u8"
        );
        assert_eq!(
            select_variant(&variants, &Quality::Best).unwrap().height(),
            1080
        );
        assert_eq!(
            select_variant(&variants, &Quality::Worst).unwrap().height(),
            360
        );
        let v720 = select_variant(&variants, &"720p".parse().unwrap()).unwrap();
        assert_eq!(v720.url.as_str(), "https://other.example.com/720.m3u8");
        assert!(select_variant(&variants, &Quality::MaxHeight(240)).is_err());

        assert_eq!(select_audio(&audio, "aud", None).unwrap().name, "English");
        assert_eq!(
            select_audio(&audio, "aud", Some("sv")).unwrap().name,
            "Svenska"
        );
        assert!(select_audio(&audio, "missing", None).is_none());
    }

    #[test]
    fn media_key_carry_forward_and_byteranges() {
        let Manifest::Media(m) = parse(&base(), MEDIA.as_bytes()).unwrap() else {
            panic!("expected media");
        };
        assert!(m.ended);
        assert_eq!(m.target_duration, 6.0);
        let s = &m.segments;
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].seq, 100);
        // Key carried forward from segment 0 to segment 1, with sequence-derived IVs.
        assert_eq!(
            s[1].key.as_ref().unwrap().url.as_str(),
            "https://cdn.example.com/show/key.bin"
        );
        assert_eq!(s[1].key.as_ref().unwrap().iv, iv_from_sequence(101));
        assert_eq!(s[2].key.as_ref().unwrap().iv[14..], [0xAB, 0xCD]);
        assert!(s[3].key.is_none());
        assert_eq!(s[3].byte_range, Some((0, 1000)));
        assert_eq!(s[4].byte_range, Some((1000, 500)));
    }

    #[test]
    fn drm_is_rejected() {
        let pl = "#EXTM3U
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"
#EXTINF:6.0,
a.ts
";
        assert!(matches!(
            parse(&base(), pl.as_bytes()),
            Err(Error::Unsupported(_))
        ));
    }

    fn media(pl: &str) -> MediaInfo {
        match parse(&base(), pl.as_bytes()).unwrap() {
            Manifest::Media(m) => m,
            Manifest::Master { .. } => panic!("expected media"),
        }
    }

    #[test]
    fn fractional_target_duration_does_not_add_a_phantom_segment() {
        let m = media(
            "#EXTM3U
#EXT-X-TARGETDURATION:5.5
#EXT-X-MEDIA-SEQUENCE:10
#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\"
#EXTINF:5.5,
a.ts
#EXTINF:5.5,
b.ts
",
        );
        assert_eq!(m.target_duration, 6.0);
        assert_eq!(m.segments.len(), 2);
        assert_eq!(m.segments[0].seq, 10);
        assert_eq!(m.segments[0].url.path(), "/show/a.ts");
        assert_eq!(m.segments[0].key.as_ref().unwrap().iv, iv_from_sequence(10));
    }

    #[test]
    fn leniently_quoted_keys_are_honoured() {
        for line in [
            "#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\",IV=\"0x0000000000000000000000000000ABCD\"",
            "#EXT-X-KEY:METHOD=\"AES-128\",URI=\"k.bin\",IV=0x0000000000000000000000000000ABCD",
            "#EXT-X-KEY:METHOD=AES-128, URI=\"k.bin\", IV=0xABCD, KEYFORMAT=identity",
        ] {
            let m = media(&format!(
                "#EXTM3U\n#EXT-X-TARGETDURATION:6\n{line}\n#EXTINF:6,\na.ts\n"
            ));
            let key = m.segments[0]
                .key
                .as_ref()
                .unwrap_or_else(|| panic!("{line}"));
            assert_eq!(key.url.path(), "/show/k.bin");
            assert_eq!(key.iv[14..], [0xAB, 0xCD], "{line}");
        }
    }

    #[test]
    fn key_order_is_kept_for_method_none() {
        let m = media(
            "#EXTM3U
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=NONE
#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\"
#EXTINF:6,
a.ts
#EXTINF:6,
b.ts
#EXT-X-KEY:METHOD=AES-128,URI=\"k2.bin\"
#EXT-X-KEY:METHOD=NONE
#EXTINF:6,
c.ts
",
        );
        let keyed: Vec<bool> = m.segments.iter().map(|s| s.key.is_some()).collect();
        assert_eq!(keyed, [true, true, false]);
    }

    #[test]
    fn unparseable_key_is_an_error_not_plaintext() {
        let pl = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-KEY:URI=\"k.bin\"\n#EXTINF:6,\na.ts\n";
        assert!(matches!(
            parse(&base(), pl.as_bytes()),
            Err(Error::Parse(e)) if e.contains("EXT-X-KEY")
        ));
    }

    #[test]
    fn drm_with_unquoted_keyformat_is_still_rejected() {
        let pl = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=com.apple.streamingkeydelivery\n#EXTINF:6,\na.ts\n";
        assert!(matches!(
            parse(&base(), pl.as_bytes()),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn bom_crlf_and_trailing_whitespace() {
        let pl = "\u{feff}#EXTM3U\r\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360 \r\n360.m3u8 \r\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080 \r\n1080.m3u8\r\n#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720\r\n720.m3u8\r\n";
        let Manifest::Master { variants, .. } = parse(&base(), pl.as_bytes()).unwrap() else {
            panic!("expected master");
        };
        let got: Vec<(u64, &str)> = variants
            .iter()
            .map(|v| (v.height(), v.url.path()))
            .collect();
        assert_eq!(
            got,
            [
                (360, "/show/360.m3u8"),
                (1080, "/show/1080.m3u8"),
                (720, "/show/720.m3u8")
            ]
        );
    }

    #[test]
    fn unparseable_stream_inf_is_an_error_not_a_shifted_table() {
        let pl = "#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
360.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=\"1920x1080\"
1080.m3u8
";
        assert!(matches!(
            parse(&base(), pl.as_bytes()),
            Err(Error::Parse(e)) if e.contains("EXT-X-STREAM-INF")
        ));
    }

    #[test]
    fn audio_only_variants_are_not_picked_as_video() {
        let pl = "#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.2\"
audio.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\"
360.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\"
720.m3u8
";
        let Manifest::Master { variants, .. } = parse(&base(), pl.as_bytes()).unwrap() else {
            panic!("expected master");
        };
        assert!(variants[0].is_audio_only());
        assert!(!variants[1].is_audio_only());
        let pick = |q| select_variant(&variants, &q).map(|v| v.url.path().to_string());
        assert_eq!(pick(Quality::Worst).unwrap(), "/show/360.m3u8");
        assert_eq!(pick(Quality::Best).unwrap(), "/show/720.m3u8");
        assert!(matches!(
            pick(Quality::MaxHeight(240)),
            Err(Error::NoVariant(_))
        ));
        // An audio-only master still works.
        assert_eq!(
            select_variant(&variants[..1], &Quality::Best)
                .unwrap()
                .url
                .path(),
            "/show/audio.m3u8"
        );
    }

    #[test]
    fn init_section_key_requires_an_explicit_iv() {
        let m = media(
            "#EXTM3U
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\",IV=0x01
#EXT-X-MAP:URI=\"init.mp4\"
#EXTINF:6,
a.m4s
#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\"
#EXT-X-MAP:URI=\"init2.mp4\"
#EXTINF:6,
b.m4s
#EXT-X-KEY:METHOD=NONE
#EXT-X-MAP:URI=\"init3.mp4\"
#EXTINF:6,
c.m4s
",
        );
        let inits: Vec<_> = m.segments.iter().map(|s| s.init.clone().unwrap()).collect();
        let k = inits[0].key.as_ref().unwrap();
        assert_eq!((k.url.path(), k.iv[15]), ("/show/k.bin", 1));
        assert!(inits[1].key.is_none());
        assert!(inits[2].key.is_none());
    }

    #[test]
    fn media_sequence_reset_starts_a_new_epoch() {
        let pl = |ms: u64, names: &[&str]| {
            let mut s = format!("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{ms}\n");
            for n in names {
                s.push_str(&format!("#EXTINF:2,\n{n}\n"));
            }
            media(&s)
        };
        let seqs = |m: &MediaInfo| m.segments.iter().map(|s| s.seq).collect::<Vec<_>>();
        let mut e = Epoch::default();
        let u = base();

        let mut m = pl(50, &["s50.ts", "s51.ts", "s52.ts"]);
        e.renumber(&u, &mut m);
        assert_eq!(seqs(&m), [50, 51, 52]);
        // A lagging origin serves an older copy of the same timeline: numbering is kept.
        let mut m = pl(49, &["s49.ts", "s50.ts", "s51.ts"]);
        e.renumber(&u, &mut m);
        assert_eq!(seqs(&m), [49, 50, 51]);
        // The encoder restarts at 0: the new segments continue after the highest id.
        let mut m = pl(0, &["s0.ts", "s1.ts"]);
        e.renumber(&u, &mut m);
        assert_eq!(seqs(&m), [53, 54]);
        // The new epoch then slides normally.
        let mut m = pl(1, &["s1.ts", "s2.ts"]);
        e.renumber(&u, &mut m);
        assert_eq!(seqs(&m), [54, 55]);
    }

    #[test]
    fn iv_helpers() {
        assert_eq!(iv_from_sequence(1)[15], 1);
        assert_eq!(parse_iv("0x01").unwrap()[15], 1);
        assert!(parse_iv(&format!("0x{}", "f".repeat(40))).is_err());
    }
}
