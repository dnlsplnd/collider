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
    /// The manifest URL as requested; live refreshes re-request it.
    pub url: Url,
    /// The URL the manifest was finally served from, after redirects. Relative URIs in the
    /// manifest resolve against it (RFC 8216 §4.1, RFC 3986 §5.1.3, ISO 23009-1 §5.6).
    pub base: Url,
    pub body: Bytes,
}

pub async fn probe(client: &Client, url: &Url) -> Result<Probe> {
    let (base, body) = client.get_bytes_with_url(url, None).await?;
    let protocol = detect(&base, &body)?;
    Ok(Probe {
        protocol,
        url: url.clone(),
        base,
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
        Protocol::Hls => Ok(hls::describe(&hls::parse(&p.base, &p.body)?)),
        Protocol::Dash => Ok(dash::describe(&dash::parse(&p.base, utf8(&p.body)?)?)),
    }
}

/// Like [`describe`], but for HLS also reads the media playlist of the best variant, so
/// liveness, duration, segment count and encryption are known before downloading. A
/// playlist encrypted with anything but clear-key AES-128 is reported as DRM rather than
/// as an error. Reading the media playlist is best effort: on failure the master's own
/// description is returned.
pub async fn inspect(client: &Client, p: &Probe) -> Result<StreamInfo> {
    if p.protocol != Protocol::Hls {
        return describe(p);
    }
    let manifest = match hls::parse(&p.base, &p.body) {
        Err(Error::Unsupported(_)) => return Ok(drm_info()),
        other => other?,
    };
    let mut info = hls::describe(&manifest);
    if let hls::Manifest::Master { variants, .. } = &manifest {
        let Ok(best) = hls::select_variant(variants, &Quality::Best) else {
            return Ok(info);
        };
        let media = client
            .get_bytes_with_url(&best.url, None)
            .await
            .and_then(|(base, body)| hls::parse(&base, &body));
        match media {
            Ok(m @ hls::Manifest::Media(_)) => {
                let m = hls::describe(&m);
                info.live = m.live;
                info.duration = m.duration;
                info.segments = m.segments;
                info.encrypted = m.encrypted;
            }
            Err(Error::Unsupported(_)) => info.drm = true,
            Ok(hls::Manifest::Master { .. }) | Err(_) => {}
        }
    }
    Ok(info)
}

fn drm_info() -> StreamInfo {
    StreamInfo {
        protocol: Protocol::Hls,
        live: false,
        variants: Vec::new(),
        audio: Vec::new(),
        duration: None,
        segments: None,
        encrypted: true,
        drm: true,
    }
}

pub struct PlannedTrack {
    pub name: String,
    pub kind: TrackKind,
    pub description: String,
    /// What this track records: the media playlist, or the MPD plus representation id.
    /// A `.parts` directory is only resumed into by a track with the same identity.
    pub identity: String,
    pub source: Source,
}

/// `url` without query and fragment, so rotating auth tokens do not change the identity.
fn identity(url: &Url, rep: Option<&str>) -> String {
    let mut u = url.clone();
    u.set_query(None);
    u.set_fragment(None);
    match rep {
        Some(r) => format!("{u} representation={r}"),
        None => u.to_string(),
    }
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
    match hls::parse(&p.base, &p.body)? {
        hls::Manifest::Media(m) => Ok(vec![PlannedTrack {
            name: "main".into(),
            kind: TrackKind::Other,
            description: format!("media playlist, {} segments", m.segments.len()),
            identity: identity(&p.url, None),
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
                identity: identity(&v.url, None),
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
                        identity: identity(&a.url, None),
                        source: Source::Hls(HlsSource::new(client.clone(), a.url.clone(), None)),
                    });
                }
            }
            Ok(tracks)
        }
    }
}

fn plan_dash(client: &Client, p: &Probe, sel: &Selection<'_>) -> Result<Vec<PlannedTrack>> {
    let m = dash::parse(&p.base, utf8(&p.body)?)?;
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
        identity: identity(&p.url, Some(&main.id)),
        source: Source::Dash(DashSource::new(
            client.clone(),
            p.url.clone(),
            m.clone(),
            &main,
            sel.quality,
            sel.audio_lang,
        )),
    }];
    if let Some(a) = audio {
        tracks.push(PlannedTrack {
            name: "audio".into(),
            kind: TrackKind::Audio,
            description: describe(&a),
            identity: identity(&p.url, Some(&a.id)),
            source: Source::Dash(DashSource::new(
                client.clone(),
                p.url.clone(),
                m,
                &a,
                sel.quality,
                sel.audio_lang,
            )),
        });
    }
    Ok(tracks)
}

