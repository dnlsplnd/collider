# collider GUI: "Deep Prussian" design spec

Screens: `screen-1.html` Downloads, `screen-2.html` New download, `screen-3.html` Live job details, `screen-4.html` Settings
(PNG renders next to them, `screen-1-min.png` plus `min-2/3/4.png` at 1100 x 700).
Source generator: `src/common.py` (tokens, CSS, icons) and `src/s1..s4.py`. Contrast scripts: `contrast.py`, `contrast_report.py`.

## 1. Direction

Prussian Blue is the surface itself. The window is not dark grey with a blue accent: every surface, from the deepest well to
a hovered row, sits on one hue (about 204 to 208 degrees, a green-leaning blue that is not navy). The only change between
surfaces is lightness. `#003153` is the card and panel colour, so most of the screen area at first glance is literally
Prussian Blue. Text is a cool white with a slight blue tint. One warm complement, **brass** `#D9A441`, is kept for two
things: the primary action on a screen, and the live-recording state (LIVE badge, REC timecode, in-flight segments,
the live progress strip). Semantic colours (green, orange, coral, cyan) are used sparingly and only as text or as a
tinted background, never as large fills. The feel is calm and precise, like a mastering console. Technical values are
set in monospace, and nothing moves except hover and press states and one spinner.

## 2. Palette: Prussian Dark (default)

| Token | Hex | Usage |
|---|---|---|
| `p-950` | `#001726` | Deepest: status bar, code/log wells |
| `p-900` | `#001F33` | Sidebar, URL field fill, tab-group track |
| `p-850` | `#00263F` | **Window / content background**, inputs (sunken), segment-map well |
| `p-800` | `#003153` | **Brand. Cards, panels, job rows**, disabled-control fill |
| `p-750` | `#073A60` | Raised: secondary buttons, table header, nav-active, tile fill, start bar |
| `p-700` | `#0E4570` | Hover fill, progress track, active tab |
| `p-650` | `#175181` | Pressed / selected row, selected option, protocol badge, tooltip |
| `p-600` | `#1F5E93` | Selected segmented-control item |
| `p-500` | `#2E6FA6` | Reused-segment part of a progress bar, badge outline, logo stroke |
| `p-400` | `#5A90BF` | **Component boundary** (inputs, secondary buttons, toggles, checkboxes), DVR cells |
| `p-300` | `#7FB0DA` | Progress fill, slider fill, checked checkbox, radio dot, sparkline, downloaded cells |
| `p-200` | `#A6C6E2` | Icon tint in tiles, selected-seg ring, in-flight tip on the live strip, links |
| `p-100` | `#D6E6F3` | Light-theme tracks |
| `line` | `#0F4169` | Decorative dividers and card borders (no contrast requirement) |
| `line-2` | `#164D78` | Stronger decorative border (tiles, kbd, table header rule) |
| `tx-1` | `#EEF5FA` | Primary text, values |
| `tx-2` | `#B4CADC` | Secondary text, idle icons |
| `tx-3` | `#97B3CB` | Tertiary text: meta, labels, axis. Never on `p-650` or lighter |
| `tx-dis` | `#6F8FAA` | Disabled text |
| `ac` | `#D9A441` | **Brass accent**: primary button, LIVE badge, REC timecode, in-flight, toggle-on |
| `ac-hi` / `ac-lo` | `#E6B656` / `#C4902F` | Primary hover / pressed (and primary 1 px border) |
| `on-ac` | `#00182A` | Text and icons on brass |
| `ac-soft` / `ac-line` | `#234350` / `#62654B` | Brass badge background / live tile border |
| `ok` / `ok-soft` | `#4CC38A` / `#0C485C` | Completed, 0 gaps, ffmpeg detected |
| `warn` / `warn-soft` | `#F0955A` / `#264154` | Retries, slow playlist reloads |
| `bad` / `bad-soft` | `#F4887D` / `#273E57` | Not supported (DRM), missing segments, Cancel |
| `info` / `info-soft` | `#6CC6E6` / `#11496B` | Watch mode, resumed, spinner |
| `focus` | `#8CC4F2` | Keyboard focus ring (2 px, with a 2 px `p-850` gap) |

The soft backgrounds are solid pre-mixed colours (16 % of the semantic colour over `#003153`), not alpha, so egui can paint them directly.

