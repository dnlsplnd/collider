//! The job details page: KPIs, the segment map, throughput, tracks, the event log and
//! where the output goes.

use std::time::Instant;

use egui::{
    Align, Align2, Color32, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui,
    Vec2,
};
use egui_phosphor::regular as ph;

use crate::app::{App, Page};
use crate::format;
use crate::jobs::{Cell, Intent, JobState, Level, Phase, BUCKETS};
use crate::theme::*;
use crate::views::downloads::{self, RowAction};
use crate::widgets::{self, Badge, Button, Size};

pub fn page(app: &mut App, ui: &mut Ui, id: u64) {
    let ctx = ui.ctx().clone();
    let now = Instant::now();
    let Some(job) = app.job(id) else {
        app.page = Page::Downloads;
        return;
    };
    let mut actions = Vec::new();
    let ffmpeg = app.ffmpeg_status();
    let volume_free = app
        .volume
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|v| v.free);
    let st = job.state();
    let view = downloads::row_view(job, &st, now);
    let narrow = ui.available_width() < 1040.0;

    if widgets::breadcrumb(ui, &job.name) {
        actions.push(Nav::Back);
    }
    ui.add_space(10.0);

    // Header card with the KPI strip.
    widgets::card()
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 16))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (tile, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                        widgets::paint_tile(ui, tile, view.tile.0, view.tile.1);
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 5.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new(&job.name).font(title()).color(TX1));
                                ui.add_space(4.0);
                                for (text, kind) in &view.badges {
                                    widgets::badge_ui(ui, text, *kind);
                                }
                            });
                            ui.label(
                                RichText::new(crate::app::truncate_middle(&job.spec.url, 96))
                                    .font(mono(12.5))
                                    .color(TX2),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            header_actions(ui, &st, job.id, &job.spec.output, &mut actions);
                        });
                    });
                });
            let r = ui.min_rect();
            ui.painter()
                .hline(r.x_range(), ui.cursor().top(), Stroke::new(1.0, LINE));
            kpis(ui, &st, job.spec.concurrency, volume_free, now, narrow);
        });
    ui.add_space(14.0);

    if let Some(note) = &view.note {
        widgets::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&view.headline).font(card_title()).color(TX1));
            ui.label(RichText::new(note).font(regular(13.0)).color(TX2));
            if matches!(st.phase, Phase::Unsupported { .. }) {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "DRM systems such as Widevine, PlayReady and FairPlay encrypt the media \
                         with keys only licensed players receive. Getting around that is \
                         circumvention, which collider deliberately does not do. Streams with \
                         clear-key AES-128 (the key is served openly) download normally.",
                    )
                    .font(small())
                    .color(TX3),
                );
            }
        });
        ui.add_space(14.0);
    }

    // Segment map.
    if st.total() > 0 {
        segment_map(ui, &st);
        ui.add_space(14.0);
    }

    // Two columns: throughput + log | tracks + output.
    let gap = 14.0;
    let full = ui.available_width();
    let left_w = if narrow { full } else { (full - gap) * 0.56 };
    let right_w = if narrow { full } else { full - gap - left_w };
    let columns = |ui: &mut Ui, left: &mut dyn FnMut(&mut Ui), right: &mut dyn FnMut(&mut Ui)| {
        if narrow {
            left(ui);
            ui.add_space(gap);
            right(ui);
        } else {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(left_w);
                        left(ui);
                    },
                );
                ui.add_space(gap - ui.spacing().item_spacing.x);
                ui.allocate_ui_with_layout(
                    Vec2::new(right_w, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(right_w);
                        right(ui);
                    },
                );
            });
        }
    };
    let mut copy_log = None;
    columns(
        ui,
        &mut |ui| {
            ui.spacing_mut().item_spacing.y = gap;
            throughput(ui, &st, now);
            if let Some(text) = event_log(ui, &st) {
                copy_log = Some(text);
            }
        },
        &mut |ui| {
            ui.spacing_mut().item_spacing.y = gap;
            tracks(ui, &st);
            output_card(ui, &st, &job.spec, ffmpeg.as_ref());
        },
    );
    drop(st);

    if let Some(text) = copy_log {
        ctx.copy_text(text);
    }
    for a in actions {
        match a {
            Nav::Back => app.page = Page::Downloads,
            Nav::Row(a) => downloads::apply(app, &ctx, a),
        }
    }
}

