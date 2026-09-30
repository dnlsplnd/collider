#!/usr/bin/env bash
# End-to-end tests for collider.
#
# Generates HLS and DASH fixtures with ffmpeg, serves them from a local HTTP server,
# downloads them with collider and verifies the results with ffprobe/ffmpeg.
#
# Requirements: ffmpeg, ffprobe, python3, a built collider binary (cargo build).
# Environment:
#   COLLIDER       path to the binary (default: target/debug/collider)
#   E2E_PORT       base port (default 8791; the next port is used for a Range-ignoring server)
#   E2E_DIR        work directory (default: a temp dir, deleted on success)
#   E2E_SKIP_LIVE  set to 1 to skip the live-stream tests (~40 s)
set -uo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BIN=${COLLIDER:-$ROOT/target/debug/collider}
PORT=${E2E_PORT:-8791}
PORT_NORANGE=$((PORT + 1))
WORK=${E2E_DIR:-$(mktemp -d -t collider-e2e.XXXXXX)}
BASE="http://127.0.0.1:$PORT"
KEEP_WORK=${E2E_DIR:+1}

pass=0
fail=0
pids=()

cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
  if [ "$fail" -eq 0 ] && [ -z "$KEEP_WORK" ]; then rm -rf "$WORK"; fi
}
trap cleanup EXIT

ok() { pass=$((pass + 1)); echo "PASS  $1"; }
bad() { fail=$((fail + 1)); echo "FAIL  $1"; [ -f "$WORK/last.log" ] && sed 's/^/      | /' "$WORK/last.log" | tail -5; }
check() { local desc=$1; shift; if "$@"; then ok "$desc"; else bad "$desc"; fi; }

run() { "$BIN" "$@" >/dev/null 2>"$WORK/last.log"; }
decodes() { local out; out=$(ffmpeg -v error -i "$1" -f null - 2>&1); [ -z "$out" ]; }
duration() { ffprobe -v error -show_entries format=duration -of csv=p=0 "$1" 2>/dev/null; }
width() { ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$1" 2>/dev/null; }
has_audio() { [ -n "$(ffprobe -v error -select_streams a:0 -show_entries stream=codec_type -of csv=p=0 "$1" 2>/dev/null)" ]; }
near() { awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d <= t) }'; }
packets() { ffprobe -v error -show_packets -show_entries packet=stream_index,pts,size -of csv "$1" 2>/dev/null; }
wait_http() { for _ in $(seq 1 50); do curl -fs -o /dev/null "$1" && return 0; sleep 0.2; done; return 1; }

command -v ffmpeg >/dev/null || { echo "ffmpeg is required"; exit 2; }
command -v python3 >/dev/null || { echo "python3 is required"; exit 2; }
[ -x "$BIN" ] || { echo "collider binary not found at $BIN (run cargo build)"; exit 2; }

mkdir -p "$WORK/www" && cd "$WORK"
echo "work dir: $WORK"

# ---------------------------------------------------------------- fixtures
SRC=(-f lavfi -i "testsrc2=size=640x360:rate=30" -f lavfi -i "sine=frequency=440" -t 20)
TSENC=(-c:v mpeg4 -q:v 5 -g 30 -bsf:v dump_extra -c:a aac)
MP4ENC=(-c:v mpeg4 -q:v 5 -g 30 -c:a aac)
FF=(ffmpeg -hide_banner -loglevel error -y)

mkdir -p www/enc www/fmp4 www/dash-tl www/dash-num www/dash-single
head -c 16 /dev/urandom >www/enc/key.bin
printf '%s/enc/key.bin\n%s/www/enc/key.bin\n' "$BASE" "$WORK" >keyinfo
"${FF[@]}" "${SRC[@]}" "${TSENC[@]}" -hls_time 2 -hls_playlist_type vod -hls_key_info_file keyinfo \
  -hls_segment_filename www/enc/seg%03d.ts www/enc/index.m3u8
