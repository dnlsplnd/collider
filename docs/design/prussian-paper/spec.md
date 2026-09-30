# collider GUI: "Prussian Ink on Paper"

Design direction and implementation spec for the collider desktop app (egui / eframe).
Mockups: `screen-1.html` to `screen-4.html` (1440 × 900) and `screen-1-min.png` (1100 × 700).
Sources: `src/` holds the shared CSS and screen files. `build.py` inlines the fonts and Phosphor-style icons, and `render.sh` renders the PNGs. `contrast.py` computes every ratio in section 9.

## 1. Direction

Treat collider like a well-set technical document. The canvas is warm off-white paper and Prussian Blue #003153 is the ink. Prussian Blue is used for every element that carries meaning or authority: the full-height sidebar, headings, primary buttons, selected rows, focus rings, progress fills and the segment map. Everything else stays quiet: warm neutrals, hairline rules, generous whitespace and a clear type hierarchy, so the few coloured elements stand out. The one complementary accent is copper (#B85A2B, the complement of Prussian Blue's 205° hue). It appears only in the logo's collision spark, the active-nav tick, in-flight segments and "Soon" badges. The tone is calm and exact. It states facts plainly, including refusals: a DRM-protected stream is shown as "Not downloaded", which is a normal outcome, not an alarm.

Typography rule used on every screen:
- **Inter (tabular figures)** for counts, sizes and rates, such as `4 362 segments`, `1.9 GiB of 3.0 GiB` and `48 MiB/s`.
- **JetBrains Mono** for machine strings: URLs, paths, codecs, IDs, header names and values, clock times and durations (`01:12:44`, `00:24`).

## 2. Palette (Prussian Light, the default theme)

| Token | Hex | Usage |
|---|---|---|
| `prussian-950` | `#001A2E` | Sidebar border, tooltip arrow, deepest shadow tint |
| `prussian-900` | `#00243F` | Primary button pressed, tooltip fill, primary button 1 px border |
| **`prussian-800`** | **`#003153`** | **Brand.** Sidebar fill (top of gradient), H1/H2/H3, primary button, focus ring, selected-row bar, toggle on, slider fill and knob, segmented "on", active tab underline |
| `sidebar-bottom` | `#002845` | Bottom stop of the sidebar's single two-stop vertical gradient |
| `prussian-750` | `#083D63` | Sidebar item hover |
| `prussian-700` | `#0B4774` | Sidebar item active, primary button hover, progress fill, downloaded segments, sparkline stroke, badge text (HLS/DASH, Default) |
| `prussian-600` | `#1A5A8A` | Links, card-header icons, "Show all" footers |
| `prussian-500` | `#3A74A0` | Info icons in the event log |
| `prussian-400` | `#6B96BB` | Secondary button border, reused (resumed) segments, DVR-window segments, pending-segment outline, average line |
| `prussian-300` | `#9DB8CE` | Sidebar muted text, captions and icons, disk bar |
| `prussian-200` | `#C4D5E4` | Scroll thumb, tooltip secondary text |
| `prussian-150` | `#DBE6F0` | Progress / slider track, focus halo (3 px), text-selection fill |
| `prussian-100` | `#E4EDF5` | Selected row fill, protocol badge fill, info tile, sparkline area |
| `prussian-50` | `#F0F5F9` | Row hover, selected job row, secondary button hover |
| `paper` | `#F7F5F0` | Window / central panel background |
| `surface` | `#FFFEFB` | Cards, inputs, top bar |
| `sunken` | `#F0EDE6` | Status bar, table headers, segmented track, chips, disabled fills |
| `sunken-2` | `#E9E5DC` | Idle watch-timer track |
| `line` | `#E3DED3` | Card borders, dividers (decorative hairlines) |
| `line-soft` | `#ECE8DF` | Row separators inside cards |
| `line-strong` | `#7F8A95` | Input, checkbox, radio and toggle-off borders (3:1 component boundary) |
| `ink-900` | `#1B2A38` | Body text |
| `ink-800` | `#23384A` | Technical values, ghost-button text |
| `ink-600` | `#56636F` | Secondary text, captions, table headers |
| `ink-500` | `#6B7682` | Placeholders, input icons |
| `ink-400` | `#8E979F` | Disabled text (exempt from AA) |
| **accent `copper`** | `#B85A2B` | Logo spark, active-nav tick, in-flight segments, rail count badges |
| `copper-text` / `copper-bg` | `#A8491F` / `#F8EBE2` | "Soon" badge |
| `ok` / `ok-text` / `ok-bg` | `#1F7A4A` / `#1C6E43` / `#E5F1E9` | Completed, ffmpeg detected, 0 gaps |
| `warn` / `warn-text` / `warn-bg` | `#B07400` / `#7F5200` / `#FBF0D8` | Watching / waiting, slow reload, retries |
| `bad` / `bad-text` / `bad-bg` | `#B3261E` / `#A8231B` / `#FBE9E6` | Failed, DRM refusal, Cancel (danger ghost) |
| `live` / `live-text` / `live-bg` | `#D0202E` / `#B3141F` / `#FDE7E8` | LIVE badge, recording dot, missing segments |

