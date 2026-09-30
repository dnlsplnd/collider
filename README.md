# collider

**High-performance, resumable HLS and MPEG-DASH stream downloader and live recorder, written in Rust.**

collider treats stream capture as an engineering problem: parallel segment fetching,
crash-safe resume, correct decryption of clear-key AES-128, live-playlist following,
and lossless remuxing. It is built as a reusable engine (`collider-core`) with a
command-line front end (`collider`).

## Features

- **HLS and MPEG-DASH.** The protocol is detected from the manifest itself. DASH support covers `SegmentTemplate` (`$Number$`/`$Time$`, with or without `SegmentTimeline`), `SegmentList`, single-file `SegmentBase` presentations indexed via `sidx`, multi-period MPDs and `BaseURL` inheritance.
- **Parallel downloads.** Segments are fetched concurrently (`-j`), with retry and exponential backoff on network errors, `429` and `5xx`. Servers that ignore `Range` requests are handled transparently.
- **Crash-safe resume.** Each segment is written atomically, so rerunning an interrupted command only fetches what is missing.
- **Master playlist support.** Pick a variant with `-q best|worst|720p`. Separate audio renditions are selected by language (`--audio-lang`) and muxed in.
- **Live recording.** collider follows live HLS playlists and dynamic MPDs, captures the DVR window, and stops on Ctrl-C or after `--max-duration` while keeping everything recorded. A playlist that disappears at stream end, or a network outage, ends the capture with everything saved. Gaps are reported.
- **Watch mode.** `--wait` polls until a stream exists and has segments, then records it: start it before the broadcast begins.
- **Formats.** MPEG-TS and fragmented MP4 (`EXT-X-MAP`) segments, byte-range playlists, and packed audio.
- **Clear-key AES-128.** Keys are carried forward across segments and IVs are derived from sequence numbers as the spec requires.
- **Lossless output.** ffmpeg remuxes with stream copy and never re-encodes. The container follows the output extension (`.mp4`, `.mkv`, ...).

## Install

```sh
cargo install --path crates/collider-cli
```

ffmpeg is optional but recommended. Without it, collider saves the raw `.ts`/`.mp4` stream files.

## Usage

```sh
# Inspect what a stream offers (add --json for scripting)
collider info https://example.com/master.m3u8
collider info https://example.com/manifest.mpd --json

# Download the best variant
collider get https://example.com/master.m3u8 -o show.mp4

# 720p max, Swedish audio, 16 parallel segments, custom headers
collider get URL -q 720p --audio-lang sv -j 16 -H "Referer: https://example.com" -o show.mkv

# Record a live stream for one hour (or until Ctrl-C)
collider get https://example.com/live.m3u8 -o live.mp4 --max-duration 3600

# Wait for a broadcast to start (checking every 20 s), then record it
collider get https://example.com/live/manifest.mpd -o show.mp4 --wait 20
```

Pressing Ctrl-C once stops gracefully. For a live stream the capture is saved. For a VOD
download the finished segments are kept, and running the same command again resumes.
A second Ctrl-C quits immediately.

collider will not overwrite an existing output file unless you pass `--force`. It also
refuses to resume into a `.parts` directory left behind by a different stream, variant or
audio rendition. `--timeout` is a stall timeout: long segment transfers are fine as long as
data keeps arriving.

## Scope and responsible use

collider downloads streams that are served without technical protection measures.
It **does not and will not** circumvent DRM (Widevine, PlayReady, FairPlay, or SAMPLE-AES
with protected key formats). Such playlists are rejected with a clear error. Only use
collider for content you have the right to save, and respect the terms of the services
you access.

## Roadmap

- [x] HLS engine: parallel, resumable, AES-128, fMP4, byte ranges, alternate audio
- [x] Live recording with graceful stop, duration limits and gap reports
- [x] MPEG-DASH: templates, timelines, lists, single-file `sidx`, multi-period, live
- [x] Watch mode (`--wait`) that records a stream as soon as it goes live
- [ ] Pipelined live capture (download while polling) and clock-skew correction via `UTCTiming`
- [ ] Bandwidth limiting and per-host connection caps
- [ ] Subtitles (WebVTT / TTML renditions) and chapters
- [ ] Multiple URLs and channel lists for `--wait`
- [ ] Daemon with a REST/gRPC API, job queue, TUI and web dashboard
- [ ] Plugin system for site extractors (yt-dlp as a first backend)
- [ ] Prometheus metrics and structured JSON logs

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build && scripts/e2e.sh   # end-to-end: ffmpeg-generated HLS/DASH fixtures, live streams, resume, watch mode
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