"${FF[@]}" "${SRC[@]}" -map 0:v -map 0:v -map 1:a "${MP4ENC[@]}" -s:v:1 320x180 -b:v:0 1M -b:v:1 300k \
  -f hls -hls_time 2 -hls_playlist_type vod -hls_segment_type fmp4 -master_pl_name master.m3u8 \
  -var_stream_map "v:0,agroup:aud v:1,agroup:aud a:0,agroup:aud,language:en,name:English,default:yes" \
  -hls_segment_filename 'www/fmp4/%v_%03d.m4s' www/fmp4/%v.m3u8
"${FF[@]}" "${SRC[@]}" -map 0:v -map 0:v -map 1:a "${MP4ENC[@]}" -s:v:1 320x180 -b:v:0 1M -b:v:1 300k \
  -f dash -seg_duration 2 -use_template 1 -use_timeline 1 -adaptation_sets "id=0,streams=v id=1,streams=a" \
  www/dash-tl/manifest.mpd
"${FF[@]}" "${SRC[@]}" -map 0:v -map 1:a "${MP4ENC[@]}" -f dash -seg_duration 2 -use_template 1 -use_timeline 0 \
  www/dash-num/manifest.mpd
"${FF[@]}" "${SRC[@]}" -map 0:v -map 1:a "${MP4ENC[@]}" -f dash -seg_duration 2 -single_file 1 \
  www/dash-single/manifest.mpd
# Audio outlasts video (10.3 s vs 10 s): mediaPresentationDuration implies a sixth video
# segment that does not exist.
mkdir -p www/dash-overhang
"${FF[@]}" -f lavfi -i "testsrc2=size=320x180:rate=30:duration=10" -f lavfi -i "sine=frequency=440:duration=10.3" \
  -map 1:a -map 0:v "${MP4ENC[@]}" -f dash -seg_duration 2 -use_template 1 -use_timeline 0 \
  www/dash-overhang/manifest.mpd

# Two periods whose media timestamps both start at zero (e.g. an ad break stitched in).
mkdir -p www/dash-mp/p1 www/dash-mp/p2
for p in 1 2; do
  "${FF[@]}" -f lavfi -i "testsrc2=size=320x180:rate=30" -f lavfi -i "sine=frequency=$((330 * p))" -t 6 \
    -map 0:v -map 1:a "${MP4ENC[@]}" -f dash -seg_duration 2 -use_template 1 -use_timeline 0 \
    www/dash-mp/p$p/manifest.mpd
done
python3 - www/dash-mp <<'PY'
import re, sys
d = sys.argv[1]
mpd = open(f"{d}/p1/manifest.mpd").read().replace('mediaPresentationDuration="PT6.0S"', 'mediaPresentationDuration="PT12.0S"')
period = re.search(r"<Period.*?</Period>", mpd, re.S).group(0)
periods = "\n".join(
    period.replace('<Period id="0" start="PT0.0S">', f'<Period id="p{i}" start="PT{6 * (i - 1)}.0S">\n<BaseURL>p{i}/</BaseURL>')
    for i in (1, 2)
)
open(f"{d}/manifest.mpd", "w").write(mpd.replace(period, periods))
PY

# Master with an audio-only variant (no RESOLUTION), which must never be picked as video.
printf '#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS="mp4a.40.2"\n2.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=400000,RESOLUTION=320x180\n1.m3u8\n' \
  >www/fmp4/audio-only.m3u8