**Prussian Light** (the second theme, shown as a preview in Settings): bg `#EEF3F7`, cards `#FFFFFF`, card border `#C9D8E5`,
**sidebar stays `#003153`** (the brand anchor), text `#00253F`, secondary text `#3D5D78`, input border `#6F8FAA`,
primary button `#003153` with white text, brass used as text `#8A5F0E`, live badge is still `#D9A441` with `#00182A` text.

## 3. Typography (Inter + JetBrains Mono)

egui has no variable font weights, so load **Inter Regular, Medium and SemiBold** as three font data entries and register
`FontFamily::Name("inter-medium")` and `FontFamily::Name("inter-semibold")`. Monospace is **JetBrains Mono Regular**, with
tabular figures by nature. (The mockups fall back to Adwaita Sans, which is derived from Inter, and to Noto Sans Mono, because neither Inter nor JetBrains Mono is installed on the render machine.)

| egui TextStyle | Size / line | Family / weight | Used for |
|---|---|---|---|
| `Heading` | 22 / 26 | Inter SemiBold | Page titles (Downloads, Settings) |
| `Name("Title")` | 18 / 24 | Inter SemiBold | Detail header (city-council-live), stream name |
| `Name("CardTitle")` | 14.5 / 20 | Inter SemiBold | Card headers, job names |
| `Body` | 13.5 / 19.5 | Inter Regular | Default text, form values |
| `Name("BodyStrong")` | 13 / 18 | Inter SemiBold | Setting labels, state words ("Recording") |
| `Button` | 13 / 16 (14 for large) | Inter SemiBold | All buttons, tabs use Medium |
| `Small` | 12 / 16 | Inter Regular | Help text, meta lines, legends |
| `Name("Label")` | 11 / 14, uppercase, `extra_letter_spacing` 0.9 | Inter SemiBold | Section labels, table headers |
| `Name("Badge")` | 10.5 / 12, uppercase, spacing 0.6 | Inter SemiBold (Bold if bundled) | Badges |
| `Monospace` | 12.5 / 18 | JetBrains Mono | Bitrates, codecs, resolutions, paths, timecodes, counts |
| `Name("MonoLarge")` | 19 / 24 | JetBrains Mono | KPI values (01:12:44, 6.2 Mbit/s) |
| `Name("MonoSmall")` | 11.5 / 14 | JetBrains Mono | Log timestamps, axis labels, row labels |

Numbers use a thin space as the thousands separator in the UI font ("4 362") and a plain space in mono.

## 4. Spacing, radii, elevation

- **Spacing scale (px):** 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24. The base unit is 4, with 2 and 6 for tight inner gaps. Page gutter 24, card padding 18 x 14 to 16, gap between cards 12 to 14, gap inside a job row 20 (16 when compact), gap between list rows 8.
- **Heights:** top bar 64, status bar 30, sidebar item 38, large button 40 (start button 44), button 32, small button 28, input 34 (URL field 40), table row 32 to 36, job row about 84.
- **Radii:** 5 badges, 6 segmented items / tabs / tooltips, 8 buttons and inputs, 10 tiles and theme cards, 12 cards and job rows. Toggles, progress bars and dots are fully round.
- **Elevation:** cards and rows use one shadow, `offset (0,6) blur 16 color rgba(0,10,20,0.28)` plus a 1 px `rgba(0,10,20,0.35)` bottom edge. The tooltip uses `(0,4) blur 12 alpha 0.45`. There is no blur of content and no glass.

## 5. Components (all states)

**Buttons** (radius 8, Button text style, icon 14 to 18 px, gap 8)
- *Primary*: fill `ac`, 1 px `ac-lo` border, label `on-ac`. Hover `ac-hi`, pressed `ac-lo`. Disabled fill `p-800`, border `line-2`, label `tx-dis`. Focus adds a 2 px `focus` ring outside a 2 px gap. At most one primary per view region (the top bar has Add, the content has Start download or Stop and save).
- *Secondary*: fill `p-750`, 1 px `p-400`, label `tx-1`. Hover `p-700`, pressed `p-650`. Disabled as primary. Focus ring.
- *Ghost*: transparent, label `tx-2`. Hover fill `p-700` with `tx-1`, pressed `p-650`. Disabled `tx-dis`.
- *Danger*: transparent, 1 px `#B06C72`, label `bad`. Hover fill `bad-soft`, pressed `#33405A`. It is only used for destructive actions such as Cancel recording. "Not supported" is never shown as danger-red chrome.
- *Icon button*: 32 x 32 (28 in tables), `tx-2` icon. Hover `p-700` fill with a `tx-1` icon.