Shadows (egui `Shadow`): card `offset [0,1] blur 2 spread 0 color #00243F0F`; popup/tooltip `offset [0,4] blur 14 color #00243F1A`.

**Prussian Dark (the theme option in Settings):** bg `#0C1A26`, surface `#132636`, sidebar `#001A2E`, line `#1E3548`, input border `#5A7590` (3.23:1), text `#E4EDF5` (13.05:1), muted `#9DB8CE` (8.55:1), primary button fill `#1A5A8A` with white text (7.31:1), and selection fill `#003153`. Semantic colours use the same hues lifted one step.

## 3. Typography (Inter + JetBrains Mono)

egui needs one `FontFamily` per weight: `Name("Inter")` = Regular, `Name("Inter-Medium")`, `Name("Inter-SemiBold")`, and `Monospace` = JetBrains Mono Regular (Bold as `Name("Mono-Bold")`).

| egui TextStyle | Font | Size / weight | Use |
|---|---|---|---|
| `Name("Title")` | Inter-SemiBold | 26 px, −2 % tracking | Page H1 ("Downloads", "Settings"); 24 px on detail pages |
| `Heading` | Inter-SemiBold | 17 px | Card / section titles (H2) |
| `Name("Subheading")` | Inter-SemiBold | 14 px | Card headers (H3), job names at 15 px |
| `Body` | Inter | 14 px | Default text, table cells at 13.5 px |
| `Button` | Inter-Medium | 14 px (13 px `sm`, 15 px SemiBold `lg`) | Buttons, tabs, nav items |
| `Small` | Inter | 12–12.5 px | Help text, stats, status bar |
| `Name("Caption")` | Inter-SemiBold | 11 px, UPPERCASE, +0.07 em (`extra_letter_spacing` 0.8) | Table headers, KPI labels, nav captions |
| `Name("Kpi")` | Inter-SemiBold | 24 px | KPI values |
| `Monospace` | JetBrains Mono | 12.5 px | URLs, paths, codecs, IDs, header values, timestamps |
| `Name("MonoSmall")` | JetBrains Mono | 11.5 px | Event-log times, chips, tooltips |
| `Name("MonoStrong")` | Mono-Bold | 13.5 px (22 px for the recording timer) | Live timer, ETA |

Digit grouping uses a thin space (U+2009) in Inter: `4 362`. Numeric table columns are right-aligned.

## 4. Spacing, sizing, radii

- **Spacing scale (px):** 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24, 28, 32. Card padding is 18 horizontal × 14–16 vertical. Gutters between cards are 14 px. The page margin is 32 × 22 px (22 × 20 px at the minimum size).
- **Control heights:** input and button 36, `sm` 30, `lg` 44, table row 35 (settings rows 50), nav item 38, top bar 68, status bar 30, sidebar 232 wide (64 as a rail).
- **Radii:** `r-sm` 4 (badges, chips-mono, kbd), `r-md` 6 (inputs, sm buttons, segmented inner), `r-lg` 8 (buttons, nav items, segmented track), `r-xl` 12 (cards). The status tile and theme card use 10. Pills (counts, toggles) are fully rounded.
- **Strokes:** 1 px hairlines. Checkbox, radio and toggle borders are 1.5 px. The focus ring is 2 px `prussian-800` plus a 3 px `prussian-150` halo, or a 2 px offset on buttons. The selected-row bar is 3 px.