enum Nav {
    Back,
    Row(RowAction),
}

fn header_actions(
    ui: &mut Ui,
    st: &JobState,
    id: u64,
    output: &std::path::Path,
    actions: &mut Vec<Nav>,
) {
    ui.spacing_mut().item_spacing.x = 8.0;
    match &st.phase {
        Phase::Running if st.live => {
            if ui
                .add(Button::primary("Stop and save").icon(ph::STOP).filled())
                .on_hover_text("Stop recording and save what was captured")
                .clicked()
            {
                actions.push(Nav::Row(RowAction::Stop(id, Intent::Save)));
            }
            if ui.add(Button::danger("Cancel").icon(ph::X)).clicked() {
                actions.push(Nav::Row(RowAction::Discard(id)));
            }
        }
        Phase::Running => {
            if ui
                .add(Button::secondary("Pause").icon(ph::PAUSE).filled())
                .clicked()
            {
                actions.push(Nav::Row(RowAction::Stop(id, Intent::Pause)));
            }
            if ui.add(Button::danger("Cancel").icon(ph::X)).clicked() {
                actions.push(Nav::Row(RowAction::Discard(id)));
            }
        }
        Phase::Waiting { .. } => {
            if ui
                .add(Button::secondary("Check now").icon(ph::ARROWS_CLOCKWISE))
                .clicked()
            {
                actions.push(Nav::Row(RowAction::Stop(id, Intent::Restart)));
            }
            if ui
                .add(Button::danger("Stop watching").icon(ph::X))
                .clicked()
            {
                actions.push(Nav::Row(RowAction::Discard(id)));
            }
        }
        Phase::Paused => {
            if ui.add(Button::primary("Resume").icon(ph::PLAY)).clicked() {
                actions.push(Nav::Row(RowAction::Start(id)));
            }
            if ui.add(Button::danger("Cancel").icon(ph::X)).clicked() {
                actions.push(Nav::Row(RowAction::Discard(id)));
            }
        }
        Phase::Failed { .. } => {
            if ui
                .add(Button::primary("Retry").icon(ph::ARROW_CLOCKWISE))
                .clicked()
            {
                actions.push(Nav::Row(RowAction::Start(id)));
            }
        }
        Phase::Completed { .. } | Phase::Unsupported { .. } | Phase::Cancelled => {
            let remove = ui
                .add(Button::secondary("Remove from list").icon(ph::X))
                .clicked();
            if remove {
                actions.push(Nav::Row(RowAction::Remove(id)));
            }
        }
        _ => {}
    }
    if ui
        .add(Button::secondary("Open folder").icon(ph::FOLDER_OPEN))
        .clicked()
    {
        let target = match &st.phase {
            Phase::Completed { files } => files.first().cloned(),
            _ => None,
        }
        .unwrap_or_else(|| output.to_path_buf());
        actions.push(Nav::Row(RowAction::OpenFolder(target)));
    }
}