**Text input**: fill `p-850` (sunken), 1 px `p-400`, radius 8, height 34, padding 10, value `tx-1` (mono for paths and numbers), unit suffix `tx-3`, placeholder `tx-3`. Hover border `p-300`. Focus border `focus` at 2 px (drawn as 1 px plus a 1 px outer stroke). Disabled fill `p-800`, border `line-2`, text `tx-dis`. Error border `bad` with a Small `bad` message below.
**URL field**: the same, but 40 high on `p-900`, with a leading Link icon, a trailing kbd hint (Ctrl L), and a trailing `b-ok` "Probed · 412 ms" badge after a probe.
**Dropdown (ComboBox)**: an input with a CaretDown icon (`tx-2`) at the right. The popup is `p-750` with 1 px `p-400`, radius 8, and the tooltip shadow. Items are 30 high, hover `p-700`, selected `p-650` with a Check icon.
**Segmented control**: track `p-850` with 1 px `p-400`, padding 2. Items are 28 high with radius 6 and label `tx-2`. Selected item `p-600` with a 1 px inner `p-200` ring and `tx-1`. Hover item `p-700`.
**Toggle**: 36 x 20. Off: track `p-850`, 1 px `p-400`, knob `tx-2`. On: track `ac`, border `ac-lo`, knob `on-ac`. Disabled: `p-800` / `line-2` / `tx-dis` knob. Focus ring.
**Slider**: track 4 px `p-650`, trailing fill `p-300`, knob 16 px `tx-1` with a 2 px `p-300` ring. Hover knob ring `p-200`. Dragging knob 18 px. It is paired with a numeric input (Parallel segments 1 to 32) and mono tick labels.
**Checkbox / radio**: 16 px, 1 px `p-400` on `p-850`. Checked: fill `p-300` with a `p-950` check, or a radio dot `p-300` with a 1.5 px ring. Disabled: `line-2` on `p-800`.
**Badges** (20 high, radius 5, Badge style): protocol `b-proto` (`p-650` fill, `p-500` border, `tx-1`), outline (VOD), LIVE `b-live` (brass fill, `on-ac` text, 6 px dot), and soft semantic badges `b-ok`, `b-warn`, `b-bad`, `b-info`, `b-ac`. Soon uses `p-750` with `tx-2`.
**Progress bar**: 6 px, track `p-700`, fill `p-300` (downloading). A two-part fill shows resumed jobs (reused segments `p-500`, then new segments `p-300`). Completed `ok`, watch countdown `info`. The live strip is a brass segment strip (`ac` cells over `#A07A30` separators) with a `p-200` in-flight tip, painted with `rect_filled` runs.
**Table rows**: header 32 high, `p-750`, Label style `tx-3`, rule `line-2`. Rows 32 to 38 high with a 1 px `line` rule. Hover `p-700`. Selected `p-650` with a 3 px brass bar inside the left edge and a `p-300` radio dot. Numbers are right-aligned mono. Disabled rows use `tx-dis`.
**Job row (card)**: `p-800`, 1 px `line`, radius 12, shadow, padding 14 x 16. It is a grid of five columns: 40 px state tile, name + meta (badges + host), progress block (state line + value, bar, stat line), metrics (mono throughput + small), actions. Hover `#04375C` with border `line-2`. Selected border `p-400`. Tile colour follows state: live (brass soft), downloading (neutral `p-750`), watching (info soft), completed (ok soft), unsupported (`p-850` with `tx-2`). A failed or unsupported row is calm: a neutral shield tile, a coral "Not supported" badge, and two lines of plain explanation. It is never a red card.
**Sidebar item**: 38 high, radius 8, `tx-2` label with an 18 px icon. Hover `p-800`. Active `p-750` with a 1 px `line-2` ring, a `tx-1` label, a brass icon, and a 3 px brass bar at the sidebar edge. The count pill is mono 11.5 on `p-800` (`p-650` when active).
**Tabs (filter)**: a pill group on a `p-900` track with 1 px `line`. The tab is 30 high with a Medium label in `tx-2` and a mono count in `tx-3`. Hover `p-800`. Active `p-700` with a 1 px inner `p-400` ring and `tx-1`.
**Status bar**: 30 high on `p-950` with a top `line` border. Items are Small `tx-2` with mono values in `tx-1`, separated by 1 px `line` verticals. Coloured 7 px dots use the state colours. On the right, ffmpeg status shows a CheckCircle in `ok` and the version.
**Segment map**: a well of `p-850`. There is one row per 12 minutes (360 positions x 2 s), each row 10 px high with a 3 px gap, with mono start times on the left. The colours are DVR window `p-400`, downloaded `p-300`, in flight `ac`, pending (1 px `p-400` outline), retried `warn`, and missing `bad`. Runs of equal state are painted as one rect, with 1 px `p-800` minute separators. On hover the cell gets a 1.5 px `tx-1` outline and a one-line tooltip on `p-650`.
**Sparkline**: 5 minutes at 2 s resolution. It uses a 1.5 px `p-300` line over a solid `#0B4068` area fill, gridlines at 0/4/8 Mbit/s in `line`, mono axis labels in `tx-3`, event markers as dashed 1 px `warn` verticals with a label, and a current-value dot in brass (live).
**KPI strip**: six equal cells split by 1 px `line`. Each has a label (Small `tx-3`), a value (MonoLarge, where the recording time is brass and 0 gaps is `ok`), and a sub line (Small `tx-3`).
**Event log**: rows 25 high with a mono timestamp (`tx-3`), a 15 px level icon (Info `tx-3`, Warning `warn`, CheckCircle `ok`), and the message in `tx-2` with key numbers in `tx-1` Medium. Hover `p-750`.
**Spinner**: `egui::Spinner` 16 px, colour `info` on a `p-600` ring (watch mode only).