## 5. Components

States (default → hover → active/pressed → disabled → focus):

- **Primary button**: fill `p-800`, text white, 1 px `p-900` border → `p-700` → `p-900` → fill `sunken`, text `ink-400`, border `line` → 2 px `p-800` ring offset 2 px. Use at most one per view (Add download, Start download, Stop and save).
- **Secondary button**: fill `surface`, text `p-800`, 1 px `p-400` border, card shadow → fill `p-50`, border `p-800` → fill `p-100` → as disabled above → ring. Row actions: Stop and save, Pause, Check now, Open folder, Browse, Detect.
- **Ghost button**: transparent, text `ink-800` → fill `sunken` → fill `sunken-2` → text `ink-400` → ring. Toolbar actions (Pause all, Discard, Probe again).
- **Danger (ghost)**: text `bad-text` → fill `bad-bg` → fill `#F6D9D5`. Use a solid `bad` fill with white text only in a confirmation dialog ("Cancel recording and discard 3.15 GiB?").
- **Icon button**: 36 or 30 px square with the ghost states. It needs a tooltip (`on_hover_text`).
- **Text input**: fill `surface`, 1 px `line-strong` border, radius 6, placeholder `ink-500` → border `p-600` → focus: border `p-800` plus a 3 px `p-150` halo (drawn as a second rounded rect) → disabled: fill `sunken`, border `line`, text `ink-400`. Optional leading icon (`ink-500`) and trailing suffix unit (`ink-600`, e.g. `MiB/s`, `.mkv`). The URL field is 40 px high, uses mono once filled, and shows a trailing hint: `Ctrl L` kbd chips, or "Probed in 0.4 s" in `ok-text`.
- **Dropdown (`ComboBox`)**: looks like an input, with a trailing `caret-down` icon (`ink-600`). The popup is a `surface` card with a 12 px radius and the popup shadow. Rows are 32 px, hover `p-50`, and the selected row is `p-100` with a `check` icon in `p-800`.
- **Segmented control** (custom): track `sunken` with a 1 px `line-strong` border and 3 px inset. Segments are 28 px high; "on" is fill `p-800`, white text and a small shadow; off hover is `sunken-2`. Used for quality, container, theme density and track pickers.
- **Toggle** (custom, 36 × 20): off is fill `surface` with a 1.5 px `line-strong` border and a `line-strong` knob. On is fill `p-800` with a white knob. Hover darkens the border to `p-800`. Disabled uses a `p-200` border and knob. Focus adds the halo.
- **Checkbox / radio**: 16 px, 1.5 px `line-strong` border. Checked is fill `p-800` with a white check (radio: a 10 px `p-800` dot). Disabled uses fill `sunken` and a `p-200` border.
- **Slider**: 4 px track `p-150`, fill `p-800`, 18 px knob (`surface` with a 2 px `p-800` border). Hover gives the knob a `p-50` fill. Mono tick labels (1, 8, 16, 24, 32) sit below it, and the current value is shown in `p-800` SemiBold in the label row.
- **Stepper**: a 32 px bordered group of − / value / +.
- **Badges** (20 px, radius 4, 11 px SemiBold uppercase): `HLS`/`DASH` in mono on `p-100`/`p-700`; `VOD` in `ink-600` on `sunken` with a hairline; `LIVE` in white on `live` with a white dot; `Watching` (warn); `Resumed`/`Default` (info); `DRM` (bad); `Soon`/`Live only` (copper or neutral).
- **Chips** (22 px pills, mono 11.5): track facts such as `1920×1080 avc1`, `en mp4a`, `AES-128`, with a leading 13 px icon. The output-file chip is outlined, not filled.
- **Progress bar**: 6 px high, radius 3, track `p-150`. Fill is `p-700` while downloading, `ok` when completed and `warn` for the watch countdown. A resumed job shows two segments: reused in `p-400`, then new in `p-700`, separated by 2 px of `surface`. A legend with swatches sits underneath.
- **Table**: header row fill `sunken`, Caption style, `ink-600`, 9 px vertical padding. Body rows are 35 px with `line-soft` separators. Hover is fill `p-50`. The selected row is fill `p-100` with a 3 px `p-800` inset bar on the left and a filled radio. Numbers are right-aligned in Inter; codecs and IDs use mono.
- **Job row** (Downloads list): a single card with rows 13 px top and bottom, laid out as a grid `[40 tile][1.15fr identity][1fr progress][196 actions]`.
  - **Tile**: a 40 px rounded 10 box tinted by state. Live is `live-bg` with `broadcast`, downloading is `p-100` with `download-simple`, resumed uses `arrows-clockwise`, watching is `warn-bg` with `eye`, done is `ok-bg` with `check-circle`, and DRM is `sunken` with `shield-slash` in `bad`.
  - **Identity**: name (15 px SemiBold), badges, the mono URL (ellipsised) and chips.
  - **Progress**: a state label (icon + colour) with the primary value next to it and a secondary value right-aligned. Under that sits a bar, a sparkline for live jobs, or explanation text for DRM, then stats.
  - **Actions**: one secondary action plus a `dots-three` menu.
  - Row hover is `p-50`. The selected row is `p-50` with a 3 px `p-800` bar. Clicking a row opens the job details.