fn kpis(
    ui: &mut Ui,
    st: &JobState,
    concurrency: usize,
    volume_free: Option<u64>,
    now: Instant,
    narrow: bool,
) {
    let rate = st.rate.current(now);
    let elapsed = st.elapsed(now);
    let avg = if elapsed > 0.5 {
        st.fetched as f64 / elapsed
    } else {
        0.0
    };
    let per_track = st
        .tracks
        .iter()
        .map(|t| format!("{} {}", format::count(t.done), t.name))
        .collect::<Vec<_>>()
        .join(" · ");
    let (bytes_v, bytes_u) = format::bytes_split(st.bytes());
    let started = st
        .capture_started
        .map(|(_, at)| format!("since {}", at.format("%H:%M:%S")))
        .unwrap_or_else(|| "not started".into());
    let (rate_v, rate_u, rate_sub) = if st.live {
        let (v, u) = format::bitrate_split(rate * 8.0);
        let (avg_v, _) = format::bitrate_split(avg * 8.0);
        let (peak_v, peak_u) = format::bitrate_split(st.rate.peak(now) * 8.0);
        (
            v,
            u.to_string(),
            format!("avg {avg_v} · peak {peak_v} {peak_u}"),
        )
    } else {
        let s = format::speed(rate);
        let (v, u) = s
            .split_once(' ')
            .map_or((s.clone(), ""), |(a, b)| (a.to_string(), b));
        (v, u.to_string(), format!("avg {}", format::speed(avg)))
    };
    let gaps = st.gaps();
    let cells: Vec<(&str, String, String, Color32, String)> = vec![
        (
            if st.live { "Recorded" } else { "Elapsed" },
            format::clock(elapsed),
            String::new(),
            if st.live { AC } else { TX1 },
            started,
        ),
        (
            "Segments",
            format::count_mono(st.done()),
            if st.live {
                String::new()
            } else {
                format!("/ {}", format::count_mono(st.total()))
            },
            TX1,
            if per_track.is_empty() {
                "–".into()
            } else {
                per_track
            },
        ),
        ("Throughput", rate_v, rate_u, TX1, rate_sub),
        (
            "Saved",
            bytes_v,
            bytes_u.to_string(),
            TX1,
            volume_free
                .map(|f| format!("{} free on volume", format::bytes(f)))
                .unwrap_or_default(),
        ),
        (
            "Gaps",
            gaps.to_string(),
            String::new(),
            if gaps == 0 { OK } else { BAD },
            if st.live {
                "missing live segments".into()
            } else {
                "VOD downloads never skip".into()
            },
        ),
        (
            "In flight",
            st.in_flight().to_string(),
            String::new(),
            TX1,
            format!("of {concurrency} parallel"),
        ),
    ];
    let per_row = if narrow { 3 } else { 6 };
    let width = ui.available_width();
    let cell_w = width / per_row as f32;
    for row in cells.chunks(per_row) {
        let (r, _) = ui.allocate_exact_size(Vec2::new(width, 76.0), Sense::hover());
        for (i, (label, value, unit, color, sub)) in row.iter().enumerate() {
            let c = Rect::from_min_size(
                Pos2::new(r.left() + i as f32 * cell_w, r.top()),
                Vec2::new(cell_w, r.height()),
            );
            if i > 0 {
                ui.painter()
                    .vline(c.left(), c.y_range(), Stroke::new(1.0, LINE));
            }
            let p = ui.painter();
            let x = c.left() + 18.0;
            p.text(
                Pos2::new(x, c.top() + 17.0),
                Align2::LEFT_CENTER,
                *label,
                small(),
                TX3,
            );
            let g = p.layout_no_wrap(value.clone(), mono_large(), *color);
            let vw = g.size().x;
            p.galley(Pos2::new(x, c.top() + 26.0), g, *color);
            if !unit.is_empty() {
                p.text(
                    Pos2::new(x + vw + 5.0, c.top() + 41.0),
                    Align2::LEFT_CENTER,
                    unit,
                    mono(12.0),
                    TX2,
                );
            }
            let sub = crate::app::truncate_middle(sub, ((cell_w - 30.0) / 6.2) as usize);
            p.text(
                Pos2::new(x, c.top() + 62.0),
                Align2::LEFT_CENTER,
                sub,
                small(),
                TX3,
            );
        }
    }
}

fn cell_color(c: Cell) -> Option<Color32> {
    match c {
        Cell::Done => Some(P300),
        Cell::Resumed => Some(P500),
        Cell::InFlight => Some(AC),
        Cell::Gap => Some(BAD),
        Cell::Skipped => Some(P400),
        Cell::Pending => None,
    }
}

/// One map position for all tracks: the least finished state wins, so a position only
/// shows as downloaded once every track has it.
fn merge(a: Cell, b: Cell) -> Cell {
    let rank = |c: Cell| match c {
        Cell::Gap => 0,
        Cell::Pending => 1,
        Cell::InFlight => 2,
        Cell::Skipped => 3,
        Cell::Resumed => 4,
        Cell::Done => 5,
    };
    if rank(a) <= rank(b) {
        a
    } else {
        b
    }
}

fn merged_cells(st: &JobState) -> Vec<Cell> {
    let len = st.tracks.iter().map(|t| t.cells.len()).max().unwrap_or(0);
    (0..len)
        .map(|i| {
            st.tracks
                .iter()
                .filter_map(|t| t.cells.get(i).copied())
                .reduce(merge)
                .unwrap_or(Cell::Pending)
        })
        .collect()
}