## 6. Phosphor icons used (egui-phosphor, regular weight; the fill variant where noted)

DownloadSimple, Binoculars, ClockCounterClockwise, GearSix, Link, MagnifyingGlass, Plus, Stop (fill), Pause (fill),
X, DotsThree, FolderOpen, CheckCircle, Check, ShieldWarning, ArrowClockwise (resumed job, reset), ArrowsClockwise (Check now, Detect),
Record (live tile), Broadcast (live options), CaretDown, ArrowLeft, FilmStrip, SpeakerHigh, Subtitles, Lock (AES-128),
Info, Warning, Trash, Copy, Globe (network), SlidersHorizontal, Palette, TerminalWindow, SortAscending.

## 7. Contrast (computed with `contrast.py`, WCAG 2.x relative luminance)

| Foreground | on Background | Ratio | Result | Where |
|---|---|---|---|---|
| `tx-1` #EEF5FA | `p-950` #001726 | 16.58:1 | AA text |  |
| `tx-1` #EEF5FA | `p-900` #001F33 | 15.34:1 | AA text |  |
| `tx-1` #EEF5FA | `p-850` #00263F | 14.14:1 | AA text | body text on window |
| `tx-1` #EEF5FA | `p-800` #003153 | 12.20:1 | AA text | body text on cards |
| `tx-1` #EEF5FA | `p-750` #073A60 | 10.70:1 | AA text |  |
| `tx-1` #EEF5FA | `p-700` #0E4570 | 9.07:1 | AA text |  |
| `tx-1` #EEF5FA | `p-650` #175181 | 7.54:1 | AA text | selected row / protocol badge |
| `tx-1` #EEF5FA | `p-600` #1F5E93 | 6.19:1 | AA text | selected segment item |
| `tx-1` #EEF5FA | `job-hover` #04375C | 11.18:1 | AA text |  |
| `tx-2` #B4CADC | `p-900` #001F33 | 9.99:1 | AA text |  |
| `tx-2` #B4CADC | `p-850` #00263F | 9.21:1 | AA text |  |
| `tx-2` #B4CADC | `p-800` #003153 | 7.94:1 | AA text | secondary text on cards |
| `tx-2` #B4CADC | `p-750` #073A60 | 6.97:1 | AA text |  |
| `tx-2` #B4CADC | `p-700` #0E4570 | 5.90:1 | AA text |  |
| `tx-2` #B4CADC | `p-650` #175181 | 4.91:1 | AA text | secondary text in selected row |
| `tx-3` #97B3CB | `p-950` #001726 | 8.37:1 | AA text |  |
| `tx-3` #97B3CB | `p-900` #001F33 | 7.74:1 | AA text |  |
| `tx-3` #97B3CB | `p-850` #00263F | 7.14:1 | AA text |  |
| `tx-3` #97B3CB | `p-800` #003153 | 6.15:1 | AA text | tertiary/meta text on cards |
| `tx-3` #97B3CB | `p-750` #073A60 | 5.40:1 | AA text |  |
| `tx-3` #97B3CB | `p-700` #0E4570 | 4.58:1 | AA text | tertiary text on hover (lowest allowed) |
| `on-ac` #00182A | `ac` #D9A441 | 8.02:1 | AA text | primary button label |
| `on-ac` #00182A | `ac-hi` #E6B656 | 9.61:1 | AA text |  |
| `on-ac` #00182A | `ac-lo` #C4902F | 6.34:1 | AA text |  |
| `ac` #D9A441 | `p-800` #003153 | 5.97:1 | AA text | recording timecode |
| `ac` #D9A441 | `p-850` #00263F | 6.92:1 | AA text |  |
| `ac` #D9A441 | `ac-soft` #234350 | 4.69:1 | AA text | brass badge (Selected, Following) |
| `ok` #4CC38A | `p-800` #003153 | 6.06:1 | AA text |  |
| `ok` #4CC38A | `ok-soft` #0C485C | 4.52:1 | AA text | success badge |
| `warn` #F0955A | `p-800` #003153 | 5.85:1 | AA text |  |
| `warn` #F0955A | `warn-soft` #264154 | 4.65:1 | AA text | warning badge |
| `bad` #F4887D | `p-800` #003153 | 5.54:1 | AA text |  |
| `bad` #F4887D | `bad-soft` #273E57 | 4.53:1 | AA text | danger badge (Not supported) |
| `info` #6CC6E6 | `p-800` #003153 | 6.94:1 | AA text |  |
| `info` #6CC6E6 | `info-soft` #11496B | 4.96:1 | AA text | info badge (Watching, Resumed) |
| `L-tx1` #00253F | `L-card` #FFFFFF | 15.71:1 | AA text | Prussian Light body |
| `L-tx1` #00253F | `L-bg` #EEF3F7 | 14.07:1 | AA text |  |
| `L-tx2` #3D5D78 | `L-card` #FFFFFF | 6.92:1 | AA text |  |
| `L-tx2` #3D5D78 | `L-bg` #EEF3F7 | 6.19:1 | AA text | Prussian Light secondary |
| `white` #FFFFFF | `L-primary` #003153 | 13.43:1 | AA text | Prussian Light primary button |
| `L-ac-text` #8A5F0E | `L-card` #FFFFFF | 5.64:1 | AA text | Prussian Light brass text |
| `L-side-tx` #B4CADC | `L-side` #003153 | 7.94:1 | AA text |  |