# Fractional EXT-X-TARGETDURATION (seen in the wild, m3u8-rs only reads integers).
sed 's/^#EXT-X-TARGETDURATION:\([0-9]*\)$/#EXT-X-TARGETDURATION:\1.0/' www/enc/index.m3u8 >www/enc/float.m3u8
# fMP4 with an AES-128 encrypted init section (RFC 8216 §4.3.2.5: KEY with IV before MAP).
HAVE_OPENSSL=0
if command -v openssl >/dev/null; then
  HAVE_OPENSSL=1
  mkdir -p www/encinit
  "${FF[@]}" "${SRC[@]}" "${MP4ENC[@]}" -f hls -hls_time 2 -hls_playlist_type vod -hls_segment_type fmp4 \
    -hls_segment_filename 'www/encinit/s%03d.m4s' www/encinit/index.m3u8
  head -c 16 /dev/urandom >www/encinit/key.bin
  KEYHEX=$(od -An -tx1 www/encinit/key.bin | tr -d ' \n')
  IVHEX=000102030405060708090a0b0c0d0e0f
  for f in www/encinit/init.mp4 www/encinit/s*.m4s; do
    openssl enc -aes-128-cbc -K "$KEYHEX" -iv "$IVHEX" -in "$f" -out "$f.enc" && mv "$f.enc" "$f"
  done
  sed -i.bak "s|^#EXT-X-MAP|#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x$IVHEX\\
#EXT-X-MAP|" www/encinit/index.m3u8
fi

python3 "$ROOT/scripts/e2e/rangeserver.py" "$PORT" www &
pids+=($!)
python3 -m http.server "$PORT_NORANGE" --bind 127.0.0.1 --directory www >/dev/null 2>&1 &
pids+=($!)
wait_http "$BASE/enc/index.m3u8" || { echo "fixture server did not start"; exit 2; }
wait_http "http://127.0.0.1:$PORT_NORANGE/enc/index.m3u8" || { echo "second server did not start"; exit 2; }

# ---------------------------------------------------------------- info
check "info: DASH manifest is recognised" bash -c "\"$BIN\" info $BASE/dash-tl/manifest.mpd | grep -q '^DASH'"
check "info: HLS media playlist reports encryption" bash -c "\"$BIN\" info $BASE/enc/index.m3u8 | grep -q 'AES-128'"
check "info --json: two variants and one audio rendition" bash -c "\"$BIN\" info --json $BASE/dash-tl/manifest.mpd | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d[\"protocol\"]==\"dash\" and len(d[\"variants\"])==2 and len(d[\"audio\"])==1'"

# ---------------------------------------------------------------- VOD downloads
run get "$BASE/enc/index.m3u8" -o enc.mp4 -j 16
check "HLS AES-128 TS: duration ~20 s" near "$(duration enc.mp4)" 20 0.5
check "HLS AES-128 TS: decodes" decodes enc.mp4

run get "$BASE/fmp4/master.m3u8" -q 180 -o fmp4.mkv
check "HLS fMP4 master: picked 320x180 variant" [ "$(width fmp4.mkv)" = 320 ]
check "HLS fMP4 master: separate audio muxed in" has_audio fmp4.mkv
check "HLS fMP4 master: decodes" decodes fmp4.mkv

run get "$BASE/dash-tl/manifest.mpd" -q 180 -o dash-tl.mp4
check "DASH timeline: picked 320x180 representation" [ "$(width dash-tl.mp4)" = 320 ]
check "DASH timeline: duration ~20 s" near "$(duration dash-tl.mp4)" 20 0.5
check "DASH timeline: decodes" decodes dash-tl.mp4

run get "$BASE/dash-num/manifest.mpd" -o dash-num.mp4
check "DASH number template: duration ~20 s" near "$(duration dash-num.mp4)" 20 0.5
check "DASH number template: decodes" decodes dash-num.mp4

run get "$BASE/dash-overhang/manifest.mpd" -o dash-overhang.mp4
check "DASH number template, audio longer than video: no 404 on the implied last segment" [ -s dash-overhang.mp4 ]
check "DASH number template, audio longer than video: duration ~10.3 s" near "$(duration dash-overhang.mp4)" 10.3 0.5
check "DASH number template, audio longer than video: decodes" decodes dash-overhang.mp4