- **Sidebar item**: 38 px, radius 8, text `#DCE7F1`, icon `p-300`. Hover fill is `p-750`. Active fill is `p-700` with a white label and icon, a 1 px `#1D5A88` inner stroke and a 3 × 20 px copper tick at the panel edge. Counts are mono 12 px pills on `rgba(0,0,0,.18)`, or `p-800` when active. Focus is a 2 px `p-300` ring.
- **Tabs**: 40 px, Inter-Medium `ink-600`. Active is `p-800` SemiBold with a 2 px `p-800` underline over the 1 px `line` baseline. Count pills are `sunken`, or `p-800` with white text when active. Hover text is `ink-900`.
- **Status bar**: 30 px, fill `sunken`, 1 px `line` top border, 12 px text, separated by 14 px wide hairline separators. From left: total throughput (`p-800` SemiBold with `download-simple`), recording (live dot), downloading, watching, free disk; then on the right: ffmpeg status (`ok` with `check-circle`), ffmpeg path (mono) and core version (mono).
- **Segment map** (custom painter): one cell per segment, 6 × 6 px with a 2 px gap and radius 1.2, wrapped row-major across the available width. Colours by state:
  - DVR window: `p-400`
  - downloaded: `p-700`
  - in flight: `copper`
  - missing: `live`
  - pending: a 1.5 px `p-400` outline only
  - Mono clock labels every 4 rows run down the left gutter (54 px), and the last row is labelled "now" in `live-text`.
  - Hovering a cell draws a 2 px `surface` gap plus a 1.5 px `p-950` outline and shows a tooltip (`p-900`, white and `p-200` text, radius 8): segment number, program time, size and retry history.
  - A legend with counts sits below, followed by "No gaps" or "N segments could not be fetched" (bad-text) on the right. A Video/Audio segmented control switches tracks.
  - For thousands of segments, paint with one `Mesh` of quads per frame.
- **Sparkline** (custom painter): 5 min at 2 s resolution (150 points), a 1.6 px `p-700` polyline over a `p-100` area fill, a dashed `p-400` average line and three hairline gridlines with mono labels (0/4/8 Mbit/s). The current value is a 3.5 px `surface` dot with a 2 px `p-800` ring. The inline row version is 26 px high with no axes.
- **Event log**: 62 px mono time column, a 16 px icon coloured by level (info `p-500`, ok `ok`, warn `warn`, live `live`), and a single-line message. The "Show all N events" footer is on `paper`.
- **Cards**: `surface`, 1 px `line`, radius 12, card shadow. The header is 52 px with a 1 px `line-soft` divider, an 18 px icon in `p-600` and an H3.
- **Spinner**: a 14 px ring (2 px `warn-bg`) with a `warn` arc, rotating. It is the only animation.

## 6. Phosphor icons used (regular weight; `egui-phosphor` constants in brackets)