| Component colour | vs Adjacent | Ratio | ≥3:1 | Component |
|---|---|---|---|---|
| `p-400` #5A90BF | `p-800` #003153 | 3.95:1 | yes | input / secondary button border on card |
| `p-400` #5A90BF | `p-850` #00263F | 4.58:1 | yes | input border on window bg |
| `p-400` #5A90BF | `p-900` #001F33 | 4.97:1 | yes | URL field border on top bar area |
| `p-400` #5A90BF | `p-750` #073A60 | 3.46:1 | yes | border vs secondary-button fill |
| `focus` #8CC4F2 | `p-800` #003153 | 7.22:1 | yes | focus ring on card |
| `focus` #8CC4F2 | `p-850` #00263F | 8.37:1 | yes | focus ring on window bg |
| `ac` #D9A441 | `p-800` #003153 | 5.97:1 | yes | primary button fill vs card |
| `ac` #D9A441 | `p-850` #00263F | 6.92:1 | yes | primary button fill vs window |
| `p-300` #7FB0DA | `p-700` #0E4570 | 4.34:1 | yes | progress fill vs track |
| `ac` #D9A441 | `p-700` #0E4570 | 4.44:1 | yes | live progress vs track |
| `ok` #4CC38A | `p-700` #0E4570 | 4.51:1 | yes | completed progress vs track |
| `info` #6CC6E6 | `p-700` #0E4570 | 5.16:1 | yes | watch countdown vs track |
| `p-300` #7FB0DA | `p-800` #003153 | 5.83:1 | yes | slider fill / checked box / radio dot vs card |
| `tx-1` #EEF5FA | `p-800` #003153 | 12.20:1 | yes | slider knob vs card |
| `p-300` #7FB0DA | `p-850` #00263F | 6.76:1 | yes | segment map: downloaded vs well |
| `ac` #D9A441 | `p-850` #00263F | 6.92:1 | yes | segment map: in flight vs well |
| `bad` #F4887D | `p-850` #00263F | 6.42:1 | yes | segment map: missing vs well |
| `warn` #F0955A | `p-850` #00263F | 6.78:1 | yes | segment map: retried vs well |
| `p-400` #5A90BF | `p-850` #00263F | 4.58:1 | yes | segment map: pending outline vs well |
| `p-300` #7FB0DA | `p-800` #003153 | 5.83:1 | yes | sparkline line vs card |
| `L-border` #6F8FAA | `L-card` #FFFFFF | 3.40:1 | yes | light: input border |
| `L-primary` #003153 | `L-bg` #EEF3F7 | 12.02:1 | yes | light: primary button vs bg |
| #A6C6E2 | #1F5E93 | 3.83:1 | yes | seg-on ring p-200 vs its p-600 fill |
| #B06C72 | #003153 | 3.35:1 | yes | danger button border vs card |
| #5A90BF | #00263F | 4.58:1 | yes | segment map DVR cell (p-400) vs well |
| #7FB0DA | #5A90BF | 1.48:1 | no | segment map downloaded vs DVR (adjacent) |
| `tx-dis` #6F8FAA | `p-800` #003153 | 3.95:1 | n/a | disabled text (WCAG exempt, still 3.95) |