/// Paint legend entries (swatch, label, count) right-aligned, ending at `right`.
fn paint_legend(ui: &Ui, right: f32, y: f32, items: &[(Color32, bool, &str, usize)]) {
    let p = ui.painter();
    let layout: Vec<_> = items
        .iter()
        .map(|(color, outline, label, n)| {
            let l = p.layout_no_wrap(label.to_string(), small(), TX2);
            let c = p.layout_no_wrap(format::count_mono(*n), mono(11.5), TX1);
            (*color, *outline, l, c)
        })
        .collect();
    let total: f32 = layout
        .iter()
        .map(|(_, _, l, c)| 10.0 + 5.0 + l.size().x + 5.0 + c.size().x + 16.0)
        .sum();
    let mut x = right - total + 16.0;
    for (color, outline, l, c) in layout {
        let sw = Rect::from_center_size(Pos2::new(x + 5.0, y), Vec2::splat(10.0));
        if outline {
            p.rect_stroke(sw, 2, Stroke::new(1.0, color), StrokeKind::Inside);
        } else {
            p.rect_filled(sw, 2, color);
        }
        x += 15.0;
        let lw = l.size().x;
        p.galley(Pos2::new(x, y - l.size().y / 2.0), l, TX2);
        x += lw + 5.0;
        let cw = c.size().x;
        p.galley(Pos2::new(x, y - c.size().y / 2.0), c, TX1);
        x += cw + 16.0;
    }
}

fn segment_map(ui: &mut Ui, st: &JobState) {
    let cells = merged_cells(st);
    let mut counts = [0usize; 6];
    for c in &cells {
        counts[*c as usize] += 1;
    }
    let names = st
        .tracks
        .iter()
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join(" + ");
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let head = ui
            .horizontal(|ui| {
                ui.label(RichText::new("Segment map").font(card_title()).color(TX1));
                ui.label(
                    RichText::new(format!(
                        "{} positions · {names}",
                        format::count(cells.len())
                    ))
                    .font(small())
                    .color(TX3),
                );
            })
            .response
            .rect;
        let mut legend = vec![(P300, false, "Downloaded", counts[Cell::Done as usize])];
        if counts[Cell::Resumed as usize] > 0 {
            legend.push((P500, false, "Reused", counts[Cell::Resumed as usize]));
        }
        legend.push((AC, false, "In flight", counts[Cell::InFlight as usize]));
        legend.push((P400, true, "Pending", counts[Cell::Pending as usize]));
        legend.push((BAD, false, "Missing", counts[Cell::Gap as usize]));
        paint_legend(ui, ui.max_rect().right(), head.center().y, &legend);
        ui.add_space(8.0);

        let width = ui.available_width();
        let label_w = 58.0;
        let map_w = width - label_w;
        // Cells are at least 3.5 px wide; a row holds up to 360 (12 min of 2 s segments).
        let per_row = ((map_w / 3.5) as usize).clamp(24, 360);
        let cell_w = map_w / per_row as f32;
        let rows = cells.len().div_ceil(per_row).max(1);
        let height = rows as f32 * 13.0 + 6.0;
        let (area, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let well = Rect::from_min_max(Pos2::new(area.left() + label_w, area.top()), area.max);
        let p = ui.painter();
        p.rect_filled(well.expand(3.0), 6, P850);
        for row in 0..rows {
            let y = well.top() + 3.0 + row as f32 * 13.0;
            let start = row * per_row;
            let end = (start + per_row).min(cells.len());
            p.text(
                Pos2::new(area.left(), y + 5.0),
                Align2::LEFT_CENTER,
                format!("#{}", start + 1),
                mono_small(),
                TX3,
            );
            // Runs of the same state are painted as one rect each.
            let mut i = start;
            while i < end {
                let state = cells[i];
                let mut j = i + 1;
                while j < end && cells[j] == state {
                    j += 1;
                }
                let r = Rect::from_min_max(
                    Pos2::new(well.left() + (i - start) as f32 * cell_w, y),
                    Pos2::new(well.left() + (j - start) as f32 * cell_w - 0.5, y + 10.0),
                );
                match cell_color(state) {
                    Some(c) => {
                        p.rect_filled(r, 1, c);
                    }
                    None => {
                        p.rect_stroke(r.shrink(0.5), 1, Stroke::new(1.0, P400), StrokeKind::Inside);
                    }
                }
                i = j;
            }
        }
        // Hover: which segment is under the pointer.
        if let Some(pos) = resp.hover_pos().filter(|p| well.contains(*p)) {
            let col = ((pos.x - well.left()) / cell_w) as usize;
            let row = ((pos.y - well.top() - 3.0) / 13.0).max(0.0) as usize;
            let idx = row * per_row + col;
            if col < per_row {
                if let Some(c) = cells.get(idx) {
                    let r = Rect::from_min_size(
                        Pos2::new(
                            well.left() + col as f32 * cell_w,
                            well.top() + 3.0 + row as f32 * 13.0,
                        ),
                        Vec2::new(cell_w.max(3.0), 10.0),
                    );
                    ui.painter().rect_stroke(
                        r.expand(1.0),
                        1,
                        Stroke::new(1.5, TX1),
                        StrokeKind::Outside,
                    );
                    let what = match c {
                        Cell::Done => "downloaded",
                        Cell::Resumed => "reused from an earlier run",
                        Cell::InFlight => "downloading",
                        Cell::Gap => "missing (could not be fetched)",
                        Cell::Skipped => "not on the server (end of period)",
                        Cell::Pending => "pending",
                    };
                    resp.on_hover_text(format!("Segment {}: {what}", idx + 1));
                }
            }
        }
    });
}