download-simple (DOWNLOAD_SIMPLE), eye (EYE), eye-slash, clock-counter-clockwise, gear-six, magnifying-glass, plus, link-simple, stop (STOP, FILL variant for Stop and save), pause, play, x, arrow-clockwise, arrows-clockwise, arrow-left, folder-open, folder-simple, dots-three, check, check-circle, warning, warning-circle, info, question, shield-slash, shield-check, lock-simple, lock-key, broadcast, record, timer, gauge, video-camera, speaker-high, closed-captioning, file-video, copy, trash, sliders-horizontal, globe-simple, palette, wrench, terminal-window, hard-drives, list-bullets, keyboard, book-open, sun, moon, caret-down, caret-up-down, caret-right.
Sizes: 18 px in navigation and card headers, 16 px in buttons, 13–15 px in chips and stats, 20 px in job tiles.

## 7. egui mapping (egui ≥ 0.29; field names as in `egui::Visuals` / `egui::Style`)

```rust
let mut v = egui::Visuals::light();
v.panel_fill            = PAPER;          // CentralPanel
v.window_fill           = SURFACE;        // windows, popups
v.extreme_bg_color      = SURFACE;        // TextEdit background
v.faint_bg_color        = P50;            // striped / hovered rows
v.code_bg_color         = SUNKEN;
v.hyperlink_color       = P600;
v.warn_fg_color         = WARN_TEXT;
v.error_fg_color        = BAD_TEXT;
v.window_stroke         = Stroke::new(1.0, LINE);
v.window_corner_radius  = 12.into();      // `window_rounding` before 0.30
v.menu_corner_radius    = 8.into();
v.window_shadow         = Shadow { offset: [0, 4], blur: 14, spread: 0, color: rgba(0x00,0x24,0x3F,26) };
v.popup_shadow          = v.window_shadow;
v.selection.bg_fill     = P150;           // text selection, SelectableLabel
v.selection.stroke      = Stroke::new(1.0, P800);
v.text_cursor.stroke    = Stroke::new(2.0, P800);
v.slider_trailing_fill  = true;           // scope selection.bg_fill = P800 around sliders
v.striped               = false;

let w = &mut v.widgets;
w.noninteractive.bg_fill   = SURFACE;  w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
w.noninteractive.fg_stroke = Stroke::new(1.0, INK_900);
w.inactive.weak_bg_fill    = SURFACE;  w.inactive.bg_fill = SUNKEN;           // button fill / checkbox, slider rail
w.inactive.bg_stroke       = Stroke::new(1.0, LINE_STRONG);
w.inactive.fg_stroke       = Stroke::new(1.0, INK_800);
w.hovered.weak_bg_fill     = P50;      w.hovered.bg_fill = P100;
w.hovered.bg_stroke        = Stroke::new(1.0, P800); w.hovered.fg_stroke = Stroke::new(1.5, P800);
w.hovered.expansion        = 0.0;
w.active.weak_bg_fill      = P100;     w.active.bg_fill = P150;
w.active.bg_stroke         = Stroke::new(1.5, P800); w.active.fg_stroke = Stroke::new(2.0, P900);
w.open                      = w.active;
for s in [&mut w.noninteractive,&mut w.inactive,&mut w.hovered,&mut w.active,&mut w.open] { s.corner_radius = 8.into(); }

let mut st = (*ctx.style()).clone();
st.visuals = v;
st.spacing.item_spacing     = vec2(8.0, 6.0);
st.spacing.button_padding   = vec2(14.0, 9.0);   // 36 px buttons with 14 px Inter
st.spacing.interact_size    = vec2(36.0, 36.0);
st.spacing.window_margin    = Margin::same(16);
st.spacing.menu_margin      = Margin::same(8);
st.spacing.icon_width       = 16.0; st.spacing.icon_spacing = 8.0;
st.spacing.slider_width     = 260.0; st.spacing.combo_width = 200.0;
st.spacing.scroll           = ScrollStyle { bar_width: 6.0, floating: true, ..ScrollStyle::floating() };
st.text_styles = [ (TextStyle::Heading,  FontId::new(17.0, inter_semibold)),
                   (TextStyle::Body,     FontId::new(14.0, inter)),
                   (TextStyle::Button,   FontId::new(14.0, inter_medium)),
                   (TextStyle::Small,    FontId::new(12.0, inter)),
                   (TextStyle::Monospace,FontId::new(12.5, FontFamily::Monospace)),
                   (TextStyle::Name("Title".into()),   FontId::new(26.0, inter_semibold)),
                   (TextStyle::Name("Caption".into()), FontId::new(11.0, inter_semibold)),
                   (TextStyle::Name("Kpi".into()),     FontId::new(24.0, inter_semibold)),
                   (TextStyle::Name("MonoSmall".into()), FontId::new(11.5, FontFamily::Monospace)) ].into();
ctx.set_style(st);
```