fn utf8(b: &[u8]) -> Result<&str> {
    std::str::from_utf8(b).map_err(|e| Error::Parse(format!("manifest is not UTF-8: {e}")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// Minimal HTTP/1.1 server for tests: `routes` maps a path to `(status, extra headers, body)`.
    /// A path listed several times answers with each entry in turn, then keeps the last.
    /// Every request is appended to the returned log. Runs until the test process exits.
    pub(crate) fn serve(
        routes: Vec<(&'static str, u16, &'static str, Vec<u8>)>,
    ) -> (Url, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            let mut hits = std::collections::HashMap::<String, usize>::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                log2.lock().unwrap().push(path.clone());
                let matching: Vec<_> = routes.iter().filter(|r| r.0 == path).collect();
                let n = hits.entry(path.clone()).or_default();
                let (status, headers, body) = matching
                    .get((*n).min(matching.len().saturating_sub(1)))
                    .map(|r| (r.1, r.2, r.3.clone()))
                    .unwrap_or((404, "", Vec::new()));
                *n += 1;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        (Url::parse(&format!("http://{addr}/")).unwrap(), log)
    }

    fn client() -> Client {
        Client::new(&[], 0, std::time::Duration::from_secs(5)).unwrap()
    }

    #[tokio::test]
    async fn inspect_reads_the_media_playlist_behind_an_hls_master() {
        let master = b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900,RESOLUTION=640x360\nlo.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3000,RESOLUTION=1280x720\nhi.m3u8\n";
        let vod = b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n\
#EXTINF:4,\na.ts\n#EXTINF:3.5,\nb.ts\n#EXT-X-ENDLIST\n";
        let live = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n";
        let drm = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n\
#EXTINF:2,\na.ts\n#EXT-X-ENDLIST\n";
        let (root, log) = serve(vec![
            ("/vod/master.m3u8", 200, "", master.to_vec()),
            ("/vod/hi.m3u8", 200, "", vod.to_vec()),
            ("/live/master.m3u8", 200, "", master.to_vec()),
            ("/live/hi.m3u8", 200, "", live.to_vec()),
            ("/drm/master.m3u8", 200, "", master.to_vec()),
            ("/drm/hi.m3u8", 200, "", drm.to_vec()),
            ("/drm/media.m3u8", 200, "", drm.to_vec()),
        ]);
        let c = client();
        let info = |path: &'static str| {
            let c = c.clone();
            let url = root.join(path).unwrap();
            async move {
                let p = probe(&c, &url).await.unwrap();
                inspect(&c, &p).await.unwrap()
            }
        };

        let vod = info("vod/master.m3u8").await;
        assert_eq!(vod.variants.len(), 2);
        assert!(!vod.live && vod.encrypted && !vod.drm);
        assert_eq!((vod.duration, vod.segments), (Some(7.5), Some(2)));
        // Only the best variant's playlist is read.
        assert!(!log.lock().unwrap().iter().any(|p| p.ends_with("lo.m3u8")));

        let live = info("live/master.m3u8").await;
        assert!(live.live && !live.encrypted);

        assert!(info("drm/master.m3u8").await.drm);
        // A media playlist with DRM is described, not an error.
        assert!(info("drm/media.m3u8").await.drm);
    }

    #[tokio::test]
    async fn hls_relative_uris_resolve_against_the_redirect_target() {
        let master =
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,RESOLUTION=640x360\nv1/index.m3u8\n";
        let media = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nseg0.ts\n#EXT-X-ENDLIST\n";
        let (root, log) = serve(vec![
            (
                "/master.m3u8",
                302,
                "Location: /edge/abc/master.m3u8\r\n",
                Vec::new(),
            ),
            ("/edge/abc/master.m3u8", 200, "", master.to_vec()),
            (
                "/edge/abc/v1/index.m3u8",
                302,
                "Location: /edge2/index.m3u8\r\n",
                Vec::new(),
            ),
            ("/edge2/index.m3u8", 200, "", media.to_vec()),
        ]);
        let c = client();
        let p = probe(&c, &root.join("master.m3u8").unwrap()).await.unwrap();
        assert_eq!(p.url.path(), "/master.m3u8");
        assert_eq!(p.base.path(), "/edge/abc/master.m3u8");
        let sel = Selection {
            quality: &Quality::Best,
            audio_lang: None,
        };
        let mut tracks = plan(&c, &p, &sel).unwrap();
        let snap = tracks[0].source.refresh().await.unwrap();
        assert_eq!(snap.segments[0].url.path(), "/edge2/seg0.ts");
        // A refresh re-requests the original URL, not the redirect target.
        assert_eq!(
            log.lock().unwrap().as_slice(),
            [
                "/master.m3u8",
                "/edge/abc/master.m3u8",
                "/edge/abc/v1/index.m3u8",
                "/edge2/index.m3u8"
            ]
        );
    }

    #[tokio::test]
    async fn dash_relative_uris_resolve_against_the_redirect_target() {
        let mpd = br#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT4S">
  <Period><AdaptationSet contentType="video" mimeType="video/mp4">
    <SegmentTemplate initialization="init.mp4" media="seg-$Number$.m4s" startNumber="1" duration="2" timescale="1"/>
    <Representation id="v" bandwidth="1000" width="640" height="360"/>
  </AdaptationSet></Period>
</MPD>"#;
        let (root, _log) = serve(vec![
            (
                "/a/manifest.mpd",
                302,
                "Location: /b/manifest.mpd\r\n",
                Vec::new(),
            ),
            ("/b/manifest.mpd", 200, "", mpd.to_vec()),
        ]);
        let c = client();
        let p = probe(&c, &root.join("a/manifest.mpd").unwrap())
            .await
            .unwrap();
        let sel = Selection {
            quality: &Quality::Best,
            audio_lang: None,
        };
        let mut tracks = plan(&c, &p, &sel).unwrap();
        let snap = tracks[0].source.refresh().await.unwrap();
        assert_eq!(snap.segments[0].url.path(), "/b/seg-1.m4s");
        assert_eq!(
            snap.segments[0].init.as_ref().unwrap().url.path(),
            "/b/init.mp4"
        );
    }

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