run get "$BASE/dash-single/manifest.mpd" -o dash-single.mp4
check "DASH single-file (sidx): duration ~20 s" near "$(duration dash-single.mp4)" 20 0.5
check "DASH single-file (sidx): decodes" decodes dash-single.mp4

run get "http://127.0.0.1:$PORT_NORANGE/dash-single/manifest.mpd" -o dash-single-norange.mp4
packets dash-single.mp4 >a.packets
packets dash-single-norange.mp4 >b.packets
check "Range-ignoring server: output identical to Range-honouring server" bash -c '[ -s a.packets ] && cmp -s a.packets b.packets'

run get "$BASE/fmp4/audio-only.m3u8" -q worst -o audio-only.mkv
check "HLS master with an audio-only variant: -q worst picks the video" [ "$(width audio-only.mkv)" = 320 ]

run get "$BASE/enc/float.m3u8" -o float.mp4
check "HLS fractional TARGETDURATION: duration ~20 s" near "$(duration float.mp4)" 20 0.5
check "HLS fractional TARGETDURATION: decodes" decodes float.mp4

if [ "$HAVE_OPENSSL" = 1 ]; then
  run get "$BASE/encinit/index.m3u8" -o encinit.mp4
  check "HLS fMP4 with encrypted init section: duration ~20 s" near "$(duration encinit.mp4)" 20 0.5
  check "HLS fMP4 with encrypted init section: decodes" decodes encinit.mp4
else
  echo "SKIP  encrypted init section (openssl not found)"
fi

run get "$BASE/dash-mp/manifest.mpd" -o dash-mp.mp4
check "DASH two periods, timestamps restarting: duration ~12 s" near "$(duration dash-mp.mp4)" 12 0.5
check "DASH two periods: decodes" decodes dash-mp.mp4

run get "$BASE/redir/fmp4/master.m3u8" -q 180 -o redir-hls.mkv
check "HLS behind a 302: relative URIs resolve against the redirect target" [ "$(width redir-hls.mkv)" = 320 ]
run get "$BASE/redir/dash-num/manifest.mpd" -o redir-dash.mp4
check "DASH behind a 302: relative URIs resolve against the redirect target" near "$(duration redir-dash.mp4)" 20 0.5

# ---------------------------------------------------------------- safety
run get "$BASE/enc/index.m3u8" -o tty.mp4
check "status lines reach a stderr that is not a terminal" grep -q "» HLS stream" "$WORK/last.log"

cp dash-num.mp4 exists.mp4
check "existing output: refused without --force" bash -c "! \"$BIN\" get $BASE/enc/index.m3u8 -o exists.mp4 2>/dev/null"
check "existing output: left untouched" cmp -s exists.mp4 dash-num.mp4
run get "$BASE/enc/index.m3u8" -o exists.mp4 --force
check "--force: overwrites the existing output" bash -c "cmp -s <(packets exists.mp4) <(packets enc.mp4)"

run get "$BASE/enc/index.m3u8" -o stale.mp4 --keep-parts
rm -f stale.mp4
"$BIN" get "$BASE/enc/float.m3u8" -o stale.mp4 >/dev/null 2>"$WORK/last.log"
check "work dir of another stream: refused, not spliced in" grep -q "another download" "$WORK/last.log"

# ---------------------------------------------------------------- resume
run get "$BASE/enc/index.m3u8" -o res.mp4 --keep-parts
rm -f res.mp4
find res.mp4.parts/main -name '*.seg' | sort | head -3 | xargs rm -f
run get "$BASE/enc/index.m3u8" -o res.mp4
check "resume: 7 of 10 segments reused" grep -q "7 resumed" "$WORK/last.log"
packets res.mp4 >a.packets
packets enc.mp4 >b.packets
check "resume: identical to uninterrupted download" bash -c '[ -s a.packets ] && cmp -s a.packets b.packets'