fn throughput(ui: &mut Ui, st: &JobState, now: Instant) {
    // A rolling mean over five buckets (10 s), like the rate figure: live segments arrive
    // every few seconds, and raw 2 s buckets would alternate between zero and double.
    let raw = st.rate.series(now, BUCKETS + 4);
    let series: Vec<f64> = (4..raw.len())
        .map(|i| raw[i - 4..=i].iter().sum::<f64>() / 5.0 * 8.0)
        .collect();
    let current = st.rate.current(now) * 8.0;
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Throughput").font(card_title()).color(TX1));
            ui.label(RichText::new("last 5 minutes").font(small()).color(TX3));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format::bitrate(current))
                        .font(mono(13.0))
                        .color(TX1),
                );
            });
        });
        ui.add_space(8.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 118.0), Sense::hover());
        let plot = Rect::from_min_max(
            Pos2::new(rect.left() + 34.0, rect.top() + 6.0),
            Pos2::new(rect.right() - 4.0, rect.bottom() - 20.0),
        );
        let max = series.iter().copied().fold(current, f64::max).max(1e6);
        // Round the scale up to a readable step.
        let step = nice_step(max / 2.0);
        let top = (max / step).ceil() * step;
        let p = ui.painter();
        for k in 0..=2 {
            let v = top * k as f64 / 2.0;
            let y = plot.bottom() - (v / top) as f32 * plot.height();
            p.hline(plot.x_range(), y, Stroke::new(1.0, LINE));
            p.text(
                Pos2::new(rect.left(), y),
                Align2::LEFT_CENTER,
                short_bits(v),
                mono_small(),
                TX3,
            );
        }
        for (k, label) in ["-5 min", "-4", "-3", "-2", "-1", "now"].iter().enumerate() {
            let x = plot.left() + plot.width() * k as f32 / 5.0;
            let align = match k {
                0 => Align2::LEFT_CENTER,
                5 => Align2::RIGHT_CENTER,
                _ => Align2::CENTER_CENTER,
            };
            p.text(
                Pos2::new(x, rect.bottom() - 8.0),
                align,
                *label,
                mono_small(),
                TX3,
            );
        }
        let n = series.len().max(2);
        let points: Vec<Pos2> = series
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Pos2::new(
                    plot.left() + plot.width() * i as f32 / (n - 1) as f32,
                    plot.bottom() - (v / top) as f32 * plot.height(),
                )
            })
            .collect();
        if points.iter().any(|p| p.y < plot.bottom() - 0.5) {
            // Area fill as vertical strips between consecutive points.
            let mut mesh = egui::Mesh::default();
            for w in points.windows(2) {
                let base = mesh.vertices.len() as u32;
                for (pt, bottom) in [(w[0], false), (w[1], false), (w[1], true), (w[0], true)] {
                    let pos = if bottom {
                        Pos2::new(pt.x, plot.bottom())
                    } else {
                        pt
                    };
                    mesh.colored_vertex(pos, SPARK_FILL);
                }
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
            p.add(Shape::mesh(mesh));
            p.add(Shape::line(points.clone(), Stroke::new(1.5, P300)));
            if let Some(last) = points.last() {
                p.circle_filled(*last, 3.0, if st.live { AC } else { P300 });
            }
        } else {
            p.text(
                plot.center(),
                Align2::CENTER_CENTER,
                if st.phase.is_running() {
                    "waiting for data"
                } else {
                    "no data in the last 5 minutes"
                },
                small(),
                TX3,
            );
        }
    });
}