Notes. All body and secondary text passes AA (4.5:1) on every surface it is used on. `tx-3` must not be used on `p-650` or
lighter, where it measures 3.80:1. Every UI boundary that identifies a component passes 3:1. The one pair below 3:1 is
DVR versus downloaded segments in the map (1.48:1). Both mean "downloaded", so that distinction is supplementary and is
repeated in the legend, the row time labels and the tooltip. Card borders (`line`) are decorative. The card is
distinguished from the window by fill (`p-800` vs `p-850`) and the component inside it carries its own 3:1 border.

## 8. egui mapping

```rust
let mut v = egui::Visuals::dark();
v.panel_fill            = P850;            // CentralPanel / window background
v.window_fill           = P800;            // cards drawn with Frame::group use P800 explicitly
v.extreme_bg_color      = P850;            // TextEdit background (sunken)
v.faint_bg_color        = P750;            // striped / table header
v.code_bg_color         = P950;
v.window_stroke         = Stroke::new(1.0, LINE);
v.window_rounding       = Rounding::same(12.0);   // (egui >= 0.29: window_corner_radius = 12)
v.menu_rounding         = Rounding::same(8.0);
v.window_shadow         = Shadow { offset: vec2(0.,6.), blur: 16., spread: 0., color: rgba(0,10,20,72) };
v.popup_shadow          = Shadow { offset: vec2(0.,4.), blur: 12., spread: 0., color: rgba(0,10,20,115) };
v.selection.bg_fill     = P600;            // text selection, selected SelectableLabel
v.selection.stroke      = Stroke::new(1.0, P200);
v.hyperlink_color       = P200;
v.warn_fg_color         = WARN;
v.error_fg_color        = BAD;
v.text_cursor.stroke    = Stroke::new(2.0, FOCUS);
v.slider_trailing_fill  = true;            // trailing fill uses selection.bg_fill; override to P300 in a custom slider if exact
v.handle_shape          = HandleShape::Circle;
v.striped               = false;
let w = &mut v.widgets;
w.noninteractive.bg_fill = P800; w.noninteractive.weak_bg_fill = P800;
w.noninteractive.bg_stroke = Stroke::new(1.0, LINE); w.noninteractive.fg_stroke = Stroke::new(1.0, TX2);
w.inactive.bg_fill = P750; w.inactive.weak_bg_fill = P750;
w.inactive.bg_stroke = Stroke::new(1.0, P400); w.inactive.fg_stroke = Stroke::new(1.0, TX1);
w.hovered.bg_fill  = P700; w.hovered.weak_bg_fill = P700;
w.hovered.bg_stroke = Stroke::new(1.0, P300); w.hovered.fg_stroke = Stroke::new(1.5, TX1); w.hovered.expansion = 0.0;
w.active.bg_fill   = P650; w.active.weak_bg_fill = P650;
w.active.bg_stroke = Stroke::new(1.0, P200); w.active.fg_stroke = Stroke::new(1.5, TX1); w.active.expansion = 0.0;
w.open.bg_fill     = P700; w.open.bg_stroke = Stroke::new(1.0, P400);
for s in [&mut w.noninteractive,&mut w.inactive,&mut w.hovered,&mut w.active,&mut w.open] { s.rounding = Rounding::same(8.0); }

let mut st = (*ctx.style()).clone();
st.visuals = v;
st.spacing.item_spacing   = vec2(8.0, 8.0);
st.spacing.button_padding = vec2(13.0, 7.0);   // 32 px buttons with 13 px text
st.spacing.interact_size  = vec2(32.0, 32.0);
st.spacing.slider_width   = 240.0;
st.spacing.icon_width     = 16.0; st.spacing.icon_spacing = 8.0;
st.spacing.menu_margin    = Margin::same(6.0);
st.spacing.window_margin  = Margin::symmetric(18.0, 16.0);
st.spacing.scroll         = ScrollStyle { bar_width: 8.0, floating: true, ..ScrollStyle::floating() };
st.text_styles = [ (Heading, 22 semibold), (Body, 13.5), (Button, 13 semibold), (Small, 12), (Monospace, 12.5), (Name("Title"),18), ... ].into();
ctx.set_style(st);
```