# ---------------------------------------------------------------- live
if [ "${E2E_SKIP_LIVE:-0}" != 1 ]; then
  LIVE=(-re -f lavfi -i "testsrc2=size=320x180:rate=30" -f lavfi -i "sine=frequency=660" -t 45)
  mkdir -p www/live www/dlive
  "${FF[@]}" "${LIVE[@]}" "${TSENC[@]}" -f hls -hls_time 1 -hls_list_size 4 -hls_flags delete_segments \
    -hls_segment_filename 'www/live/s%05d.ts' www/live/index.m3u8 &
  pids+=($!)
  "${FF[@]}" "${LIVE[@]}" "${MP4ENC[@]}" -f dash -seg_duration 1 -streaming 1 -window_size 5 -remove_at_exit 1 \
    -use_template 1 -use_timeline 0 www/dlive/manifest.mpd &
  pids+=($!)
  sleep 5

  run get "$BASE/live/index.m3u8" -o live-limit.mp4 --max-duration 6
  check "live HLS: --max-duration 6 gives ~6 s" near "$(duration live-limit.mp4)" 6 0.6
  check "live HLS: decodes" decodes live-limit.mp4

  "$BIN" get "$BASE/live/index.m3u8" -o live-int.mp4 >"$WORK/last.log" 2>&1 &
  cpid=$!
  sleep 6
  kill -INT "$cpid"
  wait "$cpid"
  check "live HLS: Ctrl-C exits 0" [ $? -eq 0 ]
  check "live HLS: Ctrl-C keeps the capture (>= 6 s)" awk -v d="$(duration live-int.mp4)" 'BEGIN { exit !(d >= 6) }'
  check "live HLS: interrupted capture decodes" decodes live-int.mp4

  run get "$BASE/dlive/manifest.mpd" -o dlive-limit.mp4 --max-duration 6
  check "live DASH (number template): --max-duration 6 gives ~6 s" near "$(duration dlive-limit.mp4)" 6 0.6
  check "live DASH: decodes" decodes dlive-limit.mp4

  # The origin deletes the playlist when the stream ends, without ever writing ENDLIST.
  mkdir -p www/gone
  ( "${FF[@]}" -re -f lavfi -i "testsrc2=size=320x180:rate=30" -f lavfi -i "sine=frequency=550" -t 8 \
      "${TSENC[@]}" -f hls -hls_time 1 -hls_list_size 4 -hls_flags delete_segments+omit_endlist \
      -hls_segment_filename 'www/gone/s%05d.ts' www/gone/index.m3u8; rm -f www/gone/index.m3u8 ) &
  pids+=($!)
  wait_http "$BASE/gone/index.m3u8"
  timeout 60 "$BIN" get "$BASE/gone/index.m3u8" -o gone.mp4 >/dev/null 2>"$WORK/last.log"
  check "live HLS, playlist deleted at the end: exits 0" [ $? -eq 0 ]
  check "live HLS, playlist deleted at the end: capture saved (>= 5 s)" awk -v d="$(duration gone.mp4)" 'BEGIN { exit !(d >= 5) }'
  check "live HLS, playlist deleted at the end: decodes" decodes gone.mp4

  # Watch mode: the stream only appears 5 s after collider starts polling.
  mkdir -p www/late
  ( sleep 5; "${FF[@]}" -re -f lavfi -i "testsrc2=size=320x180:rate=30" -f lavfi -i "sine=frequency=700" -t 20 \
      "${TSENC[@]}" -f hls -hls_time 1 -hls_list_size 4 -hls_flags delete_segments \
      -hls_segment_filename 'www/late/s%05d.ts' www/late/index.m3u8 ) &
  pids+=($!)
  run get "$BASE/late/index.m3u8" -o late.mp4 --wait 2 --max-duration 4
  check "watch mode: records a stream that starts late" near "$(duration late.mp4)" 4 0.6
fi

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
