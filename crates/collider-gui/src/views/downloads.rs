//! The Downloads page, and the Watch list (the same rows, limited to watched streams).

use std::path::PathBuf;
use std::time::Instant;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Galley, Id, Layout, Pos2, Rect, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2,
};
use egui_phosphor::regular as ph;
use std::sync::Arc;

use crate::app::{App, Page};
use crate::format;
use crate::jobs::{Intent, Job, JobState, Phase};
use crate::theme::*;
use crate::widgets::{self, Badge, Button, Size, Tile};

/// What a click in a row asks for; applied after the list is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum RowAction {
    Open(u64),
    Stop(u64, Intent),
    Start(u64),
    Discard(u64),
    Remove(u64),
    OpenFolder(PathBuf),
    CopyUrl(String),
}

pub fn apply(app: &mut App, ctx: &egui::Context, action: RowAction) {
    match action {
        RowAction::Open(id) => app.page = Page::Job(id),
        RowAction::Stop(id, intent) => {
            if let Some(j) = app.job_mut(id) {
                j.stop(intent);
            }
        }
        RowAction::Start(id) => {
            let rt = app.rt.clone();
            if let Some(j) = app.job_mut(id) {
                j.start(&rt, ctx);
            }
        }
        RowAction::Discard(id) => {
            let has_data = app.job(id).is_some_and(|j| j.state().bytes() > 0);
            if has_data {
                app.request_discard(id);
            } else {
                app.discard(id);
            }
        }
        RowAction::Remove(id) => app.remove(id),
        RowAction::OpenFolder(p) => app.open_folder(&p),
        RowAction::CopyUrl(url) => ctx.copy_text(url),
    }
}

pub fn page(app: &mut App, ui: &mut Ui, watch: bool) {
    let ctx = ui.ctx().clone();
    let now = Instant::now();
    let today = chrono::Local::now().date_naive();

    let mut open = 0;
    let mut finished_today = 0;
    let mut tab_counts = [0usize; 4];
    for j in &app.jobs {
        let st = j.state();
        tab_counts[0] += 1;
        if st.phase.is_finished() {
            if j.created.date_naive() == today {
                finished_today += 1;
            }
            match st.phase {
                Phase::Completed { .. } => tab_counts[2] += 1,
                Phase::Failed { .. } | Phase::Unsupported { .. } => tab_counts[3] += 1,
                _ => {}
            }
        } else {
            open += 1;
            tab_counts[1] += 1;
        }
    }

    // Header.
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                egui::RichText::new(if watch { "Watch list" } else { "Downloads" })
                    .font(heading())
                    .color(TX1),
            );
            let subtitle = if watch {
                "Streams that are checked until they go live, then recorded".to_string()
            } else {
                format!(
                    "{} job{} · {open} active · {finished_today} finished today",
                    app.jobs.len(),
                    if app.jobs.len() == 1 { "" } else { "s" }
                )
            };
            ui.label(egui::RichText::new(subtitle).font(regular(13.0)).color(TX3));
        });
        if !watch {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let sort = if app.newest_first {
                    "Newest first"
                } else {
                    "Oldest first"
                };
                if ui
                    .add(Button::ghost(sort).icon(ph::SORT_ASCENDING))
                    .on_hover_text("Change the order")
                    .clicked()
                {
                    app.newest_first = !app.newest_first;
                }
                ui.add_space(8.0);
                let labels = [
                    ("All", tab_counts[0]),
                    ("Active", tab_counts[1]),
                    ("Completed", tab_counts[2]),
                    ("Failed", tab_counts[3]),
                ];
                if let Some(i) = widgets::tabs(ui, &labels, app.filter) {
                    app.filter = i;
                }
            });
        }
    });
    ui.add_space(14.0);

    let mut rows: Vec<(u64, bool)> = app
        .jobs
        .iter()
        .filter_map(|j| {
            let st = j.state();
            let finished = st.phase.is_finished();
            let keep = if watch {
                j.spec.wait.is_some() && !finished
            } else {
                match app.filter {
                    1 => !finished,
                    2 => matches!(st.phase, Phase::Completed { .. }),
                    3 => matches!(st.phase, Phase::Failed { .. } | Phase::Unsupported { .. }),
                    _ => true,
                }
            };
            keep.then_some((j.id, finished))
        })
        .collect();
    if app.newest_first {
        rows.reverse();
    }

    if rows.is_empty() {
        empty_state(app, ui, watch);
        return;
    }

    let compact = ui.available_width() < 1000.0;
    let mut actions = Vec::new();
    let active: Vec<u64> = rows.iter().filter(|r| !r.1).map(|r| r.0).collect();
    let done: Vec<u64> = rows.iter().filter(|r| r.1).map(|r| r.0).collect();
    if !active.is_empty() {
        widgets::section_header(
            ui,
            if watch { "Watching" } else { "Active" },
            active.len(),
            None,
        );
        ui.add_space(4.0);
        for id in &active {
            if let Some(job) = app.job(*id) {
                actions.extend(job_row(ui, job, now, compact));
            }
            ui.add_space(8.0);
        }
    }
    if !done.is_empty() {
        ui.add_space(10.0);
        if widgets::section_header(ui, "Finished", done.len(), Some("Clear finished")) {
            for id in &done {
                actions.push(RowAction::Remove(*id));
            }
        }
        ui.add_space(4.0);
        for id in &done {
            if let Some(job) = app.job(*id) {
                actions.extend(job_row(ui, job, now, compact));
            }
            ui.add_space(8.0);
        }
    }
    if !watch && !compact {
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.horizontal(|ui| {
                let text = "Ctrl+L jumps to the link field. Finished jobs are kept in History.";
                let w = ui
                    .painter()
                    .layout_no_wrap(text.into(), small(), TX3)
                    .size()
                    .x
                    + 22.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                ui.label(widgets::icon_text(ph::INFO, 13.0, TX3));
                ui.label(egui::RichText::new(text).font(small()).color(TX3));
            });
        });
    }
    for a in actions {
        apply(app, &ctx, a);
    }
}

