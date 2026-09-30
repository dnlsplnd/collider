//! MPEG-DASH: MPD parsing, representation selection and segment enumeration.
//!
//! Supports SegmentTemplate (`$Number$` and `$Time$`, with or without SegmentTimeline),
//! SegmentList, and SegmentBase single-file representations indexed by `sidx`.
//! Static and dynamic (live) presentations, multi-period, and BaseURL inheritance are handled.
//! Presentations with `ContentProtection` (DRM) are rejected.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dash_mpd::{AdaptationSet, Period, Representation, SegmentTemplate, MPD};
use url::Url;

use crate::error::{Error, Result};
use crate::hls::Quality;
use crate::http::Client;
use crate::model::{
    parse_range, AudioInfo, InitSection, Protocol, Segment, Snapshot, StreamInfo, TrackKind,
    VariantInfo, PERIOD_SHIFT,
};
use crate::sidx;

/// A representation resolved within one period, with all inheritance applied.
#[derive(Debug, Clone)]
pub struct Rep {
    /// Position of the period in this MPD document (changes when a live MPD drops periods).
    pub period_idx: usize,
    /// Identity of the period across MPD refreshes: `Period@id`, else its start time.
    pub period_key: String,
    /// Namespace for this period's segment seqs. `parse` sets the positional index;
    /// `DashSource` overrides it with an ordinal that stays stable across refreshes.
    pub period_ord: u64,
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
        /// Media time (in `timescale` ticks) at which the period starts.
        presentation_time_offset: u64,
        /// Seconds; `f64::INFINITY` for `availabilityTimeOffset="INF"`.
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
    let mut period_keys = HashSet::new();
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
        let mut period_key = match &period.id {
            Some(id) => format!("id:{id}"),
            None => format!("start:{}", (start * 1000.0).round() as i64),
        };
        if !period_keys.insert(period_key.clone()) {
            // Duplicate ids (invalid, but seen) must not share a seq namespace.
            period_key = format!("{period_key}#{pi}");
        }
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
                    period_key: period_key.clone(),
                    period_ord: pi as u64,
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
            presentation_time_offset: t.presentationTimeOffset.unwrap_or(0),
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
                key: None,
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

/// Pack a period ordinal and a per-period segment key (number, media time or hash) into a seq.
fn seq_id(period_ord: u64, key: u64) -> u64 {
    (period_ord << PERIOD_SHIFT) | (key & ((1 << PERIOD_SHIFT) - 1))
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
        presentation_time_offset,
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
            key: None,
        }),
        None => None,
    };
    let ts = *timescale as f64;
    let period_duration = rep.period_duration.filter(|d| d.is_finite() && *d >= 0.0);
    // `key` is the dedup/resume identity within the period; `number` and `time` fill the URL.
    let mk = |key: u64, number: u64, time: u64, dur: f64| -> Result<Segment> {
        Ok(Segment {
            seq: seq_id(rep.period_ord, key),
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
            optional: false,
            discontinuity: false,
        })
    };
    let mut out = Vec::new();

    if let Some(tl) = timeline {
        // S@t is media time; the period starts at presentationTimeOffset (ISO 23009-1 5.3.9.6).
        let pto = *presentation_time_offset;
        let first_t = tl.first().and_then(|e| e.t).unwrap_or(0);
        let end = period_duration
            .map(|d| pto.saturating_add((d * ts).round() as u64))
            // A timeline that starts at or past the period end means PTO is missing or wrong:
            // keep what the timeline lists rather than dropping everything.
            .filter(|end| first_t < *end);
        let mut number = *start_number;
        let mut time = 0u64;
        for (i, e) in tl.iter().enumerate() {
            if let Some(t) = e.t {
                time = t;
            }
            if e.d == 0 {
                continue;
            }
            let repeats = if e.r >= 0 {
                e.r as u64
            } else {
                // Repeat until the next entry's start, else until the period ends.
                match tl.get(i + 1).and_then(|n| n.t).or(end) {
                    Some(limit) if limit > time => (limit - time).div_ceil(e.d) - 1,
                    _ => 0,
                }
            };
            for _ in 0..=repeats {
                if end_number.is_some_and(|n| number > n) || end.is_some_and(|end| time >= end) {
                    break;
                }
                // Keyed by media time: live packagers slide the window without changing
                // startNumber, so positions (and $Number$) are not stable across refreshes.
                out.push(mk(time, number, time, e.d as f64 / ts)?);
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
    // Segments the period can hold; the last one is truncated when the duration is not a
    // multiple of the segment duration.
    let period_count = period_duration.map(|d| (d / seg_secs).ceil() as i64);
    let by_number = end_number.map(|e| e as i64 - *start_number as i64 + 1);
    let (first, last) = if live {
        let ast = m
            .availability_start
            .ok_or_else(|| Error::Parse("dynamic MPD without availabilityStartTime".into()))?;
        let since_start =
            (now - ast).to_std().map(|d| d.as_secs_f64()).unwrap_or(0.0) - rep.period_start;
        let ato = *availability_time_offset;
        let (elapsed, latest) = if ato.is_finite() {
            let elapsed = since_start + ato;
            (elapsed, (elapsed / seg_secs).floor() as i64 - 1)
        } else {
            // availabilityTimeOffset="INF": everything up to the period end is available.
            let latest = match period_count {
                Some(c) => c - 1,
                None => (since_start / seg_secs).floor() as i64 - 1,
            };
            (since_start, latest)
        };
        // A period that has ended (or ends at a known time) never grows past its end.
        let latest = period_count.map_or(latest, |c| latest.min(c - 1));
        let earliest = match m.time_shift_buffer.map(|d| d.as_secs_f64()) {
            Some(depth) if depth.is_finite() => {
                ((elapsed - depth) / seg_secs).floor().max(0.0) as i64
            }
            _ => 0,
        };
        (earliest, latest)
    } else {
        let count = match (by_number, period_count) {
            (Some(n), Some(c)) => n.min(c),
            (Some(n), None) => n,
            (None, Some(c)) => c,
            (None, None) => {
                return Err(Error::Parse(
                    "static SegmentTemplate without a period duration or endNumber".into(),
                ))
            }
        };
        (0, count - 1)
    };
    let cap = by_number.map(|n| n - 1).unwrap_or(i64::MAX);
    for idx in first.max(0)..=last.min(cap) {
        let number = *start_number + idx as u64;
        let dur_secs = match period_duration {
            Some(d) => (d - idx as f64 * seg_secs).min(seg_secs).max(0.0),
            None => seg_secs,
        };
        let mut seg = mk(number, number, (idx as f64 * dur) as u64, dur_secs)?;
        // The duration-derived last segment may not exist: MPD durations usually follow the
        // longest track, so a shorter track has one segment fewer. endNumber settles it.
        seg.optional = period_count == Some(idx + 1)
            && dur_secs < seg_secs
            && !matches!(by_number, Some(n) if n <= idx + 1);
        out.push(seg);
    }
    Ok(out)
}

/// Stable 40-bit key for a list entry that has no number of its own.
fn list_key(url: &Url, range: Option<(u64, u64)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.as_str().hash(&mut h);
    range.hash(&mut h);
    h.finish()
}

/// Segments of a SegmentList representation. Live lists slide, so their entries are keyed
/// by URL and byte range rather than by position.
pub fn list_segments(rep: &Rep, live: bool) -> Vec<Segment> {
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
            seq: seq_id(
                rep.period_ord,
                if live {
                    list_key(url, *range)
                } else {
                    i as u64
                },
            ),
            url: url.clone(),
            duration: duration.unwrap_or(0.0),
            byte_range: *range,
            key: None,
            init: init.clone(),
            optional: false,
            discontinuity: false,
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
            seq: seq_id(rep.period_ord, 0),
            url: rep.base.clone(),
            duration: rep.period_duration.unwrap_or(0.0),
            byte_range: None,
            key: None,
            init: None,
            optional: false,
            discontinuity: false,
        }]);
    };
    let index = client
        .get_bytes(&rep.base, Some((*idx_off, *idx_len)))
        .await?;
    let subs = sidx::parse_sidx(&index, *idx_off)?;
    let init = Some(InitSection {
        url: rep.base.clone(),
        byte_range: Some(init_range.unwrap_or((0, *idx_off))),
        key: None,
    });
    Ok(subs
        .into_iter()
        .enumerate()
        .map(|(i, s)| Segment {
            seq: seq_id(rep.period_ord, i as u64),
            url: rep.base.clone(),
            duration: s.duration,
            byte_range: Some((s.offset, s.size)),
            key: None,
            init: init.clone(),
            optional: false,
            discontinuity: false,
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

/// The highest-ranked video representation among `cands` allowed by `quality`.
fn pick_video<'a>(cands: &[&'a Rep], quality: &Quality) -> Option<&'a Rep> {
    let rank = |r: &&&Rep| (r.height.unwrap_or(0), r.bandwidth);
    match quality {
        Quality::Best => cands.iter().max_by_key(rank),
        Quality::Worst => cands.iter().min_by_key(rank),
        Quality::MaxHeight(h) => cands
            .iter()
            .filter(|r| r.height.unwrap_or(0) <= *h)
            .max_by_key(rank),
    }
    .copied()
}

