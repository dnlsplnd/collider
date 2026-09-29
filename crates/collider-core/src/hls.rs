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
    match m3u8_rs::parse_playlist_res(bytes) {
        Ok(Playlist::MasterPlaylist(m)) => {
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

fn convert_media(base: &Url, pl: m3u8_rs::MediaPlaylist) -> Result<MediaInfo> {
    // EXT-X-KEY and EXT-X-MAP apply to every following segment until replaced;
    // m3u8-rs only attaches them to the segment they precede, so carry them forward.
    let mut key: Option<m3u8_rs::Key> = None;
    let mut init: Option<InitSection> = None;
    let mut prev_range_end: Option<(Url, u64)> = None;
    let mut segments = Vec::with_capacity(pl.segments.len());

    for (i, s) in pl.segments.into_iter().enumerate() {
        let seq = pl.media_sequence + i as u64;
        if let Some(k) = s.key {
            key = if matches!(k.method, KeyMethod::None) {
                None
            } else {
                Some(k)
            };
        }
        // m3u8-rs 6 rejects the valid `METHOD=NONE` (no IV) and leaves it as an unknown tag.
        if s.unknown_tags.iter().any(|t| {
            t.tag == "X-KEY" && t.rest.as_deref().is_some_and(|r| r.contains("METHOD=NONE"))
        }) {
            key = None;
        }
        if let Some(m) = s.map {
            init = Some(InitSection {
                url: base.join(&m.uri)?,
                byte_range: m.byte_range.map(|r| (r.offset.unwrap_or(0), r.length)),
            });
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
        segments.push(Segment {
            seq,
            url,
            duration: s.duration as f64,
            byte_range,
            key: seg_key,
            init: init.clone(),
        });
    }

    Ok(MediaInfo {
        segments,
        target_duration: pl.target_duration as f64,
        ended: pl.end_list,
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
    let pick = match q {
        Quality::Best => variants.iter().max_by_key(rank),
        Quality::Worst => variants.iter().min_by_key(rank),
        Quality::MaxHeight(h) => variants
            .iter()
            .filter(|v| v.height() <= *h)
            .max_by_key(rank),
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
}

impl HlsSource {
    pub fn new(client: Client, url: Url, first: Option<MediaInfo>) -> Self {
        Self { client, url, first }
    }

    pub async fn refresh(&mut self) -> Result<Snapshot> {
        let media = match self.first.take() {
            Some(m) => m,
            None => {
                let body = self.client.get_bytes(&self.url, None).await?;
                match parse(&self.url, &body)? {
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

    #[test]
    fn iv_helpers() {
        assert_eq!(iv_from_sequence(1)[15], 1);
        assert_eq!(parse_iv("0x01").unwrap()[15], 1);
        assert!(parse_iv(&format!("0x{}", "f".repeat(40))).is_err());
    }
}