fn empty_state(app: &mut App, ui: &mut Ui, watch: bool) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space(26.0);
            ui.label(widgets::icon_text(
                if watch {
                    ph::BINOCULARS
                } else {
                    ph::DOWNLOAD_SIMPLE
                },
                34.0,
                P300,
            ));
            ui.add_space(6.0);
            let (title, body) = if watch {
                (
                    "No streams are being watched",
                    "Probe a link that is not live yet and choose Wait for it: collider checks \
                     it on a timer and starts recording the moment the broadcast begins.",
                )
            } else if app.filter != 0 && !app.jobs.is_empty() {
                ("Nothing here", "No jobs match this filter.")
            } else {
                (
                    "No downloads yet",
                    "Paste a link to an HLS (.m3u8) or DASH (.mpd) stream above. Probe shows \
                     what it offers; Add starts right away with your default settings.",
                )
            };
            ui.label(egui::RichText::new(title).font(card_title()).color(TX1));
            ui.add_space(2.0);
            ui.label(egui::RichText::new(body).font(regular(13.0)).color(TX2));
            ui.add_space(26.0);
        });
    });
}

/// A laid-out single line, cut with an ellipsis at `max_width`.
fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width(max_width.max(10.0));
    ui.painter().layout_job(job)
}

/// One piece of a stat line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S {
    /// A number or value: mono `TX1`.
    Value,
    /// A word: small `TX3`.
    Word,
    /// A value that is good news (0 gaps): mono `OK`.
    Good,
    /// A value that is bad news: mono `BAD`.
    Bad,
}

pub type StatGroup = Vec<(String, S)>;

/// A stat line: groups of values and words, spaced apart.
fn stat_job(groups: &[StatGroup]) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (g, group) in groups.iter().enumerate() {
        for (i, (text, kind)) in group.iter().enumerate() {
            let gap = match (g, i) {
                (0, 0) => 0.0,
                (_, 0) => 14.0,
                _ => 4.0,
            };
            let (font_id, color) = match kind {
                S::Value => (mono(12.0), TX1),
                S::Word => (small(), TX3),
                S::Good => (mono(12.0), OK),
                S::Bad => (mono(12.0), BAD),
            };
            job.append(
                text,
                gap,
                TextFormat {
                    font_id,
                    color,
                    valign: Align::Center,
                    ..Default::default()
                },
            );
        }
    }
    job
}