- **Panels:** `SidePanel::left` 232 px `Frame::none().fill(P900).stroke(LINE)`. `TopBottomPanel::top` 64 px on `P850` with a bottom `LINE`. `TopBottomPanel::bottom` 30 px on `P950`. `CentralPanel` holds a `ScrollArea::vertical` with 24 px margins.
- **Cards and job rows:** `Frame::none().fill(P800).stroke(Stroke::new(1.,LINE)).rounding(12.).shadow(card_shadow).inner_margin(Margin::symmetric(16.,14.))`. Hover is `ui.interact(rect, id, Sense::click())`, then repaint the fill `#04375C`.
- **Primary button:** `Button::new(RichText::new(label).color(ON_AC).strong()).fill(AC).stroke(Stroke::new(1.,AC_LO)).rounding(8.)`, with fill swapped to `AC_HI` or `AC_LO` from `response.hovered()` / `is_pointer_button_down_on()`. Danger is `.fill(TRANSPARENT).stroke(1, #B06C72)` with `BAD` text.
- **Focus:** egui's own keyboard focus is subtle. When `response.has_focus()`, paint `rect.expand(2.)` with `Stroke::new(2., FOCUS)` and radius +2.
- **Badges, toggle, progress, segment map, sparkline, KPI strip:** custom widgets drawn with `Painter::rect_filled`, `rect_stroke`, `circle_filled`, `line_segment`, `Shape::line` and `Shape::convex_polygon` (or `Mesh` for the sparkline area). Collapse equal-state runs in the segment map to one rect each, so a 2 000-segment map costs fewer than 100 shapes per frame. The live strip and the map repaint only when the job state changes (`ctx.request_repaint_after(500 ms)` while recording).
- **Icons:** `egui-phosphor` (regular + fill) merged into the Inter family, sized via RichText at 14 to 20 px, coloured with the token of their context.

## 9. Layout and collapse (1440 x 900 down to 1100 x 700 minimum)

- **Default (at least 1300 wide):** sidebar 232 px with wordmark, labels, counts and the output-volume card. Content gutter 24.
- **Below 1300 wide:** the sidebar collapses to a **64 px icon rail**. The wordmark gives way to the mark, labels and the volume card are hidden, and counts become corner badges on the icons. The active item keeps its brass bar. The status bar is unchanged.
  - *Downloads:* job-row columns shrink to 40 / 190+ / 230+ / 92 / auto. The metric column keeps only the throughput value, and the third item of each stat line (the codec summary) is dropped. The footer tip hides. The list scrolls beneath the fixed top bar and status bar. At 700 high about 4.5 rows are visible.
  - *New download:* the output column narrows to 320. The summary KPIs (duration, segments, tracks) hide, since the URL badge still shows the probe. The codecs column is dropped from the variants table. Audio and Subtitles stack. Start download stays at the bottom of the output card, and the content scrolls.
  - *Job details:* the KPI strip becomes a 3 x 2 grid. The segment-map hint text hides and the legend stays. The map rows reflow to the new width (same 12 min per row). The throughput and tracks columns become 1:1. The page scrolls.
  - *Settings:* the two-column card grid becomes one column (Defaults, Tools, Network, Appearance).
- **Height below 760 (Downloads):** page-header margins tighten (16 top, 8 below the header).