fn nice_step(x: f64) -> f64 {
    let mag = 10f64.powf(x.max(1.0).log10().floor());
    let f = x / mag;
    let nice = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

fn short_bits(bits: f64) -> String {
    if bits >= 1e9 {
        format!("{:.0}G", bits / 1e9)
    } else if bits >= 1e6 {
        format!("{:.0}M", bits / 1e6)
    } else if bits >= 1e3 {
        format!("{:.0}k", bits / 1e3)
    } else {
        "0".into()
    }
}

/// Returns the log as text when Copy was pressed.
fn event_log(ui: &mut Ui, st: &JobState) -> Option<String> {
    let mut copy = None;
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Event log").font(card_title()).color(TX1));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(Button::ghost("Copy").icon(ph::COPY).size(Size::Small))
                    .clicked()
                {
                    copy = Some(
                        st.log
                            .iter()
                            .map(|l| format!("{}  {}", l.at.format("%H:%M:%S"), l.text))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("event-log")
            .max_height(220.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for line in st.log.iter().rev() {
                    let (r, resp) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 25.0), Sense::hover());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(r, 4, P750);
                    }
                    p.text(
                        Pos2::new(r.left() + 4.0, r.center().y),
                        Align2::LEFT_CENTER,
                        line.at.format("%H:%M:%S").to_string(),
                        mono_small(),
                        TX3,
                    );
                    let (icon_str, color) = match line.level {
                        Level::Info => (ph::INFO, TX3),
                        Level::Warn => (ph::WARNING, WARN),
                        Level::Ok => (ph::CHECK_CIRCLE, OK),
                    };
                    p.text(
                        Pos2::new(r.left() + 84.0, r.center().y),
                        Align2::CENTER_CENTER,
                        icon_str,
                        icon(14.0),
                        color,
                    );
                    let max_chars = ((r.width() - 110.0) / 6.8) as usize;
                    p.text(
                        Pos2::new(r.left() + 100.0, r.center().y),
                        Align2::LEFT_CENTER,
                        crate::app::truncate_middle(&line.text, max_chars.max(10)),
                        regular(12.5),
                        TX2,
                    );
                    if line.text.chars().count() > max_chars {
                        resp.on_hover_text(&line.text);
                    }
                }
                if st.log.is_empty() {
                    ui.label(RichText::new("Nothing yet.").font(small()).color(TX3));
                }
            });
    });
    copy
}