fn val(text: impl Into<String>) -> (String, S) {
    (text.into(), S::Value)
}

fn word(text: impl Into<String>) -> (String, S) {
    (text.into(), S::Word)
}

pub enum Bar {
    None,
    Parts(Vec<(f32, Color32)>),
    Live { in_flight: bool },
}

pub struct RowView {
    pub tile: (Tile, &'static str),
    pub badges: Vec<(String, Badge)>,
    pub headline: String,
    pub headline_color: Color32,
    pub detail: String,
    pub right: Option<(String, Color32)>,
    pub bar: Bar,
    pub stats: Vec<StatGroup>,
    /// Two lines of plain explanation instead of a bar (failed, DRM).
    pub note: Option<String>,
    pub metric: String,
    pub metric_sub: String,
}

fn protocol_badge(job: &Job, st: &JobState) -> Option<String> {
    st.protocol.map(|p| p.to_string()).or_else(|| {
        let path = job.spec.url.split(['?', '#']).next().unwrap_or("");
        if path.ends_with(".m3u8") || path.ends_with(".m3u") {
            Some("HLS".into())
        } else if path.ends_with(".mpd") {
            Some("DASH".into())
        } else {
            None
        }
    })
}

pub fn row_view(job: &Job, st: &JobState, now: Instant) -> RowView {
    let mut badges = Vec::new();
    if st.live && st.phase.is_running() && !matches!(st.phase, Phase::Waiting { .. }) {
        badges.push(("Live".to_string(), Badge::Live));
    }
    if let Some(p) = protocol_badge(job, st) {
        badges.push((p, Badge::Proto));
    }
    let (done, total) = (st.done(), st.total());
    let pct = st.progress().map(|p| (p * 100.0).floor() as u32);
    let parts = |st: &JobState| -> Vec<(f32, Color32)> {
        let total = st.total().max(1) as f32;
        let resumed = st.resumed() as f32 / total;
        let rest = (st.settled() - st.resumed()) as f32 / total;
        vec![(resumed, P500), (rest, P300)]
    };
    let mut v = RowView {
        tile: (Tile::Download, ph::DOWNLOAD_SIMPLE),
        badges: Vec::new(),
        headline: String::new(),
        headline_color: TX1,
        detail: String::new(),
        right: None,
        bar: Bar::None,
        stats: Vec::new(),
        note: None,
        metric: "–".into(),
        metric_sub: String::new(),
    };
    let summary = st.summary();
    let summary_group = || (!summary.is_empty()).then(|| vec![word(summary.clone())]);
    match &st.phase {
        Phase::Starting | Phase::Restarting => {
            v.tile = (Tile::Busy, ph::MAGNIFYING_GLASS);
            v.headline = "Starting".into();
            v.detail = "reading the manifest".into();
            v.bar = Bar::Parts(vec![]);
            v.stats = vec![vec![word(format::host(&job.spec.url))]];
        }
        Phase::Waiting {
            next_check,
            checks,
            reason,
        } => {
            badges.push(("Watching".into(), Badge::Info));
            v.tile = (Tile::Watch, ph::BINOCULARS);
            v.headline = "Waiting for the broadcast to start".into();
            let left = next_check.saturating_duration_since(now).as_secs_f64();
            let interval = job.spec.wait.map_or(20.0, |d| d.as_secs_f64()).max(1.0);
            v.right = Some((format!("next check {} s", left.ceil() as u64), TX2));
            v.bar = Bar::Parts(vec![(1.0 - (left / interval) as f32, INFO)]);
            v.stats = vec![
                vec![word("Checked"), val(checks.to_string()), word("times")],
                vec![word("last reply"), val(short_reason(reason))],
            ];
            v.metric_sub = format!("every {} s", interval as u64);
        }
        Phase::Running | Phase::Stopping if st.live => {
            v.tile = (Tile::Live, ph::RECORD);
            v.headline = if st.phase == Phase::Stopping {
                "Stopping".into()
            } else {
                "Recording".into()
            };
            v.detail = if st.phase == Phase::Stopping {
                format!("saving {} segment(s) in flight", st.in_flight())
            } else {
                "following the live edge".into()
            };
            v.right = Some((format::clock(st.elapsed(now)), AC));
            v.bar = Bar::Live {
                in_flight: st.in_flight() > 0,
            };
            let gaps = st.gaps();
            v.stats = vec![
                vec![val(format::count_mono(done)), word("segments")],
                vec![
                    (gaps.to_string(), if gaps == 0 { S::Good } else { S::Bad }),
                    word("gaps"),
                ],
            ];
            v.stats.extend(summary_group());
            v.metric = format::bitrate(st.rate.current(now) * 8.0);
            v.metric_sub = format!("{} saved", format::bytes(st.bytes()));
        }
        Phase::Running | Phase::Stopping => {
            let resumed = st.resumed() > 0;
            if resumed {
                badges.push(("Resumed".into(), Badge::Info));
            }
            v.tile = if resumed {
                (Tile::Resume, ph::ARROW_CLOCKWISE)
            } else {
                (Tile::Download, ph::DOWNLOAD_SIMPLE)
            };
            if st.phase == Phase::Stopping {
                v.headline = "Pausing".into();
                v.detail = format!("finishing {} segment(s) in flight", st.in_flight());
            } else {
                v.headline = "Downloading".into();
                v.detail = format!("{} parallel", job.spec.concurrency);
            }
            v.right = pct.map(|p| (format!("{p} %"), TX1));
            v.bar = Bar::Parts(parts(st));
            let mut first = if resumed {
                vec![
                    val(format::count_mono(st.resumed())),
                    word(format!("of {} segments reused", format::count(total))),
                ]
            } else {
                vec![val(format::bytes(st.bytes()))]
            };
            if !resumed {
                if let Some(estimate) = estimated_total(st) {
                    first.push(word("of"));
                    first.push(val(format!("~{}", format::bytes(estimate))));
                }
            }
            v.stats = vec![first];
            if let Some(eta) = st.eta(now) {
                v.stats
                    .push(vec![word("ETA"), val(format::short_duration(eta))]);
            }
            v.stats.extend(summary_group());
            v.metric = format::speed(st.rate.current(now));
            v.metric_sub = format!("{} / {} seg", format::count(done), format::count(total));
        }
        Phase::Remuxing => {
            v.tile = (Tile::Busy, ph::FILM_STRIP);
            v.headline = "Remuxing".into();
            v.detail = "ffmpeg, stream copy".into();
            v.bar = Bar::Parts(vec![(1.0, P300)]);
            v.stats = vec![vec![val(format::bytes(st.bytes())), word("captured")]];
            v.metric = format::bytes(st.bytes());
        }
        Phase::Paused => {
            badges.push(("Paused".into(), Badge::Soon));
            v.tile = (Tile::Paused, ph::PAUSE);
            v.headline = "Paused".into();
            v.detail = "resumes where it stopped".into();
            v.right = pct.map(|p| (format!("{p} %"), TX2));
            v.bar = Bar::Parts(parts(st).into_iter().map(|(f, _)| (f, P500)).collect());
            v.stats = vec![vec![
                val(format::count_mono(done)),
                word(format!("of {} segments on disk", format::count(total))),
            ]];
            v.metric_sub = format!("{} on disk", format::bytes(st.bytes()));
        }
        Phase::Completed { files } => {
            if !st.live {
                badges.push(("VOD".into(), Badge::Outline));
            } else {
                badges.push(("Recorded".into(), Badge::Outline));
            }
            v.tile = (Tile::Done, ph::CHECK_CIRCLE);
            v.headline = "Completed".into();
            let remuxed = files.len() == 1
                && files[0].extension().and_then(|e| e.to_str())
                    == job.spec.output.extension().and_then(|e| e.to_str());
            v.detail = if remuxed {
                "remuxed with ffmpeg (stream copy)".into()
            } else {
                "raw stream files (no ffmpeg)".into()
            };
            v.right = Some(("100 %".into(), TX1));
            v.bar = Bar::Parts(vec![(1.0, OK)]);
            let elapsed = st.elapsed(now);
            v.stats = vec![
                vec![val(format::bytes(st.bytes()))],
                vec![word("in"), val(format::short_duration(elapsed))],
            ];
            if st.gaps() > 0 {
                v.stats
                    .push(vec![(st.gaps().to_string(), S::Bad), word("gaps")]);
            }
            v.stats.extend(summary_group());
            if let Some(at) = st.finished_wall {
                v.stats
                    .push(vec![word("finished"), val(at.format("%H:%M").to_string())]);
            }
            v.metric = format::bytes(st.bytes());
            v.metric_sub = if elapsed > 0.5 {
                format!("avg {}", format::speed(st.fetched as f64 / elapsed))
            } else {
                String::new()
            };
        }
        Phase::Failed { message } => {
            badges.push(("Failed".into(), Badge::Warn));
            v.tile = (Tile::Neutral, ph::WARNING);
            v.headline = "Download failed".into();
            v.headline_color = TX1;
            v.note = Some(message.clone());
            v.metric_sub = if st.bytes() > 0 {
                format!("{} kept", format::bytes(st.bytes()))
            } else {
                String::new()
            };
        }
        Phase::Unsupported { .. } => {
            badges.push(("Not supported".into(), Badge::Bad));
            v.tile = (Tile::Neutral, ph::SHIELD_WARNING);
            v.headline = "This stream is DRM-protected".into();
            v.note = Some(
                "collider only saves streams served without protection, so nothing was \
                 downloaded."
                    .into(),
            );
        }
        Phase::Cancelled => {
            v.tile = (Tile::Neutral, ph::X);
            v.headline = "Cancelled".into();
            v.note = Some("Everything this job downloaded was deleted.".into());
        }
    }
    v.badges = badges;
    v
}

/// The download's likely size, extrapolated from the segments done so far.
fn estimated_total(st: &JobState) -> Option<u64> {
    let done = st.done();
    let total = st.total();
    (done >= 3 && total > done).then(|| (st.bytes() as f64 / done as f64 * total as f64) as u64)
}

/// `stream is not live yet: HTTP 404 for https://…` → `HTTP 404`.
fn short_reason(reason: &str) -> String {
    let r = reason.split(" for http").next().unwrap_or(reason);
    let r = r.rsplit(": ").next().unwrap_or(r);
    crate::app::truncate_middle(r, 40)
}

/// Draw one job row; returns what was clicked.
pub fn job_row(ui: &mut Ui, job: &Job, now: Instant, compact: bool) -> Vec<RowAction> {
    let mut actions = Vec::new();
    let st = job.state();
    let view = row_view(job, &st, now);
    let phase = st.phase.clone();
    let live = st.live;
    let has_data = st.bytes() > 0;
    drop(st);

    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 84.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return actions;
    }
    let row = ui
        .interact(rect, Id::new(("job-row", job.id)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    row.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Show {}", job.name))
    });
    let hover = row.contains_pointer();
    let painter = ui.painter().clone();
    painter.add(card_shadow().as_shape(rect, egui::CornerRadius::same(CARD_RADIUS)));
    painter.rect(
        rect,
        CARD_RADIUS,
        if hover { JOB_HOVER } else { P800 },
        Stroke::new(1.0, if hover { LINE2 } else { LINE }),
        StrokeKind::Inside,
    );
    if row.clicked() {
        actions.push(RowAction::Open(job.id));
    }