/// The best audio representation of the AdaptationSet matching `lang`, else the `Role=main`
/// set, else the first set.
fn pick_audio<'a>(cands: &[&'a Rep], lang: Option<&str>) -> Option<&'a Rep> {
    let chosen_set = lang
        .and_then(|l| {
            cands
                .iter()
                .find(|r| r.lang.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(l)))
        })
        .or_else(|| cands.iter().find(|r| r.default_audio))
        .or_else(|| cands.first())
        .map(|r| r.adaptation_id.clone())?;
    cands
        .iter()
        .filter(|r| r.adaptation_id == chosen_set)
        .max_by_key(|r| r.bandwidth)
        .copied()
}

/// `avc1.64002a` → `avc1`: representations of one family can be stream-copied together.
fn codec_family(codecs: &str) -> String {
    codecs
        .split([',', '.'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
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

    let (main, extra) = if video.is_empty() {
        (
            pick_audio(&audio, audio_lang)
                .ok_or_else(|| Error::Parse("MPD has no video or audio representations".into()))?,
            None,
        )
    } else {
        (
            pick_video(&video, quality).ok_or_else(|| Error::NoVariant(format!("{quality:?}")))?,
            pick_audio(&audio, audio_lang),
        )
    };
    for r in std::iter::once(main).chain(extra) {
        if r.protected {
            return Err(Error::Unsupported(
                "this presentation uses DRM (ContentProtection); collider does not circumvent DRM"
                    .into(),
            ));
        }
    }
    Ok((main.clone(), extra.cloned()))
}

/// A live-capable segment source for one DASH representation (followed across periods).
pub struct DashSource {
    client: Client,
    mpd_url: Url,
    rep_id: String,
    kind: TrackKind,
    /// The selection, re-applied in periods that do not carry `rep_id`.
    quality: Quality,
    lang: Option<String>,
    height: Option<u64>,
    codec_family: Option<String>,
    /// `Rep::period_key` → seq namespace; survives refreshes so that dropping or adding
    /// periods in a live MPD does not re-key the segments of the others.
    period_ords: HashMap<String, u64>,
    manifest: Option<Manifest>,
}

impl DashSource {
    /// `mpd_url` is the URL to re-request on refresh (the one the user gave, so redirecting
    /// edges can reissue tokens); `manifest` was parsed against the post-redirect URL.
    /// `rep` is the first-period choice made by [`select`] with `quality` and `audio_lang`.
    pub fn new(
        client: Client,
        mpd_url: Url,
        manifest: Manifest,
        rep: &Rep,
        quality: &Quality,
        audio_lang: Option<&str>,
    ) -> Self {
        Self {
            client,
            mpd_url,
            rep_id: rep.id.clone(),
            kind: rep.kind,
            quality: quality.clone(),
            lang: rep.lang.clone().or_else(|| audio_lang.map(str::to_string)),
            height: rep.height,
            codec_family: rep.codecs.as_deref().map(codec_family),
            period_ords: HashMap::new(),
            manifest: Some(manifest),
        }
    }

    /// The representation of one period to follow: the chosen id when the period has it,
    /// else the same selection applied to that period's representations.
    fn pick<'a>(&self, period: &[&'a Rep]) -> Option<&'a Rep> {
        if let Some(r) = period.iter().find(|r| r.id == self.rep_id) {
            return Some(r);
        }
        let cands: Vec<&Rep> = period
            .iter()
            .copied()
            .filter(|r| r.kind == self.kind)
            .collect();
        // Stay within the codec family so the stream copy can join the periods.
        let same_family: Vec<&Rep> = cands
            .iter()
            .copied()
            .filter(|r| {
                self.codec_family.is_some()
                    && r.codecs.as_deref().map(codec_family) == self.codec_family
            })
            .collect();
        let cands = if same_family.is_empty() {
            cands
        } else {
            same_family
        };
        match self.kind {
            TrackKind::Video => {
                // Do not exceed the height chosen in the first period; if everything is
                // taller, take the closest one.
                let capped: Vec<&Rep> = cands
                    .iter()
                    .copied()
                    .filter(|r| match (self.height, r.height) {
                        (Some(h), Some(rh)) => rh <= h,
                        _ => true,
                    })
                    .collect();
                pick_video(&capped, &self.quality).or_else(|| {
                    cands
                        .iter()
                        .copied()
                        .min_by_key(|r| (r.height.unwrap_or(0), r.bandwidth))
                })
            }
            _ => pick_audio(&cands, self.lang.as_deref()),
        }
    }

    pub async fn refresh(&mut self) -> Result<Snapshot> {
        let m = match self.manifest.take() {
            Some(m) => m,
            None => {
                // Relative BaseURLs and media URLs resolve against the post-redirect URL.
                let (base, body) = self.client.get_bytes_with_url(&self.mpd_url, None).await?;
                parse(
                    &base,
                    std::str::from_utf8(&body).map_err(|e| Error::Parse(e.to_string()))?,
                )?
            }
        };
        let now = Utc::now();
        let mut segments = Vec::new();
        let periods = m.reps.iter().map(|r| r.period_idx).max().unwrap_or(0);
        for pi in 0..=periods {
            let period: Vec<&Rep> = m.reps.iter().filter(|r| r.period_idx == pi).collect();
            let Some(any) = period.first() else { continue };
            let next = self.period_ords.len() as u64;
            let ord = *self
                .period_ords
                .entry(any.period_key.clone())
                .or_insert(next);
            let Some(rep) = self.pick(&period) else {
                continue;
            };
            if rep.protected {
                return Err(Error::Unsupported(
                    "DRM-protected period encountered".into(),
                ));
            }
            let mut rep = rep.clone();
            rep.period_ord = ord;
            let segs = match &rep.addressing {
                Addressing::Template { .. } => template_segments(&rep, m.live, now, &m)?,
                Addressing::List { .. } => list_segments(&rep, m.live),
                Addressing::Base { .. } => base_segments(&self.client, &rep).await?,
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
        let segs = list_segments(l, false);
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

    fn names(segs: &[Segment]) -> Vec<String> {
        segs.iter()
            .map(|s| s.url.path().rsplit('/').next().unwrap().to_string())
            .collect()
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn source(m: &Manifest, rep: &Rep, quality: &Quality) -> DashSource {
        let client = Client::new(&[], 0, Duration::from_secs(5)).unwrap();
        DashSource::new(client, url(), m.clone(), rep, quality, None)
    }

    fn live_timeline(t: u64) -> String {
        format!(
            r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" minimumUpdatePeriod="PT2S" profiles="x">
  <Period id="p" start="PT0S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" media="s$Time$.m4s" startNumber="1">
        <SegmentTimeline><S t="{t}" d="2" r="4"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>
</MPD>"#
        )
    }

    #[test]
    fn sliding_time_timeline_keeps_seqs_stable() {
        let now = Utc::now();
        let m1 = parse(&url(), &live_timeline(100)).unwrap();
        let m2 = parse(&url(), &live_timeline(102)).unwrap();
        let a = template_segments(&m1.reps[0], true, now, &m1).unwrap();
        let b = template_segments(&m2.reps[0], true, now, &m2).unwrap();
        assert_eq!(names(&b).last().unwrap(), "s110.m4s");
        let seen: HashSet<u64> = a.iter().map(|s| s.seq).collect();
        let fresh: Vec<&Segment> = b.iter().filter(|s| !seen.contains(&s.seq)).collect();
        assert_eq!(fresh.len(), 1, "only s110 is new");
        assert_eq!(fresh[0].url.path(), "/live/s110.m4s");
        // The same media keeps the same seq across refreshes.
        assert_eq!(a[1].seq, b[0].seq);
    }

    fn two_periods(with_p1: bool) -> String {
        let p1 = r#"<Period id="p1" start="PT0S" duration="PT10S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="2" media="p1-$Number$.m4s" startNumber="1"/>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>"#;
        format!(
            r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2000-01-01T00:00:00Z" publishTime="2000-01-01T00:00:00Z" minimumUpdatePeriod="PT2S" profiles="x">
  {}
  <Period id="p2" start="PT10S" duration="PT10S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="2" media="p2-$Number$.m4s" startNumber="1"/>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>
</MPD>"#,
            if with_p1 { p1 } else { "" }
        )
    }

    #[tokio::test]
    async fn dropping_an_expired_period_does_not_rekey_the_others() {
        let m1 = parse(&url(), &two_periods(true)).unwrap();
        let mut src = source(&m1, &m1.reps[0], &Quality::Best);
        let a = src.refresh().await.unwrap().segments;
        assert_eq!(a.len(), 10);
        src.manifest = Some(parse(&url(), &two_periods(false)).unwrap());
        let b = src.refresh().await.unwrap().segments;
        assert_eq!(names(&b)[0], "p2-1.m4s");
        let seen: HashSet<u64> = a.iter().map(|s| s.seq).collect();
        assert!(b.iter().all(|s| seen.contains(&s.seq)), "p2 keeps its seqs");
        // p1-1 and p2-1 share $Number$ but never a seq.
        assert_ne!(a[0].seq, b[0].seq);
    }

    #[test]
    fn live_number_edge_is_bounded_by_the_period() {
        // Closed period p1 (PT10S) keeps exactly its five segments long after it ended.
        let m = parse(&url(), &two_periods(true)).unwrap();
        let p1 = m.reps.iter().find(|r| r.period_idx == 0).unwrap();
        let segs = template_segments(p1, true, Utc::now(), &m).unwrap();
        assert_eq!(names(&segs).last().unwrap(), "p1-5.m4s");
        assert_eq!(segs.len(), 5);

        let inf = |tsb: &str, dur: &str| {
            format!(
                r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" {tsb} profiles="x">
  <Period id="1" start="PT0S" {dur}>
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="2" availabilityTimeOffset="INF" media="s$Number$.m4s"/>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>
</MPD>"#
            )
        };
        let now = at("2026-01-01T00:01:01Z");
        // INF with a known period end: the whole period is available.
        let m = parse(&url(), &inf("", r#"duration="PT20S""#)).unwrap();
        let segs = template_segments(&m.reps[0], true, now, &m).unwrap();
        assert_eq!(segs.len(), 10);
        // INF without a period end: up to now, and never unbounded.
        let m = parse(&url(), &inf(r#"timeShiftBufferDepth="PT30S""#, "")).unwrap();
        let segs = template_segments(&m.reps[0], true, now, &m).unwrap();
        assert_eq!(names(&segs).first().unwrap(), "s16.m4s");
        assert_eq!(names(&segs).last().unwrap(), "s30.m4s");
        let m = parse(&url(), &inf("", "")).unwrap();
        assert_eq!(
            template_segments(&m.reps[0], true, now, &m).unwrap().len(),
            30
        );
    }

    fn static_timeline(pto: u64, timeline: &str) -> Manifest {
        let mpd = format!(
            r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT20S" profiles="x">
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="90000" presentationTimeOffset="{pto}" media="s$Time$-$Number$.m4s">
        <SegmentTimeline>{timeline}</SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>
</MPD>"#
        );
        parse(&url(), &mpd).unwrap()
    }

    #[test]
    fn open_ended_timeline_repeat_honours_pto_and_next_entry() {
        let m = static_timeline(900000, r#"<S t="900000" d="180000" r="-1"/>"#);
        let segs = template_segments(&m.reps[0], false, Utc::now(), &m).unwrap();
        assert_eq!(segs.len(), 10, "20 s of 2 s segments after the PTO");
        assert_eq!(names(&segs)[9], "s2520000-10.m4s");

        let m = static_timeline(
            0,
            r#"<S t="0" d="180000" r="-1"/><S t="900000" d="450000" r="1"/>"#,
        );
        let segs = template_segments(&m.reps[0], false, Utc::now(), &m).unwrap();
        assert_eq!(
            names(&segs),
            [
                "s0-1.m4s",
                "s180000-2.m4s",
                "s360000-3.m4s",
                "s540000-4.m4s",
                "s720000-5.m4s",
                "s900000-6.m4s",
                "s1350000-7.m4s"
            ]
        );
        // Entries past the period end are not emitted.
        let m = static_timeline(0, r#"<S t="0" d="900000" r="3"/>"#);
        let segs = template_segments(&m.reps[0], false, Utc::now(), &m).unwrap();
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn static_number_count_uses_end_number_and_marks_overhang_optional() {
        let mpd = |attrs: &str, extra: &str| {
            format!(
                r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" {attrs} profiles="x">
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="2" startNumber="1" {extra} media="s$Number$.m4s"/>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>
</MPD>"#
            )
        };
        let segs_of = |body: String| {
            let m = parse(&url(), &body).unwrap();
            template_segments(&m.reps[0], false, Utc::now(), &m)
        };
        let segs = segs_of(mpd("", r#"endNumber="10""#)).unwrap();
        assert_eq!(segs.len(), 10);
        assert!(segs.iter().all(|s| !s.optional));
        assert!(matches!(segs_of(mpd("", "")), Err(Error::Parse(_))));

        // 10.3 s of presentation: a sixth, 0.3 s segment only if this track has one.
        let segs = segs_of(mpd(r#"mediaPresentationDuration="PT10.3S""#, "")).unwrap();
        assert_eq!(segs.len(), 6);
        assert!(segs[5].optional && !segs[4].optional);
        assert!((segs[5].duration - 0.3).abs() < 1e-9);
        // An exact multiple has no optional segment, and endNumber settles the question.
        let segs = segs_of(mpd(r#"mediaPresentationDuration="PT10S""#, "")).unwrap();
        assert!(segs.iter().all(|s| !s.optional));
        let segs = segs_of(mpd(
            r#"mediaPresentationDuration="PT10.3S""#,
            r#"endNumber="6""#,
        ))
        .unwrap();
        assert_eq!(segs.len(), 6);
        assert!(!segs[5].optional);
    }

    #[tokio::test]
    async fn later_periods_reapply_the_selection() {
        let mpd = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT20S" profiles="x">
  <Period id="main" duration="PT10S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>
      <Representation id="v1080" bandwidth="5000000" height="1080" codecs="avc1.64002a"/>
      <Representation id="v360" bandwidth="600000" height="360" codecs="avc1.4d401e"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="sv">
      <SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>
      <Representation id="sv" bandwidth="128000" codecs="mp4a.40.2"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="en">
      <Role schemeIdUri="urn:mpeg:dash:role:2011" value="main"/>
      <SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>
      <Representation id="en" bandwidth="128000" codecs="mp4a.40.2"/>
    </AdaptationSet>
  </Period>
  <Period id="ad" duration="PT10S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>
      <Representation id="ad_hi" bandwidth="6000000" height="1080" codecs="avc1.64002a"/>
      <Representation id="ad_lo" bandwidth="500000" height="360" codecs="avc1.4d401e"/>
      <Representation id="ad_hevc" bandwidth="400000" height="360" codecs="hvc1.1.6.L93"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="en">
      <Role schemeIdUri="urn:mpeg:dash:role:2011" value="main"/>
      <SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>
      <Representation id="ad_en" bandwidth="256000" codecs="mp4a.40.2"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="sv">
      <SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>
      <Representation id="ad_sv" bandwidth="96000" codecs="mp4a.40.2"/>
    </AdaptationSet>
  </Period>
</MPD>"#;
        let m = parse(&url(), mpd).unwrap();
        let q = Quality::MaxHeight(360);
        let (v, a) = select(&m, &q, Some("sv")).unwrap();
        assert_eq!(v.id, "v360");
        let segs = source(&m, &v, &q).refresh().await.unwrap().segments;
        assert_eq!(names(&segs)[5], "ad_lo-1.m4s");
        let a = a.unwrap();
        assert_eq!(a.id, "sv");
        let segs = source(&m, &a, &q).refresh().await.unwrap().segments;
        assert_eq!(names(&segs)[5], "ad_sv-1.m4s", "language is kept");
    }
}
