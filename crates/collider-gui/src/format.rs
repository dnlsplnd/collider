//! Number, size, rate and time formatting in the style of the design spec.

use std::path::{Path, PathBuf};

use url::Url;

/// Binary size with three significant digits: `412 GiB`, `66.6 MiB`, `3.15 GiB`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    format!("{} {}", sig3(v), UNITS[unit])
}

/// The value part of [`bytes`] and its unit, for a large number with a small unit label.
pub fn bytes_split(n: u64) -> (String, &'static str) {
    let s = bytes(n);
    let (v, u) = s.split_once(' ').unwrap_or((&s, ""));
    let unit = match u {
        "B" => "B",
        "KiB" => "KiB",
        "MiB" => "MiB",
        "GiB" => "GiB",
        "TiB" => "TiB",
        _ => "PiB",
    };
    (v.to_string(), unit)
}

/// Transfer speed in bytes per second: `48.0 MiB/s`.
pub fn speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1.0 {
        return "0 B/s".into();
    }
    format!("{}/s", bytes(bytes_per_sec as u64))
}

/// A bitrate: `6.2 Mbit/s`, `128 kbit/s`.
pub fn bitrate(bits_per_sec: f64) -> String {
    let (v, unit) = bitrate_split(bits_per_sec);
    format!("{v} {unit}")
}

pub fn bitrate_split(bits_per_sec: f64) -> (String, &'static str) {
    if bits_per_sec >= 1e6 {
        (format!("{:.1}", bits_per_sec / 1e6), "Mbit/s")
    } else {
        (format!("{:.0}", bits_per_sec / 1e3), "kbit/s")
    }
}

fn sig3(v: f64) -> String {
    if v < 10.0 {
        format!("{v:.2}")
    } else if v < 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    }
}

/// `4362` as `4 362`, with a thin space for proportional text.
pub fn count(n: usize) -> String {
    group(n, '\u{2009}')
}

/// `4362` as `4 362`, with a plain space for monospace text.
pub fn count_mono(n: usize) -> String {
    group(n, ' ')
}

fn group(n: usize, sep: char) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// `01:12:44`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// `00:24` below an hour, `1:02:03` above.
pub fn short_duration(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Parse `hh:mm:ss`, `mm:ss` or plain seconds.
pub fn parse_duration(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut total = 0.0;
    for part in text.split(':') {
        let v: f64 = part.trim().parse().ok().filter(|v: &f64| *v >= 0.0)?;
        total = total * 60.0 + v;
    }
    (total > 0.0).then_some(total)
}

pub fn host(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// The manifest's file name: `master.m3u8`.
pub fn manifest_file(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(str::to_string))
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// A readable name for a stream: the manifest's file stem, or its folder when the file is
/// a generic `index.m3u8`, `master.m3u8` or `manifest.mpd`.
pub fn stream_name(url: &str) -> String {
    const GENERIC: [&str; 12] = [
        "index",
        "master",
        "manifest",
        "playlist",
        "prog_index",
        "stream",
        "chunklist",
        "main",
        "media",
        "video",
        "live",
        "mpd",
    ];
    let Ok(u) = Url::parse(url) else {
        return "stream".into();
    };
    let segments: Vec<String> = u
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(decode).collect())
        .unwrap_or_default();
    for seg in segments.iter().rev() {
        let stem = seg.rsplit_once('.').map_or(seg.as_str(), |(s, _)| s);
        if GENERIC.iter().any(|g| stem.eq_ignore_ascii_case(g)) {
            continue;
        }
        let name = sanitize(stem);
        if !name.is_empty() {
            return name;
        }
    }
    let host = sanitize(u.host_str().unwrap_or_default());
    if host.is_empty() {
        "stream".into()
    } else {
        host
    }
}

/// Percent-decode a URL path segment.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A file-name-safe version of `name`: no separators, control or reserved characters.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    cleaned
        .trim()
        .trim_matches('.')
        .trim_start_matches('-')
        .chars()
        .take(120)
        .collect()
}

/// `dir/stem.ext`, or `dir/stem-1.ext`, `dir/stem-2.ext` ... when that output exists.
/// Only finished outputs are skipped: a leftover `stem.ext.parts` directory from an
/// interrupted download of the same stream makes the new download resume it.
pub fn unique_output(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let candidate = |n: usize| {
        if n == 0 {
            dir.join(format!("{stem}.{ext}"))
        } else {
            dir.join(format!("{stem}-{n}.{ext}"))
        }
    };
    (0..)
        .map(candidate)
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// `~/Videos/collider` for paths under the home directory.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", rest.display())
            };
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_rates_and_counts() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.50 KiB");
        assert_eq!(bytes(69_835_161), "66.6 MiB");
        assert_eq!(bytes(3_382_286_745), "3.15 GiB");
        assert_eq!(bytes(442_381_631_488), "412 GiB");
        assert_eq!(bitrate(6_200_000.0), "6.2 Mbit/s");
        assert_eq!(bitrate(128_000.0), "128 kbit/s");
        assert_eq!(count_mono(4362), "4 362");
        assert_eq!(count(1_204), "1\u{2009}204");
        assert_eq!(count(999), "999");
        assert_eq!(clock(4364.0), "01:12:44");
        assert_eq!(short_duration(24.0), "00:24");
        assert_eq!(short_duration(3723.0), "1:02:03");
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse_duration("1:00:00"), Some(3600.0));
        assert_eq!(parse_duration("90"), Some(90.0));
        assert_eq!(parse_duration("2:30"), Some(150.0));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("x"), None);
        assert_eq!(parse_duration("0"), None);
    }

    #[test]
    fn names_from_urls() {
        let n = stream_name;
        assert_eq!(
            n("https://cdn.example/tears-of-steel/master.m3u8"),
            "tears-of-steel"
        );
        assert_eq!(n("https://media.example/sintel-4k.mpd"), "sintel-4k");
        assert_eq!(n("https://v.example/lec07/index.m3u8?token=1"), "lec07");
        assert_eq!(n("https://v.example/my%20show/index.m3u8"), "my show");
        assert_eq!(n("https://live.example/index.m3u8"), "live.example");
        assert_eq!(
            manifest_file("https://a.example/x/master.m3u8?t=1"),
            "master.m3u8"
        );
        assert_eq!(host("https://a.example:8080/x"), "a.example");
        assert_eq!(sanitize("../a:b/c"), "a-b-c");
    }

    #[test]
    fn unique_output_skips_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(unique_output(dir, "show", "mp4"), dir.join("show.mp4"));
        std::fs::write(dir.join("show.mp4"), b"x").unwrap();
        std::fs::write(dir.join("show-1.mp4"), b"x").unwrap();
        assert_eq!(unique_output(dir, "show", "mp4"), dir.join("show-2.mp4"));
    }
}