    let inner = rect.shrink2(Vec2::new(16.0, 14.0));
    let gap = if compact { 16.0 } else { 20.0 };
    let actions_w = 176.0;
    let metrics_w = if compact { 92.0 } else { 118.0 };
    let rest = (inner.width() - 40.0 - gap * 4.0 - actions_w - metrics_w).max(200.0);
    let name_w = (rest * 0.42).max(150.0);
    let prog_w = rest - name_w;

    // State tile.
    let tile = Rect::from_min_size(
        Pos2::new(inner.left(), inner.center().y - 20.0),
        Vec2::splat(40.0),
    );
    widgets::paint_tile(ui, tile, view.tile.0, view.tile.1);

    // Name and meta.
    let x = tile.right() + gap;
    let name = elided(ui, &job.name, card_title(), TX1, name_w);
    painter.galley(Pos2::new(x, inner.top() + 3.0), name, TX1);
    let mut bx = x;
    let by = inner.top() + 30.0;
    for (text, kind) in &view.badges {
        let size = widgets::badge_size(ui, text, *kind);
        if bx + size.x > x + name_w {
            break;
        }
        let r = widgets::paint_badge(ui, Pos2::new(bx, by), text, *kind);
        bx = r.right() + 6.0;
    }
    let host_w = x + name_w - bx - 2.0;
    if host_w > 40.0 {
        let host = elided(ui, &format::host(&job.spec.url), small(), TX3, host_w);
        let h = host.size().y;
        painter.galley(Pos2::new(bx + 2.0, by + 10.0 - h / 2.0), host, TX3);
    }