Panels (add them in this order so the sidebar runs full height):
1. `SidePanel::left("nav").exact_width(232 | 64).frame(Frame::NONE.fill(P800))`. Paint the two-stop gradient as a `Mesh` with per-vertex colours from #003153 to #002845.
2. `TopBottomPanel::bottom("status").exact_height(30)` with fill `SUNKEN` and a top `LINE` stroke.
3. `TopBottomPanel::top("topbar").exact_height(68)` with fill `SURFACE` and a bottom `LINE` stroke.
4. `CentralPanel` with fill `PAPER` and inner margin 32 × 22.

Implementation notes:
- **Custom widgets:** build the segmented control, toggle, badge, chip, job row, segment map and sparkline as small `impl Widget` painters from rect, fill and stroke primitives.
- **Focus halo:** draw it with `painter.rect_stroke(rect.expand(3.0), r+3, Stroke::new(3.0, P150))`, then the 1–2 px `P800` stroke.
- **Figures:** egui does no OpenType feature shaping, so Inter's proportional digits cannot be switched to tabular. For values that tick (timer, throughput, ETA), use Mono or allocate a fixed-width rect so the layout does not jitter.
- **Window size:** `ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_min_inner_size([1100.0, 700.0])`.

## 8. Layout at the 1100 × 700 minimum (see `screen-1-min.png`)

Breakpoint: central width < 1240 logical px.
- **Sidebar**: collapses to a 64 px icon rail. The logo mark stays and the wordmark, version, captions and labels are hidden. Counts become 16 px copper badges on the icon's corner, and the disk card shrinks to its icon. Every item gets a tooltip.
- **Top bar**: horizontal padding goes to 20. The URL field shrinks first; Probe and Add download keep their labels.
- **Downloads**: the page subtitle is hidden. The job grid becomes `[36][1fr][1fr][auto]` and row actions become 30 px icon-only buttons with tooltips. The output-file chip and third-priority stats (DVR note, segment totals, ETA on resumed jobs, last reply) are hidden. The list becomes a `ScrollArea` with a floating 6 px `p-200` thumb.
- **New download**: the right column narrows from 380 to 340. The variants table drops the Frame rate and ID columns, and the Audio and Subtitles cards stack. The page scrolls vertically; the Start download button stays fixed at the bottom of the Output card.
- **Job details**: the right column narrows to 340 and the Tracks table drops the Details column (shown on hover instead). The segment map reflows to fewer columns at the same 6 px cells and gains a scroll area once it exceeds its card. The KPI strip keeps all 5 cells (about 198 px each).
- **Settings**: stays two columns (about 489 px each) and the label column shrinks from 180 to 150. The page scrolls if needed.
- **Status bar**: drops the free-disk entry and the ffmpeg path.

## 9. Contrast (WCAG 2.x relative luminance, computed by `contrast.py`)

Text pairs must reach 4.5:1 and UI component boundaries or state graphics 3:1. Pairs marked decorative are fills that carry no information on their own; the state is always carried by an adjacent 3:1 element.

