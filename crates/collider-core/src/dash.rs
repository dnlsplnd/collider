//! MPEG-DASH: MPD parsing, representation selection and segment enumeration.
//!
//! Supports SegmentTemplate (`$Number$` and `$Time$`, with or without SegmentTimeline),
//! SegmentList, and SegmentBase single-file representations indexed by `sidx`.
//! Static and dynamic (live) presentations, multi-period, and BaseURL inheritance are handled.
//! Presentations with `ContentProtection` (DRM) are rejected.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dash_mpd::{AdaptationSet, Period, Representation, SegmentTemplate, MPD};
use url::Url;

use crate::error::{Error, Result};
use crate::hls::Quality;
use crate::http::Client;
use crate::model::{
    parse_range, AudioInfo, InitSection, Protocol, Segment, Snapshot, StreamInfo, TrackKind,
    VariantInfo,
};
use crate::sidx;

/// A representation resolved within one period, with all inheritance applied.
#[derive(Debug, Clone)]
pub struct Rep {
    pub period_idx: usize,
    pub id: String,
    pub kind: TrackKind,
    pub bandwidth: u64,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub codecs: Option<String>,
    pub frame_rate: Option<f64>,
    pub lang: Option<String>,
    pub adaptation_id: String,
    pub default_audio: bool,
    pub protected: bool,
    pub base: Url,
    pub addressing: Addressing,
    /// Period start relative to the presentation, and its duration when known.
    pub period_start: f64,
    pub period_duration: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum Addressing {
    Template {
        init: Option<String>,
        media: String,
        timescale: u64,
        start_number: u64,
        end_number: Option<u64>,
        duration: Option<f64>,
        timeline: Option<Vec<TimelineEntry>>,
        availability_time_offset: f64,
    },
    List {
        init: Option<InitSection>,
        items: Vec<(Url, Option<(u64, u64)>)>,
        duration: Option<f64>,
    },
    Base {
        init_range: Option<(u64, u64)>,
        index_range: Option<(u64, u64)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineEntry {
    pub t: Option<u64>,
    pub d: u64,
    pub r: i64,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub url: Url,
    pub live: bool,
    pub availability_start: Option<DateTime<Utc>>,
    pub time_shift_buffer: Option<Duration>,
    pub min_update_period: Option<Duration>,
    pub duration: Option<f64>,
    pub reps: Vec<Rep>,
}

pub fn parse(url: &Url, body: &str) -> Result<Manifest> {
    let mpd = dash_mpd::parse(body).map_err(|e| Error::Parse(format!("MPD: {e}")))?;
    convert(url, &mpd)
}

fn convert(url: &Url, mpd: &MPD) -> Result<Manifest> {
    let live = mpd.mpdtype.as_deref() == Some("dynamic");
    let mpd_base = join_base(url, &mpd.base_url)?;
    let total = mpd.mediaPresentationDuration.map(|d| d.as_secs_f64());
    let mpd_protected = !mpd.ContentProtection.is_empty();

    let mut reps = Vec::new();
    let mut cursor = 0.0f64;
    for (pi, period) in mpd.periods.iter().enumerate() {
        let start = period.start.map(|d| d.as_secs_f64()).unwrap_or(cursor);
        let next_start = mpd
            .periods
            .get(pi + 1)
            .and_then(|p| p.start)
            .map(|d| d.as_secs_f64());
        let duration = period
            .duration
            .map(|d| d.as_secs_f64())
            .or_else(|| next_start.map(|n| n - start))
            .or_else(|| total.map(|t| t - start));
        cursor = start + duration.unwrap_or(0.0);
        let period_base = join_base(&mpd_base, &period.BaseURL)?;
        let period_protected = mpd_protected || !period.ContentProtection.is_empty();

        for (ai, adaptation) in period.adaptations.iter().enumerate() {
            let a_base = join_base(&period_base, &adaptation.BaseURL)?;
            let a_protected = period_protected || !adaptation.ContentProtection.is_empty();
            let default_audio = adaptation
                .Role
                .iter()
                .any(|r| r.value.as_deref() == Some("main"));
            for (ri, r) in adaptation.representations.iter().enumerate() {
                let base = join_base(&a_base, &r.BaseURL)?;
                let id = r.id.clone().unwrap_or_else(|| {
                    format!("p{pi}a{}r{ri}", adaptation.id.as_deref().unwrap_or("?"))
                });
                let addressing = addressing(period, adaptation, r, &base)?;
                reps.push(Rep {
                    period_idx: pi,
                    kind: kind_of(adaptation, r),
                    bandwidth: r.bandwidth.unwrap_or(0),
                    width: r.width.or(adaptation.width),
                    height: r.height.or(adaptation.height),
                    codecs: r.codecs.clone().or_else(|| adaptation.codecs.clone()),
                    frame_rate: r.frameRate.as_deref().and_then(parse_frame_rate),
                    lang: r.lang.clone().or_else(|| adaptation.lang.clone()),
                    adaptation_id: adaptation
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("p{pi}-as{ai}")),
                    default_audio,
                    protected: a_protected || !r.ContentProtection.is_empty(),
                    base,
                    addressing,
                    period_start: start,
                    period_duration: duration,
                    id,
                });
            }
        }
    }
    if reps.is_empty() {
        return Err(Error::Parse("MPD has no representations".into()));
    }
    Ok(Manifest {
        url: url.clone(),
        live,
        availability_start: mpd.availabilityStartTime,
        time_shift_buffer: mpd.timeShiftBufferDepth,
        min_update_period: mpd.minimumUpdatePeriod,
        duration: total,
        reps,
    })
}

fn join_base(parent: &Url, bases: &[dash_mpd::BaseURL]) -> Result<Url> {
    match bases.first() {
        Some(b) => Ok(parent.join(b.base.trim())?),
        None => Ok(parent.clone()),
    }
}

fn kind_of(a: &AdaptationSet, r: &Representation) -> TrackKind {
    let ty = r
        .contentType
        .clone()
        .or_else(|| a.contentType.clone())
        .or_else(|| {
            r.mimeType
                .clone()
                .or_else(|| a.mimeType.clone())
                .map(|m| m.split('/').next().unwrap_or("").to_string())
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ty.as_str() {
        "video" => TrackKind::Video,
        "audio" => TrackKind::Audio,
        _ => {
            let codecs = r
                .codecs
                .clone()
                .or_else(|| a.codecs.clone())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if ["avc", "hev", "hvc", "vp0", "av01", "dvh", "mp4v"]
                .iter()
                .any(|c| codecs.starts_with(c))
            {
                TrackKind::Video
            } else if ["mp4a", "opus", "ac-3", "ec-3", "vorbis", "flac"]
                .iter()
                .any(|c| codecs.starts_with(c))
            {
                TrackKind::Audio
            } else {
                TrackKind::Other
            }
        }
    }
}

fn parse_frame_rate(s: &str) -> Option<f64> {
    match s.split_once('/') {
        Some((n, d)) => Some(n.trim().parse::<f64>().ok()? / d.trim().parse::<f64>().ok()?),
        None => s.trim().parse().ok(),
    }
}

/// Merge SegmentTemplate attributes across Period → AdaptationSet → Representation.
fn merged_template(p: &Period, a: &AdaptationSet, r: &Representation) -> Option<SegmentTemplate> {
    let levels = [
        p.SegmentTemplate.as_ref(),
        a.SegmentTemplate.as_ref(),
        r.SegmentTemplate.as_ref(),
    ];
    if levels.iter().all(|l| l.is_none()) {
        return None;
    }
    let mut out = SegmentTemplate::default();
    for t in levels.into_iter().flatten() {
        macro_rules! take {
            ($($f:ident),*) => { $( if t.$f.is_some() { out.$f = t.$f.clone(); } )* };
        }
        take!(
            media,
            initialization,
            timescale,
            duration,
            startNumber,
            endNumber,
            presentationTimeOffset,
            availabilityTimeOffset,
            SegmentTimeline,
            Initialization
        );
    }
    Some(out)
}

fn addressing(p: &Period, a: &AdaptationSet, r: &Representation, base: &Url) -> Result<Addressing> {
    if let Some(t) = merged_template(p, a, r) {
        let media = t
            .media
            .clone()
            .ok_or_else(|| Error::Parse("SegmentTemplate without media attribute".into()))?;
        let timeline = t.SegmentTimeline.as_ref().map(|tl| {
            tl.segments
                .iter()
                .map(|s| TimelineEntry {
                    t: s.t,
                    d: s.d,
                    r: s.r.unwrap_or(0),
                })
                .collect::<Vec<_>>()
        });
        return Ok(Addressing::Template {
            init: t
                .initialization
                .clone()
                .or_else(|| t.Initialization.as_ref().and_then(|i| i.sourceURL.clone())),
            media,
            timescale: t.timescale.unwrap_or(1).max(1),
            start_number: t.startNumber.unwrap_or(1),
            end_number: t.endNumber,
            duration: t.duration,
            timeline,
            availability_time_offset: t.availabilityTimeOffset.unwrap_or(0.0),
        });
    }
    if let Some(l) = r
        .SegmentList
        .as_ref()
        .or(a.SegmentList.as_ref())
        .or(p.SegmentList.as_ref())
    {
        let timescale = l.timescale.unwrap_or(1).max(1) as f64;
        let init = match &l.Initialization {
            Some(i) => Some(InitSection {
                url: match &i.sourceURL {
                    Some(u) => base.join(u)?,
                    None => base.clone(),
                },
                byte_range: i.range.as_deref().and_then(parse_range),
            }),
            None => None,
        };
        let mut items = Vec::new();
        for su in &l.segment_urls {
            let url = match &su.media {
                Some(m) => base.join(m)?,
                None => base.clone(),
            };
            items.push((url, su.mediaRange.as_deref().and_then(parse_range)));
        }
        return Ok(Addressing::List {
            init,
            items,
            duration: l.duration.map(|d| d as f64 / timescale),
        });
    }
    let sb = r
        .SegmentBase
        .as_ref()
        .or(a.SegmentBase.as_ref())
        .or(p.SegmentBase.as_ref());
    Ok(Addressing::Base {
        init_range: sb
            .and_then(|s| s.Initialization.as_ref())
            .and_then(|i| i.range.as_deref())
            .and_then(parse_range),
        index_range: sb
            .and_then(|s| s.indexRange.as_deref())
            .and_then(parse_range),
    })
}

/// Expand `$RepresentationID$`, `$Number$`, `$Time$`, `$Bandwidth$` (with optional `%0Nd`) and `$$`.
pub fn expand_template(tpl: &str, rep_id: &str, number: u64, time: u64, bandwidth: u64) -> String {
    let mut out = String::with_capacity(tpl.len() + 16);
    let mut rest = tpl;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let ident = &after[..end];
        rest = &after[end + 1..];
        if ident.is_empty() {
            out.push('$');
            continue;
        }
        let (name, width) = match ident.split_once('%') {
            Some((n, fmt)) => (
                n,
                fmt.trim_start_matches('0')
                    .trim_end_matches('d')
                    .parse::<usize>()
                    .unwrap_or(0),
            ),
            None => (ident, 0),
        };
        let value = match name {
            "RepresentationID" => rep_id.to_string(),
            "Number" => number.to_string(),
            "Time" => time.to_string(),
            "Bandwidth" => bandwidth.to_string(),
            _ => format!("${ident}$"),
        };
        if width > 0 && name != "RepresentationID" {
            out.push_str(&format!("{value:0>width$}"));
        } else {
            out.push_str(&value);
        }
    }
    out.push_str(rest);
    out
}

fn seq_id(period_idx: usize, number: u64) -> u64 {
    ((period_idx as u64) << 40) | (number & ((1 << 40) - 1))
}

/// Segments of a template-addressed representation at wall-clock `now` (only used when live).
pub fn template_segments(
    rep: &Rep,
    live: bool,
    now: DateTime<Utc>,
    m: &Manifest,
) -> Result<Vec<Segment>> {
    let Addressing::Template {
        init,
        media,
        timescale,
        start_number,
        end_number,
        duration,
        timeline,
        availability_time_offset,
    } = &rep.addressing
    else {
        return Err(Error::Parse("not a template representation".into()));
    };
    let init = match init {
        Some(i) => Some(InitSection {
            url: rep
                .base
                .join(&expand_template(i, &rep.id, 0, 0, rep.bandwidth))?,
            byte_range: None,
        }),
        None => None,
    };
    let ts = *timescale as f64;
    let mk = |number: u64, time: u64, dur: f64| -> Result<Segment> {
        Ok(Segment {
            seq: seq_id(rep.period_idx, number),
            url: rep.base.join(&expand_template(
                media,
                &rep.id,
                number,
                time,
                rep.bandwidth,
            ))?,
            duration: dur,
            byte_range: None,
            key: None,
            init: init.clone(),
        })
    };
    let mut out = Vec::new();

    if let Some(tl) = timeline {
        let period_end_ticks = rep.period_duration.map(|d| (d * ts) as u64);
        let mut number = *start_number;
        let mut time = 0u64;
        for e in tl {
            if let Some(t) = e.t {
                time = t;
            }
            let repeats = if e.r >= 0 {
                e.r as u64
            } else {
                // Repeat until the period ends (or the next entry's start when present).
                match period_end_ticks {
                    Some(end) if end > time && e.d > 0 => {
                        (end - time).div_ceil(e.d).saturating_sub(1)
                    }
                    _ => 0,
                }
            };
            for _ in 0..=repeats {
                if end_number.is_some_and(|e| number > e) {
                    break;
                }
                out.push(mk(number, time, e.d as f64 / ts)?);
                number += 1;
                time += e.d;
            }
        }
        return Ok(out);
    }

    let Some(dur) = duration.filter(|d| *d > 0.0) else {
        return Err(Error::Parse(
            "SegmentTemplate has neither SegmentTimeline nor duration".into(),
        ));
    };
    let seg_secs = dur / ts;
    let (first, last) = if live {
        let ast = m
            .availability_start
            .ok_or_else(|| Error::Parse("dynamic MPD without availabilityStartTime".into()))?;
        let elapsed = (now - ast).to_std().map(|d| d.as_secs_f64()).unwrap_or(0.0)
            - rep.period_start
            + availability_time_offset;
        let latest = (elapsed / seg_secs).floor() as i64 - 1;
        let depth = m
            .time_shift_buffer
            .map(|d| d.as_secs_f64())
            .unwrap_or(f64::INFINITY);
        let earliest = ((elapsed - depth) / seg_secs).floor().max(0.0) as i64;
        (earliest, latest)
    } else {
        let count = rep
            .period_duration
            .map(|d| (d / seg_secs).ceil() as i64)
            .unwrap_or(0);
        (0, count - 1)
    };
    let cap = end_number
        .map(|e| e.saturating_sub(*start_number) as i64)
        .unwrap_or(i64::MAX);
    for idx in first.max(0)..=last.min(cap) {
        let number = *start_number + idx as u64;
        let dur_secs = match rep.period_duration {
            Some(d) if !live => (d - idx as f64 * seg_secs).min(seg_secs).max(0.0),
            _ => seg_secs,
        };
        out.push(mk(number, (idx as f64 * dur) as u64, dur_secs)?);
    }
    Ok(out)
}

pub fn list_segments(rep: &Rep) -> Vec<Segment> {
    let Addressing::List {
        init,
        items,
        duration,
    } = &rep.addressing
    else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .map(|(i, (url, range))| Segment {
            seq: seq_id(rep.period_idx, i as u64),
            url: url.clone(),
            duration: duration.unwrap_or(0.0),
            byte_range: *range,
            key: None,
            init: init.clone(),
        })
        .collect()
}

/// Resolve single-file (SegmentBase) representations by fetching and parsing the `sidx` index.
pub async fn base_segments(client: &Client, rep: &Rep) -> Result<Vec<Segment>> {
    let Addressing::Base {
        init_range,
        index_range,
    } = &rep.addressing
    else {
        return Ok(Vec::new());
    };
    let Some((idx_off, idx_len)) = index_range else {
        // No index: the whole file is one segment (it carries its own moov).
        return Ok(vec![Segment {
            seq: seq_id(rep.period_idx, 0),
            url: rep.base.clone(),
            duration: rep.period_duration.unwrap_or(0.0),
            byte_range: None,
            key: None,
            init: None,
        }]);
    };
    let index = client
        .get_bytes(&rep.base, Some((*idx_off, *idx_len)))
        .await?;
    let subs = sidx::parse_sidx(&index, *idx_off)?;
    let init = Some(InitSection {
        url: rep.base.clone(),
        byte_range: Some(init_range.unwrap_or((0, *idx_off))),
    });
    Ok(subs
        .into_iter()
        .enumerate()
        .map(|(i, s)| Segment {
            seq: seq_id(rep.period_idx, i as u64),
            url: rep.base.clone(),
            duration: s.duration,
            byte_range: Some((s.offset, s.size)),
            key: None,
            init: init.clone(),
        })
        .collect())
}

pub fn describe(m: &Manifest) -> StreamInfo {
    let first_period: Vec<&Rep> = m.reps.iter().filter(|r| r.period_idx == 0).collect();
    let variants = first_period
        .iter()
        .filter(|r| r.kind == TrackKind::Video)
        .map(|r| VariantInfo {
            id: r.id.clone(),
            bandwidth: r.bandwidth,
            resolution: r.width.zip(r.height),
            codecs: r.codecs.clone(),
            frame_rate: r.frame_rate,
            audio_group: None,
        })
        .collect();
    let audio = first_period
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .map(|r| AudioInfo {
            id: r.id.clone(),
            group: r.adaptation_id.clone(),
            name: r.id.clone(),
            language: r.lang.clone(),
            default: r.default_audio,
            bandwidth: Some(r.bandwidth),
            codecs: r.codecs.clone(),
        })
        .collect();
    StreamInfo {
        protocol: Protocol::Dash,
        live: m.live,
        variants,
        audio,
        duration: m.duration,
        segments: None,
        encrypted: false,
        drm: m.reps.iter().any(|r| r.protected),
    }
}

/// Pick the video representation (and separate audio, if any) for the first period.
/// Returns `(video_or_main, audio)` representation ids.
pub fn select(
    m: &Manifest,
    quality: &Quality,
    audio_lang: Option<&str>,
) -> Result<(Rep, Option<Rep>)> {
    let first: Vec<&Rep> = m.reps.iter().filter(|r| r.period_idx == 0).collect();
    let video: Vec<&Rep> = first
        .iter()
        .copied()
        .filter(|r| r.kind == TrackKind::Video)
        .collect();
    let audio: Vec<&Rep> = first
        .iter()
        .copied()
        .filter(|r| r.kind == TrackKind::Audio)
        .collect();

    let pick_video = |cands: &[&Rep]| -> Option<Rep> {
        let rank = |r: &&&Rep| (r.height.unwrap_or(0), r.bandwidth);
        match quality {
            Quality::Best => cands.iter().max_by_key(rank),
            Quality::Worst => cands.iter().min_by_key(rank),
            Quality::MaxHeight(h) => cands
                .iter()
                .filter(|r| r.height.unwrap_or(0) <= *h)
                .max_by_key(rank),
        }
        .map(|r| (**r).clone())
    };
    let pick_audio = |cands: &[&Rep]| -> Option<Rep> {
        let set_of = |r: &&Rep| r.adaptation_id.clone();
        let chosen_set = audio_lang
            .and_then(|l| {
                cands
                    .iter()
                    .find(|r| r.lang.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(l)))
            })
            .or_else(|| cands.iter().find(|r| r.default_audio))
            .or_else(|| cands.first())
            .map(set_of)?;
        cands
            .iter()
            .filter(|r| r.adaptation_id == chosen_set)
            .max_by_key(|r| r.bandwidth)
            .map(|r| (*r).clone())
    };

    let (main, extra) = if video.is_empty() {
        (
            pick_audio(&audio)
                .ok_or_else(|| Error::Parse("MPD has no video or audio representations".into()))?,
            None,
        )
    } else {
        (
            pick_video(&video).ok_or_else(|| Error::NoVariant(format!("{quality:?}")))?,
            pick_audio(&audio),
        )
    };
    for r in std::iter::once(&main).chain(extra.iter()) {
        if r.protected {
            return Err(Error::Unsupported(
                "this presentation uses DRM (ContentProtection); collider does not circumvent DRM"
                    .into(),
            ));
        }
    }
    Ok((main, extra))
}

/// A live-capable segment source for one DASH representation (followed across periods).
pub struct DashSource {
    client: Client,
    mpd_url: Url,
    rep_id: String,
    kind: TrackKind,
    manifest: Option<Manifest>,
}

impl DashSource {
    pub fn new(client: Client, manifest: Manifest, rep: &Rep) -> Self {
        Self {
            client,
            mpd_url: manifest.url.clone(),
            rep_id: rep.id.clone(),
            kind: rep.kind,
            manifest: Some(manifest),
        }
    }

    pub async fn refresh(&mut self) -> Result<Snapshot> {
        let m = match self.manifest.take() {
            Some(m) => m,
            None => {
                let body = self.client.get_bytes(&self.mpd_url, None).await?;
                parse(
                    &self.mpd_url,
                    std::str::from_utf8(&body).map_err(|e| Error::Parse(e.to_string()))?,
                )?
            }
        };
        let now = Utc::now();
        let mut segments = Vec::new();
        let periods = m.reps.iter().map(|r| r.period_idx).max().unwrap_or(0);
        for pi in 0..=periods {
            let rep = m
                .reps
                .iter()
                .find(|r| r.period_idx == pi && r.id == self.rep_id)
                .or_else(|| {
                    m.reps
                        .iter()
                        .filter(|r| r.period_idx == pi && r.kind == self.kind)
                        .max_by_key(|r| r.bandwidth)
                });
            let Some(rep) = rep else { continue };
            if rep.protected {
                return Err(Error::Unsupported(
                    "DRM-protected period encountered".into(),
                ));
            }
            let segs = match &rep.addressing {
                Addressing::Template { .. } => template_segments(rep, m.live, now, &m)?,
                Addressing::List { .. } => list_segments(rep),
                Addressing::Base { .. } => base_segments(&self.client, rep).await?,
            };
            segments.extend(segs);
        }
        // New segments appear once per segment duration regardless of what
        // minimumUpdatePeriod claims (encoders often advertise very long periods).
        let seg_dur = segments
            .last()
            .map(|s| s.duration)
            .filter(|d| *d > 0.0)
            .unwrap_or(2.0);
        let mup = m
            .min_update_period
            .filter(|d| !d.is_zero())
            .map(|d| d.as_secs_f64())
            .unwrap_or(f64::INFINITY);
        let refresh_after = Duration::from_secs_f64(mup.min(seg_dur).clamp(0.5, 10.0));
        Ok(Snapshot {
            segments,
            ended: !m.live,
            refresh_after,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url() -> Url {
        Url::parse("https://cdn.example.com/live/manifest.mpd").unwrap()
    }

    #[test]
    fn template_expansion() {
        assert_eq!(
            expand_template("v/$RepresentationID$/$Number%05d$.m4s", "720p", 7, 0, 1),
            "v/720p/00007.m4s"
        );
        assert_eq!(
            expand_template("$Time$-$Bandwidth$$$.m4s", "x", 0, 9000, 500),
            "9000-500$.m4s"
        );
        assert_eq!(expand_template("plain.m4s", "x", 1, 1, 1), "plain.m4s");
    }

    const STATIC_MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT20S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <BaseURL>media/</BaseURL>
  <Period id="0" start="PT0S">
    <AdaptationSet mimeType="video/mp4" contentType="video" lang="und">
      <SegmentTemplate timescale="1000" duration="4000" initialization="init-$RepresentationID$.m4s" media="chunk-$RepresentationID$-$Number%03d$.m4s" startNumber="1"/>
      <Representation id="v1080" bandwidth="5000000" width="1920" height="1080" codecs="avc1.64002a" frameRate="30000/1001"/>
      <Representation id="v360" bandwidth="600000" width="640" height="360" codecs="avc1.4d401e"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" contentType="audio" lang="sv">
      <Role schemeIdUri="urn:mpeg:dash:role:2011" value="main"/>
      <SegmentTemplate timescale="48000" initialization="a-$RepresentationID$-init.m4s" media="a-$RepresentationID$-$Time$.m4s">
        <SegmentTimeline><S t="0" d="96000" r="1"/><S d="192000"/><S d="96000" r="-1"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="a128" bandwidth="128000" codecs="mp4a.40.2"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" contentType="audio" lang="en">
      <SegmentTemplate timescale="48000" duration="96000" initialization="e-init.m4s" media="e-$Number$.m4s"/>
      <Representation id="a64" bandwidth="64000" codecs="mp4a.40.2"/>
    </AdaptationSet>
  </Period>
</MPD>"#;

    #[test]
    fn static_template_number_and_timeline() {
        let m = parse(&url(), STATIC_MPD).unwrap();
        assert!(!m.live);
        assert_eq!(m.reps.len(), 4);
        let info = describe(&m);
        assert_eq!(info.variants.len(), 2);
        assert_eq!(info.audio.len(), 2);
        assert!((info.variants[0].frame_rate.unwrap() - 29.97).abs() < 0.01);

        let (v, a) = select(&m, &Quality::MaxHeight(720), None).unwrap();
        assert_eq!(v.id, "v360");
        assert_eq!(
            a.as_ref().unwrap().id,
            "a128",
            "Role=main audio wins by default"
        );
        let (_, a_en) = select(&m, &Quality::Best, Some("en")).unwrap();
        assert_eq!(a_en.unwrap().id, "a64");

        let segs = template_segments(&v, false, Utc::now(), &m).unwrap();
        assert_eq!(segs.len(), 5);
        assert_eq!(
            segs[0].url.as_str(),
            "https://cdn.example.com/live/media/chunk-v360-001.m4s"
        );
        assert_eq!(
            segs[0].init.as_ref().unwrap().url.as_str(),
            "https://cdn.example.com/live/media/init-v360.m4s"
        );
        assert_eq!(segs[4].duration, 4.0);

        let a = a.unwrap();
        let segs = template_segments(&a, false, Utc::now(), &m).unwrap();
        // 2s, 2s, 4s, then 2s repeated until 20s: 12s left → 6 more.
        assert_eq!(segs.len(), 9);
        assert_eq!(
            segs[2].url.as_str(),
            "https://cdn.example.com/live/media/a-a128-192000.m4s"
        );
        assert_eq!(segs[2].duration, 4.0);
        assert_eq!(
            segs[8].url.as_str(),
            "https://cdn.example.com/live/media/a-a128-864000.m4s"
        );
    }

    #[test]
    fn live_edge_window() {
        let mpd = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" timeShiftBufferDepth="PT10S" minimumUpdatePeriod="PT2S" profiles="x">
  <Period id="1" start="PT0S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="2" media="s$Number$.m4s" startNumber="1"/>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>
</MPD>"#;
        let m = parse(&url(), mpd).unwrap();
        assert!(m.live);
        let rep = &m.reps[0];
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:01:01Z")
            .unwrap()
            .with_timezone(&Utc);
        // 61s elapsed → 30 full segments produced, last complete index 29 → number 30.
        // Window of 10s → earliest index floor(51/2)=25 → number 26.
        let segs = template_segments(rep, true, now, &m).unwrap();
        let numbers: Vec<String> = segs
            .iter()
            .map(|s| s.url.path().rsplit('/').next().unwrap().to_string())
            .collect();
        assert_eq!(
            numbers,
            ["s26.m4s", "s27.m4s", "s28.m4s", "s29.m4s", "s30.m4s"]
        );
    }

    #[test]
    fn segment_list_base_and_drm() {
        let mpd = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT6S" profiles="x">
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <Representation id="l" bandwidth="1" width="10" height="10">
        <SegmentList timescale="1" duration="3">
          <Initialization sourceURL="init.mp4" range="0-99"/>
          <SegmentURL media="s1.m4s"/><SegmentURL media="all.mp4" mediaRange="100-199"/>
        </SegmentList>
      </Representation>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4">
      <Representation id="b" bandwidth="1">
        <BaseURL>single.mp4</BaseURL>
        <SegmentBase indexRange="800-999"><Initialization range="0-799"/></SegmentBase>
      </Representation>
    </AdaptationSet>
  </Period>
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"/>
      <SegmentTemplate media="x$Number$.m4s" duration="1"/>
      <Representation id="l" bandwidth="1"/>
    </AdaptationSet>
  </Period>
</MPD>"#;
        let m = parse(&url(), mpd).unwrap();
        let l = m
            .reps
            .iter()
            .find(|r| r.id == "l" && r.period_idx == 0)
            .unwrap();
        let segs = list_segments(l);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[1].byte_range, Some((100, 100)));
        assert_eq!(segs[0].init.as_ref().unwrap().byte_range, Some((0, 100)));
        assert_eq!(segs[0].duration, 3.0);
        let b = m.reps.iter().find(|r| r.id == "b").unwrap();
        assert!(matches!(
            b.addressing,
            Addressing::Base {
                index_range: Some((800, 200)),
                init_range: Some((0, 800))
            }
        ));
        assert_eq!(b.base.as_str(), "https://cdn.example.com/live/single.mp4");
        // Period 2 is DRM protected; first-period selection still succeeds, describe() flags it.
        assert!(describe(&m).drm);
        let p2 = m.reps.iter().find(|r| r.period_idx == 1).unwrap();
        assert!(p2.protected);
        assert_eq!(p2.period_start, 6.0);
    }
}