fn tracks(ui: &mut Ui, st: &JobState) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Tracks").font(card_title()).color(TX1));
        ui.add_space(4.0);
        if st.tracks.is_empty() {
            ui.label(
                RichText::new("Tracks appear once the manifest has been read.")
                    .font(small())
                    .color(TX3),
            );
        }
        for (i, t) in st.tracks.iter().enumerate() {
            let (r, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::hover());
            let p = ui.painter();
            if i > 0 {
                p.hline(r.x_range(), r.top(), Stroke::new(1.0, LINE));
            }
            let tile =
                Rect::from_min_size(Pos2::new(r.left(), r.center().y - 17.0), Vec2::splat(34.0));
            p.rect(tile, 8, P750, Stroke::new(1.0, LINE2), StrokeKind::Inside);
            p.text(
                tile.center(),
                Align2::CENTER_CENTER,
                if t.name == "audio" {
                    ph::SPEAKER_HIGH
                } else {
                    ph::FILM_STRIP
                },
                icon(17.0),
                TX2,
            );
            let mut name = t.name.clone();
            if let Some(first) = name.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            let x = tile.right() + 12.0;
            p.text(
                Pos2::new(x, r.center().y - 9.0),
                Align2::LEFT_CENTER,
                name,
                body_strong(),
                TX1,
            );
            // Right side: state badge over "n in flight"; bytes over segment count.
            let (badge, kind) = if t.live && st.phase.is_running() {
                ("Following", Badge::Accent)
            } else if !t.cells.is_empty() && t.settled() == t.cells.len() {
                ("Done", Badge::Ok)
            } else if st.phase.is_running() {
                ("Downloading", Badge::Outline)
            } else {
                ("Stopped", Badge::Soon)
            };
            let bsize = widgets::badge_size(ui, badge, kind);
            widgets::paint_badge(
                ui,
                Pos2::new(r.right() - bsize.x, r.center().y - 21.0),
                badge,
                kind,
            );
            let p = ui.painter();
            p.text(
                Pos2::new(r.right(), r.center().y + 12.0),
                Align2::RIGHT_CENTER,
                format!("{} in flight", t.in_flight),
                small(),
                TX3,
            );
            let col = r.right() - bsize.x.max(80.0) - 24.0;
            p.text(
                Pos2::new(col, r.center().y - 9.0),
                Align2::RIGHT_CENTER,
                format::bytes(t.bytes),
                mono(13.0),
                TX1,
            );
            p.text(
                Pos2::new(col, r.center().y + 10.0),
                Align2::RIGHT_CENTER,
                format!("{} segments", format::count(t.done)),
                small(),
                TX3,
            );
            let desc_w = col - 110.0 - x;
            let desc = crate::jobs::compact_track(&t.description);
            let desc = if desc.is_empty() {
                t.description.clone()
            } else {
                desc
            };
            let g = p.layout_no_wrap(
                crate::app::truncate_middle(&desc, ((desc_w / 7.0).max(8.0)) as usize),
                mono(11.5),
                TX2,
            );
            p.galley(Pos2::new(x, r.center().y + 10.0 - g.size().y / 2.0), g, TX2);
        }
    });
}

fn output_card(
    ui: &mut Ui,
    st: &JobState,
    spec: &crate::jobs::JobSpec,
    ffmpeg: Option<&crate::jobs::Ffmpeg>,
) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Output").font(card_title()).color(TX1));
        ui.add_space(6.0);
        let file = match &st.phase {
            Phase::Completed { files } => files.first().cloned(),
            _ => None,
        }
        .unwrap_or_else(|| spec.output.clone());
        let ext = spec
            .output
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4")
            .to_string();
        let on_stop = match ffmpeg {
            Some(f) if f.path.is_some() => format!(
                "Remux to {ext} with ffmpeg {}, stream copy",
                f.version.as_deref().unwrap_or("")
            ),
            Some(_) => "Keep raw stream files (ffmpeg not found)".into(),
            None => "Remux with ffmpeg, stream copy".into(),
        };
        let rows: Vec<(&str, String)> = vec![
            ("File", format::tilde(&file)),
            ("On stop", on_stop),
            (
                "Max duration",
                spec.max_duration
                    .map(|d| format::clock(d.as_secs_f64()))
                    .unwrap_or_else(|| {
                        if st.live {
                            "None, records until stopped".into()
                        } else {
                            "None".into()
                        }
                    }),
            ),
            (
                "Resume",
                format!(
                    "Crash-safe, parts in {}",
                    format::tilde(&crate::jobs::parts_dir(&spec.output))
                ),
            ),
        ];
        egui::Grid::new("output-grid")
            .num_columns(2)
            .spacing([18.0, 10.0])
            .show(ui, |ui| {
                for (label, value) in rows {
                    ui.label(RichText::new(label).font(small()).color(TX3));
                    if label == "File" {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(crate::app::truncate_middle(&value, 46))
                                    .font(mono(12.5))
                                    .color(TX1),
                            );
                            if widgets::icon_button(ui, ph::COPY, "Copy path", 26.0).clicked() {
                                ui.ctx().copy_text(file.display().to_string());
                            }
                        });
                    } else {
                        ui.label(
                            RichText::new(crate::app::truncate_middle(&value, 58))
                                .font(regular(12.5))
                                .color(TX2),
                        );
                    }
                    ui.end_row();
                }
            });
    });
}