| Pair | Foreground | Background | Ratio | Required | Pass |
|---|---|---|---|---|---|
| Body text ink-900 on paper | `#1b2a38` | `#f7f5f0` | 13.43:1 | 4.5 | yes |
| Body text ink-900 on surface | `#1b2a38` | `#fffefb` | 14.51:1 | 4.5 | yes |
| Heading prussian-800 on paper | `#003153` | `#f7f5f0` | 12.32:1 | 4.5 | yes |
| Heading prussian-800 on surface | `#003153` | `#fffefb` | 13.31:1 | 4.5 | yes |
| Muted ink-600 on paper | `#56636f` | `#f7f5f0` | 5.65:1 | 4.5 | yes |
| Muted ink-600 on surface | `#56636f` | `#fffefb` | 6.11:1 | 4.5 | yes |
| Muted ink-600 on sunken | `#56636f` | `#f0ede6` | 5.27:1 | 4.5 | yes |
| Muted ink-600 on selection p-100 | `#56636f` | `#e4edf5` | 5.20:1 | 4.5 | yes |
| Prussian text on selection p-100 | `#003153` | `#e4edf5` | 11.34:1 | 4.5 | yes |
| Sidebar text p-50 on prussian-800 | `#eef3f8` | `#003153` | 12.03:1 | 4.5 | yes |
| Sidebar muted p-300 on prussian-800 | `#9db8ce` | `#003153` | 6.51:1 | 4.5 | yes |
| Sidebar active text white on p-700 | `#ffffff` | `#0b4774` | 9.69:1 | 4.5 | yes |
| Sidebar muted p-300 on prussian-900 | `#9db8ce` | `#00243f` | 7.69:1 | 4.5 | yes |
| Primary btn text on prussian-800 | `#ffffff` | `#003153` | 13.43:1 | 4.5 | yes |
| Primary btn hover p-700 | `#ffffff` | `#0b4774` | 9.69:1 | 4.5 | yes |
| Link/secondary text p-600 on surface | `#1a5a8a` | `#fffefb` | 7.25:1 | 4.5 | yes |
| Accent copper text on surface | `#a8491f` | `#fffefb` | 5.72:1 | 4.5 | yes |
| Success text on success-bg | `#1c6e43` | `#e5f1e9` | 5.38:1 | 4.5 | yes |
| Warning text on warning-bg | `#7f5200` | `#fbf0d8` | 5.97:1 | 4.5 | yes |
| Danger text on danger-bg | `#a8231b` | `#fbe9e6` | 6.13:1 | 4.5 | yes |
| Live text on live-bg | `#b3141f` | `#fde7e8` | 5.85:1 | 4.5 | yes |
| Info text p-700 on p-100 | `#0b4774` | `#e4edf5` | 8.18:1 | 4.5 | yes |
| Mono values ink-800 on surface | `#23384a` | `#fffefb` | 11.99:1 | 4.5 | yes |
| Placeholder ink-500 on input surface | `#6b7682` | `#fffefb` | 4.59:1 | 4.5 | yes |
| Disabled text ink-400 on sunken (exempt, info) | `#8e979f` | `#f0ede6` | 2.54:1 | - | n/a (decorative) |
| Input border line-strong on surface | `#7f8a95` | `#fffefb` | 3.49:1 | 3.0 | yes |
| Input border line-strong on paper | `#7f8a95` | `#f7f5f0` | 3.23:1 | 3.0 | yes |
| Focus ring prussian-800 vs paper | `#003153` | `#f7f5f0` | 12.32:1 | 3.0 | yes |
| Toggle off track ink-500 on surface | `#7f8a95` | `#fffefb` | 3.49:1 | 3.0 | yes |
| Toggle on track prussian-800 on surface | `#003153` | `#fffefb` | 13.31:1 | 3.0 | yes |
| Progress fill prussian-700 vs track p-100 | `#0b4774` | `#dbe6f0` | 7.65:1 | 3.0 | yes |
| Progress track p-150 vs surface (decor) | `#dbe6f0` | `#fffefb` | 1.26:1 | - | n/a (decorative) |
| Live dot on surface | `#d0202e` | `#fffefb` | 5.30:1 | 3.0 | yes |
| Success icon on surface | `#1f7a4a` | `#fffefb` | 5.28:1 | 3.0 | yes |
| Warning icon on surface | `#b07400` | `#fffefb` | 3.90:1 | 3.0 | yes |
| Danger icon on surface | `#b3261e` | `#fffefb` | 6.48:1 | 3.0 | yes |
| Segment downloaded p-700 on surface | `#0b4774` | `#fffefb` | 9.61:1 | 3.0 | yes |
| Segment in-flight copper on surface | `#c2622f` | `#fffefb` | 4.10:1 | 3.0 | yes |
| Segment missing danger on surface | `#d0202e` | `#fffefb` | 5.30:1 | 3.0 | yes |
| Segment pending p-200 on surface (decor) | `#c4d5e4` | `#fffefb` | 1.49:1 | - | n/a (decorative) |
| Segment downloaded vs pending | `#0b4774` | `#c4d5e4` | 6.45:1 | 3.0 | yes |
| Selected row bar prussian-800 on p-100 | `#003153` | `#e4edf5` | 11.34:1 | 3.0 | yes |
| Secondary btn border p-400 on surface | `#6b96bb` | `#fffefb` | 3.11:1 | 3.0 | yes |
| Secondary btn text p-800 on surface | `#003153` | `#fffefb` | 13.31:1 | 4.5 | yes |
| Sidebar nav text on prussian-800 | `#dce7f1` | `#003153` | 10.70:1 | 4.5 | yes |
| Sidebar nav text on hover p-750 | `#dce7f1` | `#083d63` | 9.01:1 | 4.5 | yes |
| Sidebar caption p-300 on sidebar bottom #002845 | `#9db8ce` | `#002845` | 7.33:1 | 4.5 | yes |
| Status bar text ink-600 on sunken | `#56636f` | `#f0ede6` | 5.27:1 | 4.5 | yes |
| Status ok text on sunken | `#1c6e43` | `#f0ede6` | 5.34:1 | 4.5 | yes |
| Badge HLS p-700 on p-100 | `#0b4774` | `#e4edf5` | 8.18:1 | 4.5 | yes |
| Badge VOD ink-600 on sunken | `#56636f` | `#f0ede6` | 5.27:1 | 4.5 | yes |
| Badge LIVE white on live | `#ffffff` | `#d0202e` | 5.35:1 | 4.5 | yes |
| Badge SOON copper-text on copper-bg | `#a8491f` | `#f8ebe2` | 4.94:1 | 4.5 | yes |
| Nav count white on copper (rail) | `#ffffff` | `#b85a2b` | 4.63:1 | 4.5 | yes |
| Live text on selected row p-50 | `#b3141f` | `#f0f5f9` | 6.30:1 | 4.5 | yes |
| Muted ink-600 on selected row p-50 | `#56636f` | `#f0f5f9` | 5.61:1 | 4.5 | yes |
| Tooltip text p-200 on p-900 | `#c4d5e4` | `#00243f` | 10.56:1 | 4.5 | yes |
| Segmented on: white on p-800 | `#ffffff` | `#003153` | 13.43:1 | 4.5 | yes |
| Segmented off text ink-800 on sunken | `#23384a` | `#f0ede6` | 10.35:1 | 4.5 | yes |
| Segment DVR p-400 on surface | `#6b96bb` | `#fffefb` | 3.11:1 | 3.0 | yes |
| Segment pending outline p-400 on surface | `#6b96bb` | `#fffefb` | 3.11:1 | 3.0 | yes |
| Segment DVR p-400 vs downloaded p-700 | `#6b96bb` | `#0b4774` | 3.09:1 | 3.0 | yes |
| Sparkline line p-700 on area p-100 | `#0b4774` | `#e4edf5` | 8.18:1 | 3.0 | yes |
| Slider knob border p-800 on surface | `#003153` | `#fffefb` | 13.31:1 | 3.0 | yes |
| Slider fill p-800 vs track p-150 | `#003153` | `#dbe6f0` | 10.61:1 | 3.0 | yes |
| Dark theme: text p-100 on #0c1a26 | `#e4edf5` | `#0c1a26` | 14.88:1 | 4.5 | yes |
| Dark theme: muted p-300 on #132636 | `#9db8ce` | `#132636` | 7.50:1 | 4.5 | yes |

The segment map and progress bars do not depend on colour alone. Each state has a legend with counts, and missing segments are also reported in text ("N segments could not be fetched"). Pending segments are outlined, not filled, so they differ in shape as well as colour.