    // Progress block.
    let px = x + name_w + gap;
    let mut head = LayoutJob::default();
    head.append(
        &view.headline,
        0.0,
        TextFormat {
            font_id: body_strong(),
            color: view.headline_color,
            ..Default::default()
        },
    );
    if !view.detail.is_empty() {
        head.append(
            &format!(" · {}", view.detail),
            0.0,
            TextFormat {
                font_id: regular(13.0),
                color: TX2,
                ..Default::default()
            },
        );
    }
    let right_w = view.right.as_ref().map_or(0.0, |(t, _)| {
        painter.layout_no_wrap(t.clone(), mono(12.5), TX1).size().x
    });
    head.wrap = TextWrapping::truncate_at_width((prog_w - right_w - 12.0).max(40.0));
    let head = painter.layout_job(head);
    let head_h = head.size().y;
    painter.galley(Pos2::new(px, inner.top() + 12.0 - head_h / 2.0), head, TX1);
    if let Some((text, color)) = &view.right {
        painter.text(
            Pos2::new(px + prog_w, inner.top() + 12.0),
            Align2::RIGHT_CENTER,
            text,
            mono(12.5),
            *color,
        );
    }
    let bar = Rect::from_min_size(Pos2::new(px, inner.top() + 25.0), Vec2::new(prog_w, 6.0));
    match &view.bar {
        Bar::None => {}
        Bar::Parts(parts) => widgets::paint_progress(ui, bar, parts),
        Bar::Live { in_flight } => widgets::paint_live_strip(ui, bar, *in_flight),
    }
    if let Some(note) = &view.note {
        let mut job_text = LayoutJob::single_section(
            note.clone(),
            TextFormat {
                font_id: regular(12.5),
                color: TX2,
                ..Default::default()
            },
        );
        job_text.wrap = TextWrapping {
            max_width: prog_w,
            max_rows: 2,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let g = painter.layout_job(job_text);
        painter.galley(Pos2::new(px, inner.top() + 24.0), g, TX2);
    } else if !view.stats.is_empty() {
        let mut stats = stat_job(&view.stats);
        stats.wrap = TextWrapping::truncate_at_width(prog_w);
        let g = painter.layout_job(stats);
        let h = g.size().y;
        painter.galley(Pos2::new(px, inner.top() + 45.0 - h / 2.0), g, TX1);
    }

    // Metrics.
    let mx = px + prog_w + gap + metrics_w;
    painter.text(
        Pos2::new(mx, inner.top() + 16.0),
        Align2::RIGHT_CENTER,
        &view.metric,
        mono(if compact { 12.5 } else { 14.0 }),
        TX1,
    );
    if !compact && !view.metric_sub.is_empty() {
        painter.text(
            Pos2::new(mx, inner.top() + 38.0),
            Align2::RIGHT_CENTER,
            &view.metric_sub,
            small(),
            TX3,
        );
    }

    // Actions, right-aligned.
    let actions_rect = Rect::from_min_max(
        Pos2::new(inner.right() - actions_w, inner.top()),
        inner.right_bottom(),
    );
    let id = job.id;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(actions_rect)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let more = widgets::icon_button(ui, ph::DOTS_THREE, "More", 28.0);
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(190.0);
                if menu_item(ui, ph::INFO, "Show details") {
                    actions.push(RowAction::Open(id));
                }
                if menu_item(ui, ph::COPY, "Copy link") {
                    actions.push(RowAction::CopyUrl(job.spec.url.clone()));
                }
                if menu_item(ui, ph::FOLDER_OPEN, "Open folder") {
                    actions.push(RowAction::OpenFolder(job.spec.output.clone()));
                }
                if phase.is_finished() && menu_item(ui, ph::X, "Remove from list") {
                    actions.push(RowAction::Remove(id));
                }
            });
            match &phase {
                Phase::Running | Phase::Stopping | Phase::Remuxing if live => {
                    if phase == Phase::Running
                        && ui
                            .add(
                                Button::primary("Stop and save")
                                    .icon(ph::STOP)
                                    .filled()
                                    .size(Size::Small),
                            )
                            .on_hover_text("Stop recording and save what was captured")
                            .clicked()
                    {
                        actions.push(RowAction::Stop(id, Intent::Save));
                    }
                }
                Phase::Running => {
                    if widgets::icon_button(ui, ph::X, "Cancel and delete", 28.0).clicked() {
                        actions.push(RowAction::Discard(id));
                    }
                    if ui
                        .add(
                            Button::secondary("Pause")
                                .icon(ph::PAUSE)
                                .filled()
                                .size(Size::Small),
                        )
                        .on_hover_text("Stop for now; resume later where it stopped")
                        .clicked()
                    {
                        actions.push(RowAction::Stop(id, Intent::Pause));
                    }
                }
                Phase::Starting | Phase::Restarting => {
                    if widgets::icon_button(ui, ph::X, "Cancel", 28.0).clicked() {
                        actions.push(RowAction::Discard(id));
                    }
                }
                Phase::Waiting { .. } => {
                    if widgets::icon_button(ui, ph::X, "Stop watching", 28.0).clicked() {
                        actions.push(RowAction::Discard(id));
                    }
                    if ui
                        .add(
                            Button::secondary("Check now")
                                .icon(ph::ARROWS_CLOCKWISE)
                                .size(Size::Small),
                        )
                        .clicked()
                    {
                        actions.push(RowAction::Stop(id, Intent::Restart));
                    }
                }
                Phase::Paused => {
                    if widgets::icon_button(ui, ph::X, "Cancel and delete", 28.0).clicked() {
                        actions.push(RowAction::Discard(id));
                    }
                    if ui
                        .add(Button::secondary("Resume").icon(ph::PLAY).size(Size::Small))
                        .clicked()
                    {
                        actions.push(RowAction::Start(id));
                    }
                }
                Phase::Completed { files } => {
                    if ui
                        .add(
                            Button::secondary("Open folder")
                                .icon(ph::FOLDER_OPEN)
                                .size(Size::Small),
                        )
                        .clicked()
                    {
                        let target = files.first().cloned().unwrap_or(job.spec.output.clone());
                        actions.push(RowAction::OpenFolder(target));
                    }
                }
                Phase::Failed { .. } => {
                    let trash = widgets::icon_button(
                        ui,
                        ph::TRASH,
                        if has_data {
                            "Delete the downloaded parts"
                        } else {
                            "Remove from list"
                        },
                        28.0,
                    );
                    if trash.clicked() {
                        actions.push(if has_data {
                            RowAction::Discard(id)
                        } else {
                            RowAction::Remove(id)
                        });
                    }
                    if ui
                        .add(
                            Button::secondary("Retry")
                                .icon(ph::ARROW_CLOCKWISE)
                                .size(Size::Small),
                        )
                        .on_hover_text("Try again; finished segments are reused")
                        .clicked()
                    {
                        actions.push(RowAction::Start(id));
                    }
                }
                Phase::Unsupported { .. } | Phase::Cancelled => {
                    if widgets::icon_button(ui, ph::TRASH, "Remove from list", 28.0).clicked() {
                        actions.push(RowAction::Remove(id));
                    }
                    if matches!(phase, Phase::Unsupported { .. })
                        && ui
                            .add(Button::ghost("Learn why").icon(ph::INFO).size(Size::Small))
                            .clicked()
                    {
                        actions.push(RowAction::Open(id));
                    }
                }
                _ => {}
            }
        },
    );
    actions
}

/// A full-width menu row; returns true when clicked.
pub fn menu_item(ui: &mut Ui, icon_str: &str, text: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(170.0), 30.0),
        Sense::click(),
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6, P700);
    }
    ui.painter().text(
        Pos2::new(rect.left() + 16.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon_str,
        icon(15.0),
        TX2,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        regular(13.0),
        TX1,
    );
    let clicked = resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
    if clicked {
        ui.close();
    }
    clicked
}
