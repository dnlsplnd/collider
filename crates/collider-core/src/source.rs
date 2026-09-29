//! Protocol detection and track planning: turns a manifest URL into segment sources.

use bytes::Bytes;
use url::Url;

use crate::dash::{self, DashSource};
use crate::error::{Error, Result};
use crate::hls::{self, HlsSource, Quality};
use crate::http::Client;
use crate::model::{Protocol, Snapshot, StreamInfo, TrackKind};

pub enum Source {
    Hls(HlsSource),
    Dash(DashSource),
}

impl Source {
    pub async fn refresh(&mut self) -> Result<Snapshot> {
        match self {
            Source::Hls(s) => s.refresh().await,
            Source::Dash(s) => s.refresh().await,
        }
    }
}

/// A fetched manifest with its detected protocol.
pub struct Probe {
    pub protocol: Protocol,
    pub url: Url,
    pub body: Bytes,
}

pub async fn probe(client: &Client, url: &Url) -> Result<Probe> {
    let body = client.get_bytes(url, None).await?;
    let protocol = detect(url, &body)?;
    Ok(Probe {
        protocol,
        url: url.clone(),
        body,
    })
}

pub fn detect(url: &Url, body: &[u8]) -> Result<Protocol> {
    let head = String::from_utf8_lossy(&body[..body.len().min(4096)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("#EXTM3U") {
        return Ok(Protocol::Hls);
    }
    if head.contains("<MPD") {
        return Ok(Protocol::Dash);
    }
    let path = url.path().to_ascii_lowercase();
    if path.ends_with(".m3u8") || path.ends_with(".m3u") {
        return Ok(Protocol::Hls);
    }
    if path.ends_with(".mpd") {
        return Ok(Protocol::Dash);
    }
    Err(Error::Parse(format!(
        "{url} is neither an HLS playlist nor a DASH MPD"
    )))
}

pub fn describe(p: &Probe) -> Result<StreamInfo> {
    match p.protocol {
        Protocol::Hls => Ok(hls::describe(&hls::parse(&p.url, &p.body)?)),
        Protocol::Dash => Ok(dash::describe(&dash::parse(&p.url, utf8(&p.body)?)?)),
    }
}

pub struct PlannedTrack {
    pub name: String,
    pub kind: TrackKind,
    pub description: String,
    pub source: Source,
}

pub struct Selection<'a> {
    pub quality: &'a Quality,
    pub audio_lang: Option<&'a str>,
}

pub fn plan(client: &Client, p: &Probe, sel: &Selection<'_>) -> Result<Vec<PlannedTrack>> {
    match p.protocol {
        Protocol::Hls => plan_hls(client, p, sel),
        Protocol::Dash => plan_dash(client, p, sel),
    }
}

fn plan_hls(client: &Client, p: &Probe, sel: &Selection<'_>) -> Result<Vec<PlannedTrack>> {
    match hls::parse(&p.url, &p.body)? {
        hls::Manifest::Media(m) => Ok(vec![PlannedTrack {
            name: "main".into(),
            kind: TrackKind::Other,
            description: format!("media playlist, {} segments", m.segments.len()),
            source: Source::Hls(HlsSource::new(client.clone(), p.url.clone(), Some(m))),
        }]),
        hls::Manifest::Master { variants, audio } => {
            let v = hls::select_variant(&variants, sel.quality)?;
            let mut tracks = vec![PlannedTrack {
                name: "video".into(),
                kind: TrackKind::Video,
                description: format!(
                    "{} @ {} kbps{}",
                    v.resolution
                        .map(|(w, h)| format!("{w}x{h}"))
                        .unwrap_or_else(|| "unknown resolution".into()),
                    v.bandwidth / 1000,
                    v.codecs
                        .as_deref()
                        .map(|c| format!(" [{c}]"))
                        .unwrap_or_default()
                ),
                source: Source::Hls(HlsSource::new(client.clone(), v.url.clone(), None)),
            }];
            if let Some(group) = &v.audio_group {
                if let Some(a) = hls::select_audio(&audio, group, sel.audio_lang) {
                    tracks.push(PlannedTrack {
                        name: "audio".into(),
                        kind: TrackKind::Audio,
                        description: format!(
                            "{}{}",
                            a.name,
                            a.language
                                .as_deref()
                                .map(|l| format!(" ({l})"))
                                .unwrap_or_default()
                        ),
                        source: Source::Hls(HlsSource::new(client.clone(), a.url.clone(), None)),
                    });
                }
            }
            Ok(tracks)
        }
    }
}

fn plan_dash(client: &Client, p: &Probe, sel: &Selection<'_>) -> Result<Vec<PlannedTrack>> {
    let m = dash::parse(&p.url, utf8(&p.body)?)?;
    let (main, audio) = dash::select(&m, sel.quality, sel.audio_lang)?;
    let describe = |r: &dash::Rep| {
        format!(
            "{}{} @ {} kbps{}{}",
            r.id,
            r.width
                .zip(r.height)
                .map(|(w, h)| format!(" {w}x{h}"))
                .unwrap_or_default(),
            r.bandwidth / 1000,
            r.codecs
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default(),
            r.lang
                .as_deref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default(),
        )
    };
    let mut tracks = vec![PlannedTrack {
        name: if main.kind == TrackKind::Video {
            "video"
        } else {
            "audio"
        }
        .into(),
        kind: main.kind,
        description: describe(&main),
        source: Source::Dash(DashSource::new(client.clone(), m.clone(), &main)),
    }];
    if let Some(a) = audio {
        tracks.push(PlannedTrack {
            name: "audio".into(),
            kind: TrackKind::Audio,
            description: describe(&a),
            source: Source::Dash(DashSource::new(client.clone(), m, &a)),
        });
    }
    Ok(tracks)
}

fn utf8(b: &[u8]) -> Result<&str> {
    std::str::from_utf8(b).map_err(|e| Error::Parse(format!("manifest is not UTF-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection() {
        let u = Url::parse("https://x/a").unwrap();
        assert_eq!(detect(&u, b"\xEF\xBB\xBF#EXTM3U\n").unwrap(), Protocol::Hls);
        assert_eq!(
            detect(&u, b"<?xml version=\"1.0\"?>\n<MPD xmlns=\"x\">").unwrap(),
            Protocol::Dash
        );
        assert_eq!(
            detect(&Url::parse("https://x/a.MPD?x=1").unwrap(), b"").unwrap(),
            Protocol::Dash
        );
        assert_eq!(
            detect(&Url::parse("https://x/a.m3u8").unwrap(), b"").unwrap(),
            Protocol::Hls
        );
        assert!(detect(&u, b"<html>").is_err());
    }
}
