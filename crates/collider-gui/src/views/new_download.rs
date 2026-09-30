//! The New download page: what a probe found, and the choices for the download.

use std::path::PathBuf;
use std::time::Duration;

use collider_core::model::{AudioInfo, VariantInfo};
use collider_core::{Protocol, StreamInfo};
use egui::{Align, Layout, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use egui_phosphor::regular as ph;

use crate::app::{App, Page};
use crate::format;
use crate::jobs::JobSpec;
use crate::jobs::ProbeResult;
use crate::settings::{Container, QualityPref, Settings};
use crate::theme::*;
use crate::widgets::{self, Badge, Button, Size};

pub struct Draft {
    pub url: String,
    pub result: Option<Result<(StreamInfo, Duration), String>>,
    pub quality: QualityPref,
    /// Chosen audio rendition (index into `StreamInfo::audio`).
    pub audio: Option<usize>,
    pub folder: PathBuf,
    pub stem: String,
    pub container: Container,
    pub concurrency: usize,
    pub max_duration: String,
    pub wait: bool,
}

impl Draft {
    pub fn probing(url: String, s: &Settings) -> Self {
        Self {
            stem: format::stream_name(&url),
            url,
            result: None,
            quality: s.quality,
            audio: None,
            folder: s.output_dir.clone(),
            container: s.container,
            concurrency: s.concurrency,
            max_duration: String::new(),
            wait: false,
        }
    }

    pub fn set_result(&mut self, result: ProbeResult, s: &Settings) {
        if let Ok((info, _)) = &result {
            self.audio = default_audio(info, self.variant(info), &s.audio_lang);
            if self.quality != QualityPref::Best && self.variant(info).is_none() {
                self.quality = QualityPref::Best;
            }
        }
        self.result = Some(result);
    }

    fn variant<'a>(&self, info: &'a StreamInfo) -> Option<&'a VariantInfo> {
        pick_variant(&info.variants, self.quality)
    }
}

/// An audio-only variant, as the engine ranks them (never picked as video).
fn audio_only(v: &VariantInfo) -> bool {
    const VIDEO: [&str; 14] = [
        "avc1", "avc3", "hvc1", "hev1", "av01", "vp08", "vp09", "vp8", "vp9", "dvh1", "dvhe",
        "dva1", "dvav", "mp4v",
    ];
    v.resolution.is_none()
        && v.codecs.as_deref().is_some_and(|c| {
            !c.split(',').any(|x| {
                VIDEO
                    .iter()
                    .any(|p| x.trim().to_ascii_lowercase().starts_with(p))
            })
        })
}

fn height(v: &VariantInfo) -> u64 {
    v.resolution.map_or(0, |r| r.1)
}

/// The variant the engine will choose for `q` (same ranking as `hls::select_variant`).
pub fn pick_variant(variants: &[VariantInfo], q: QualityPref) -> Option<&VariantInfo> {
    let has_video = variants.iter().any(|v| !audio_only(v));
    let candidates = variants.iter().filter(|v| !has_video || !audio_only(v));
    let rank = |v: &&VariantInfo| (height(v), v.bandwidth);
    match q {
        QualityPref::Best => candidates.max_by_key(rank),
        QualityPref::Worst => candidates.min_by_key(rank),
        QualityPref::MaxHeight(h) => candidates.filter(|v| height(v) <= h).max_by_key(rank),
    }
}

/// Audio renditions that go with `variant` (HLS groups them; DASH lists all).
fn audio_for<'a>(
    info: &'a StreamInfo,
    variant: Option<&VariantInfo>,
) -> Vec<(usize, &'a AudioInfo)> {
    let group = variant.and_then(|v| v.audio_group.as_deref());
    info.audio
        .iter()
        .enumerate()
        .filter(|(_, a)| group.is_none_or(|g| a.group == g))
        .collect()
}

/// The rendition the engine picks for `lang`: a language or name match, else the
/// default, else the first.
fn default_audio(info: &StreamInfo, variant: Option<&VariantInfo>, lang: &str) -> Option<usize> {
    let options = audio_for(info, variant);
    let lang = lang.trim();
    options
        .iter()
        .find(|(_, a)| {
            !lang.is_empty()
                && (a
                    .language
                    .as_deref()
                    .is_some_and(|l| l.eq_ignore_ascii_case(lang))
                    || a.name.eq_ignore_ascii_case(lang))
        })
        .or_else(|| options.iter().find(|(_, a)| a.default))
        .or_else(|| options.first())
        .map(|(i, _)| *i)
}

pub fn page(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(draft) = app.draft.as_mut() else {
        app.page = Page::Downloads;
        return;
    };
    if widgets::breadcrumb(
        ui,
        &format!("New download · {}", format::stream_name(&draft.url)),
    ) {
        app.page = Page::Downloads;
        return;
    }
    ui.add_space(10.0);
    let result = draft.result.clone();
    match result {
        None => probing(ui, &draft.url),
        Some(Err(message)) => {
            if let Some(start) = failed(ui, &draft.url, &message) {
                let url = draft.url.clone();
                let name = format::stream_name(&url);
                let mut spec = app.default_spec(&url, &name);
                if start {
                    spec.wait = Some(Duration::from_secs(app.settings.watch_interval_secs));
                    app.add_job(&ctx, name, spec);
                    app.page = Page::Watch;
                } else {
                    app.start_probe(&ctx);
                }
            }
        }
        Some(Ok((info, _))) => ready(app, ui, &info),
    }
}

fn probing(ui: &mut Ui, url: &str) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::spinner(ui, 18.0);
            ui.label(
                RichText::new("Reading the stream…")
                    .font(card_title())
                    .color(TX1),
            );
        });
        ui.label(RichText::new(url).font(mono(12.5)).color(TX3));
    });
}

/// Returns `Some(true)` to watch the stream, `Some(false)` to probe again.
fn failed(ui: &mut Ui, url: &str, message: &str) -> Option<bool> {
    let mut choice = None;
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(widgets::icon_text(ph::WARNING, 20.0, WARN));
            ui.label(
                RichText::new("Could not read this stream")
                    .font(title())
                    .color(TX1),
            );
        });
        ui.add_space(4.0);
        ui.label(RichText::new(message).font(mono(12.5)).color(TX2));
        ui.label(RichText::new(url).font(mono(12.0)).color(TX3));
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "If the broadcast has not started yet, collider can watch the link and start \
                 recording as soon as it goes live.",
            )
            .font(regular(13.0))
            .color(TX2),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add(Button::primary("Wait for it").icon(ph::BINOCULARS))
                .clicked()
            {
                choice = Some(true);
            }
            if ui
                .add(Button::secondary("Try again").icon(ph::ARROWS_CLOCKWISE))
                .clicked()
            {
                choice = Some(false);
            }
        });
    });
    choice
}

fn ready(app: &mut App, ui: &mut Ui, info: &StreamInfo) {
    let ctx = ui.ctx().clone();
    let narrow = ui.available_width() < 1040.0;
    let side_w = if narrow { 320.0 } else { 372.0 };
    let gap = 16.0;
    let main_w = ui.available_width() - side_w - gap;
    let mut start = false;

    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(main_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(main_w);
            ui.spacing_mut().item_spacing.y = 14.0;
            let draft = app.draft.as_mut().expect("page checked");
            summary(ui, draft, info, narrow);
            if !info.variants.is_empty() {
                variants(ui, draft, info, narrow);
            }
            if !info.audio.is_empty() {
                audio(ui, draft, info);
            }
            if info.drm {
                drm(ui);
            }
        });
        ui.add_space(gap - ui.spacing().item_spacing.x);
        ui.allocate_ui_with_layout(Vec2::new(side_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(side_w);
            start = output(app, ui, info);
        });
    });

    if start {
        let draft = app.draft.as_ref().expect("page checked");
        let stem = format::sanitize(draft.stem.trim());
        let stem = if stem.is_empty() {
            format::stream_name(&draft.url)
        } else {
            stem
        };
        let lang = draft
            .audio
            .and_then(|i| info.audio.get(i))
            .map(|a| a.language.clone().unwrap_or_else(|| a.name.clone()));
        let spec = JobSpec {
            url: draft.url.clone(),
            output: format::unique_output(&draft.folder, &stem, draft.container.ext()),
            quality: draft.quality.to_quality(),
            audio_lang: lang,
            concurrency: draft.concurrency,
            max_duration: format::parse_duration(&draft.max_duration).map(Duration::from_secs_f64),
            wait: draft
                .wait
                .then(|| Duration::from_secs(app.settings.watch_interval_secs)),
            retries: app.settings.retries,
            timeout: app.settings.timeout(),
            headers: app.settings.http_headers(),
        };
        app.add_job(&ctx, stem, spec);
        app.draft = None;
        app.url.clear();
        app.probed = None;
        app.page = Page::Downloads;
    }
}

fn summary(ui: &mut Ui, draft: &Draft, info: &StreamInfo, narrow: bool) {
    let videos = info.variants.iter().filter(|v| !audio_only(v)).count();
    let tracks = match (videos, info.audio.len()) {
        (0, 0) => "1 rendition".to_string(),
        (v, 0) => format!("{v} video"),
        (v, a) => format!("{v} video · {a} audio"),
    };
    let segments = info.segments.map(|n| match info.duration {
        Some(d) if !info.live && n > 0 => {
            format!("{} × {:.0} s", format::count_mono(n), d / n as f64)
        }
        _ => format::count_mono(n),
    });
    let duration = if info.live {
        "live".to_string()
    } else {
        info.duration
            .map(format::clock)
            .unwrap_or_else(|| "–".into())
    };
    let mut kpis = vec![("Duration", duration)];
    if let Some(s) = segments {
        kpis.push(("Segments", s));
    }
    kpis.push(("Tracks", tracks));

    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let full = ui.max_rect();
        // Measure the KPI block, so the name column knows how much room it has.
        let widths: Vec<f32> = kpis
            .iter()
            .map(|(l, v)| {
                let a = ui
                    .painter()
                    .layout_no_wrap(l.to_string(), small(), TX3)
                    .size()
                    .x;
                let b = ui
                    .painter()
                    .layout_no_wrap(v.clone(), mono(13.5), TX1)
                    .size()
                    .x;
                a.max(b)
            })
            .collect();
        let kpi_w: f32 = if narrow {
            0.0
        } else {
            widths.iter().sum::<f32>() + 28.0 * widths.len() as f32 + 1.0
        };
        ui.horizontal(|ui| {
            ui.set_min_height(48.0);
            let (tile, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            widgets::paint_tile(ui, tile, widgets::Tile::Download, ph::FILM_STRIP);
            ui.add_space(4.0);
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width() - kpi_w).max(160.0), 48.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format::stream_name(&draft.url))
                                .font(title())
                                .color(TX1),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(format::manifest_file(&draft.url))
                                    .font(regular(13.0))
                                    .color(TX3),
                            )
                            .truncate(),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        widgets::badge_ui(
                            ui,
                            match info.protocol {
                                Protocol::Hls => "HLS",
                                Protocol::Dash => "DASH",
                            },
                            Badge::Proto,
                        );
                        if info.live {
                            widgets::badge_ui(ui, "Live", Badge::Live);
                        } else if info.duration.is_some() {
                            widgets::badge_ui(ui, "VOD", Badge::Outline);
                        }
                        if info.drm {
                            widgets::badge_ui(ui, "DRM", Badge::Bad);
                        } else if info.encrypted {
                            widgets::badge_icon_ui(
                                ui,
                                Some(ph::LOCK),
                                "AES-128 · clear key",
                                Badge::Ok,
                            );
                        }
                    });
                },
            );
        });
        if !narrow {
            // KPIs, painted right-aligned with a divider before them.
            let p = ui.painter();
            let mut x = full.right() - kpi_w + 28.0;
            p.vline(
                x - 16.0,
                egui::Rangef::new(full.top() + 4.0, full.top() + 44.0),
                Stroke::new(1.0, LINE),
            );
            for ((label, value), w) in kpis.iter().zip(&widths) {
                p.text(
                    egui::pos2(x, full.top() + 12.0),
                    egui::Align2::LEFT_CENTER,
                    *label,
                    small(),
                    TX3,
                );
                p.text(
                    egui::pos2(x, full.top() + 34.0),
                    egui::Align2::LEFT_CENTER,
                    value,
                    mono(13.5),
                    TX1,
                );
                x += w + 28.0;
            }
        }
    });
}

fn variants(ui: &mut Ui, draft: &mut Draft, info: &StreamInfo, narrow: bool) {
    let chosen = pick_variant(&info.variants, draft.quality).map(|v| v.id.clone());
    let mut heights: Vec<u64> = info
        .variants
        .iter()
        .filter(|v| !audio_only(v))
        .map(height)
        .filter(|h| *h > 0)
        .collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    widgets::card()
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 12))
                .show(ui, |ui| {
                    widgets::card_header(ui, ph::FILM_STRIP, "Video variant", None, |ui| {
                        if !heights.is_empty() {
                            let mut h = match draft.quality {
                                QualityPref::MaxHeight(h) => h,
                                _ => heights[0],
                            };
                            let enabled = matches!(draft.quality, QualityPref::MaxHeight(_));
                            ui.add_enabled_ui(enabled, |ui| {
                                egui::ComboBox::from_id_salt("max-height")
                                    .width(92.0)
                                    .selected_text(RichText::new(format!("{h}p")).font(mono(12.5)))
                                    .show_ui(ui, |ui| {
                                        for option in &heights {
                                            ui.selectable_value(
                                                &mut h,
                                                *option,
                                                format!("{option}p"),
                                            );
                                        }
                                    });
                            });
                            if enabled {
                                draft.quality = QualityPref::MaxHeight(h);
                            }
                            let mut mode = match draft.quality {
                                QualityPref::Best => 0,
                                QualityPref::Worst => 1,
                                QualityPref::MaxHeight(_) => 2,
                            };
                            if widgets::segmented(
                                ui,
                                "draft-quality",
                                &mut mode,
                                &[(0, "Best"), (1, "Worst"), (2, "Max height")],
                            ) {
                                draft.quality = match mode {
                                    0 => QualityPref::Best,
                                    1 => QualityPref::Worst,
                                    _ => QualityPref::MaxHeight(h),
                                };
                            }
                            ui.label(RichText::new("Quality").font(medium(13.0)).color(TX2));
                        }
                    });
                });
            let mut columns: Vec<(&str, f32, bool)> = vec![
                ("", 36.0, false),
                ("ID", 70.0, false),
                ("Resolution", 176.0, false),
                ("Bitrate", 100.0, true),
                ("FPS", 60.0, true),
            ];
            if !narrow {
                columns.push(("Codecs", 0.0, false));
            }
            columns.push(("Est. size", 90.0, true));
            let width = ui.available_width() - 20.0;
            let fixed: f32 = columns.iter().map(|c| c.1).sum();
            let flex = (width - fixed).max(60.0);
            let widths: Vec<f32> = columns
                .iter()
                .map(|c| if c.1 == 0.0 { flex } else { c.1 })
                .collect();
            let row_x = ui.max_rect().left() + 10.0;
            // Header.
            let (hr, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
            let header = egui::Rect::from_min_max(
                egui::pos2(row_x, hr.top()),
                egui::pos2(row_x + width, hr.bottom()),
            );
            ui.painter().rect_filled(header, 6, P750);
            let mut x = row_x;
            for ((name, _, right), w) in columns.iter().zip(&widths) {
                let job = widgets::spaced(&name.to_uppercase(), label(), TX3, 0.9);
                let g = ui.painter().layout_job(job);
                let pos = if *right {
                    egui::pos2(
                        x + w - 12.0 - g.size().x,
                        header.center().y - g.size().y / 2.0,
                    )
                } else {
                    egui::pos2(x + 8.0, header.center().y - g.size().y / 2.0)
                };
                ui.painter().galley(pos, g, TX3);
                x += w;
            }
            let duration = info.duration.filter(|_| !info.live);
            for v in &info.variants {
                let (rr, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
                let is_chosen = chosen.as_deref() == Some(v.id.as_str());
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::RadioButton,
                        !audio_only(v),
                        is_chosen,
                        format!("Variant {}", v.id),
                    )
                });
                let row = egui::Rect::from_min_max(
                    egui::pos2(row_x, rr.top()),
                    egui::pos2(row_x + width, rr.bottom()),
                );
                let selected = chosen.as_deref() == Some(v.id.as_str());
                let only_audio = audio_only(v);
                let p = ui.painter();
                if selected {
                    p.rect_filled(row, 0, P650);
                    p.rect_filled(
                        egui::Rect::from_min_size(row.min, Vec2::new(3.0, row.height())),
                        0,
                        AC,
                    );
                } else if resp.hovered() && !only_audio {
                    p.rect_filled(row, 0, P700);
                }
                p.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, LINE));
                let text = if only_audio { TX_DIS } else { TX1 };
                let mut x = row_x;
                let cy = row.center().y;
                // Radio.
                let c = egui::pos2(x + 18.0, cy);
                p.circle_stroke(c, 7.5, Stroke::new(1.0, P400));
                if selected {
                    p.circle_filled(c, 4.0, P300);
                }
                x += widths[0];
                let cells: Vec<(String, bool)> = {
                    let mut cells = vec![
                        (v.id.clone(), false),
                        (
                            v.resolution
                                .map(|(w, h)| format!("{w}×{h}"))
                                .unwrap_or_else(|| "audio only".into()),
                            false,
                        ),
                        (format::bitrate(v.bandwidth as f64), true),
                        (
                            v.frame_rate
                                .map(|f| format!("{}", (f * 100.0).round() / 100.0))
                                .unwrap_or_else(|| "–".into()),
                            true,
                        ),
                    ];
                    if !narrow {
                        cells.push((v.codecs.clone().unwrap_or_else(|| "–".into()), false));
                    }
                    cells.push((
                        duration
                            .map(|d| format::bytes((v.bandwidth as f64 * d / 8.0) as u64))
                            .unwrap_or_else(|| "–".into()),
                        true,
                    ));
                    cells
                };
                for (i, ((value, right), w)) in cells.iter().zip(&widths[1..]).enumerate() {
                    let g = ui.painter().layout_no_wrap(
                        crate::app::truncate_middle(value, ((w - 16.0) / 7.6) as usize),
                        mono(12.5),
                        text,
                    );
                    let pos = if *right {
                        egui::pos2(x + w - 12.0 - g.size().x, cy - g.size().y / 2.0)
                    } else {
                        egui::pos2(x + 8.0, cy - g.size().y / 2.0)
                    };
                    let gw = g.size().x;
                    ui.painter().galley(pos, g, text);
                    if i == 1 && selected {
                        widgets::paint_badge(
                            ui,
                            egui::pos2(pos.x + gw + 10.0, cy - 10.0),
                            "Selected",
                            Badge::Accent,
                        );
                    }
                    x += w;
                }
                if resp.clicked() && !only_audio {
                    let h = height(v);
                    draft.quality = if h == 0 {
                        QualityPref::Best
                    } else {
                        QualityPref::MaxHeight(h)
                    };
                    draft.audio = default_audio(info, Some(v), "").or(draft.audio);
                }
                if !only_audio {
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            }
            ui.add_space(10.0);
        });
}

fn audio(ui: &mut Ui, draft: &mut Draft, info: &StreamInfo) {
    let variant = pick_variant(&info.variants, draft.quality);
    let options = audio_for(info, variant);
    if options.is_empty() {
        return;
    }
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_header(ui, ph::SPEAKER_HIGH, "Audio", None, |ui| {
            ui.label(
                RichText::new(format!("{} rendition(s)", options.len()))
                    .font(small())
                    .color(TX3),
            );
        });
        ui.add_space(6.0);
        for (i, a) in &options {
            let (r, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
            let selected = draft.audio == Some(*i);
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, &a.name)
            });
            let p = ui.painter();
            if selected {
                p.rect(r, 8, P650, Stroke::new(1.0, P400), StrokeKind::Inside);
            } else if resp.hovered() {
                p.rect_filled(r, 8, P700);
            }
            let c = egui::pos2(r.left() + 16.0, r.center().y);
            p.circle_stroke(c, 7.5, Stroke::new(1.0, P400));
            if selected {
                p.circle_filled(c, 4.0, P300);
            }
            let mut x = r.left() + 34.0;
            let g = p.layout_no_wrap(a.name.clone(), regular(13.5), TX1);
            let w = g.size().x;
            p.galley(egui::pos2(x, r.center().y - g.size().y / 2.0), g, TX1);
            x += w + 6.0;
            if let Some(l) = &a.language {
                let g = p.layout_no_wrap(l.clone(), regular(12.0), TX3);
                let w = g.size().x;
                p.galley(egui::pos2(x, r.center().y - g.size().y / 2.0 + 1.0), g, TX3);
                x += w + 8.0;
            }
            if a.default {
                widgets::paint_badge(
                    ui,
                    egui::pos2(x, r.center().y - 10.0),
                    "Default",
                    Badge::Outline,
                );
            }
            let right = [
                a.bandwidth.map(|b| format::bitrate(b as f64)),
                a.codecs.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("   ");
            if !right.is_empty() {
                ui.painter().text(
                    egui::pos2(r.right() - 12.0, r.center().y),
                    egui::Align2::RIGHT_CENTER,
                    right,
                    mono(12.5),
                    TX2,
                );
            }
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                draft.audio = Some(*i);
            }
        }
    });
}

fn drm(ui: &mut Ui) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (tile, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
            widgets::paint_tile(ui, tile, widgets::Tile::Neutral, ph::SHIELD_WARNING);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("This stream is DRM-protected")
                        .font(card_title())
                        .color(TX1),
                );
                ui.label(
                    RichText::new(
                        "collider only saves streams served without protection, so this one \
                         cannot be downloaded.",
                    )
                    .font(regular(13.0))
                    .color(TX2),
                );
            });
        });
    });
}

/// The output card; returns true when Start download was pressed.
fn output(app: &mut App, ui: &mut Ui, info: &StreamInfo) -> bool {
    let mut start = false;
    let ffmpeg = app.ffmpeg_status();
    let draft = app.draft.as_mut().expect("page checked");
    widgets::card()
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    widgets::card_header(ui, ph::SLIDERS_HORIZONTAL, "Output", None, |_| {});
                });
            let r = ui.min_rect();
            ui.painter()
                .hline(r.x_range(), ui.cursor().top(), Stroke::new(1.0, LINE));
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    field_label(ui, "Save to");
                    ui.horizontal(|ui| {
                        let mut path = format::tilde(&draft.folder);
                        let w = ui.available_width() - 100.0;
                        let resp = widgets::text_field(ui, &mut path, "", w, true);
                        if resp.changed() {
                            draft.folder = expand_tilde(&path);
                        }
                        if ui
                            .add(
                                Button::secondary("Change…")
                                    .size(Size::Medium)
                                    .min_width(90.0),
                            )
                            .clicked()
                        {
                            if let Some(dir) = rfd::FileDialog::new()
                                .set_directory(&draft.folder)
                                .pick_folder()
                            {
                                draft.folder = dir;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    field_label(ui, "File name");
                    ui.horizontal(|ui| {
                        let w = ui.available_width();
                        let resp = widgets::text_field(ui, &mut draft.stem, "name", w, false);
                        ui.painter().text(
                            egui::pos2(resp.rect.right() - 12.0, resp.rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            format!(".{}", draft.container.ext()),
                            mono(12.5),
                            TX3,
                        );
                    });
                    ui.add_space(4.0);
                    field_label(ui, "Container");
                    ui.horizontal(|ui| {
                        widgets::segmented(
                            ui,
                            "draft-container",
                            &mut draft.container,
                            &[(Container::Mp4, "mp4"), (Container::Mkv, "mkv")],
                        );
                        ui.label(
                            RichText::new(match &ffmpeg {
                                Some(f) if f.path.is_some() => "Lossless remux, no re-encode",
                                Some(_) => "ffmpeg missing: raw files are saved",
                                None => "",
                            })
                            .font(small())
                            .color(TX3),
                        );
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        field_label(ui, "Parallel segments");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(draft.concurrency.to_string())
                                    .font(mono(13.0))
                                    .color(TX1),
                            );
                        });
                    });
                    ui.spacing_mut().slider_width = ui.available_width();
                    ui.add(egui::Slider::new(&mut draft.concurrency, 1..=32).show_value(false));
                    if info.live || draft.wait {
                        ui.add_space(6.0);
                        live_options(ui, draft, info.live);
                    } else {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            widgets::toggle(ui, &mut draft.wait, "Wait for the stream to go live");
                            ui.label(
                                RichText::new("Wait for the stream to go live")
                                    .font(regular(13.0))
                                    .color(TX2),
                            );
                        });
                    }
                });
            // Footer: estimate and the start button.
            let top = ui.cursor().top();
            let full = ui.max_rect();
            let footer = egui::Rect::from_min_max(egui::pos2(full.left(), top), full.max);
            ui.painter().rect_filled(
                egui::Rect::from_min_max(footer.min, egui::pos2(footer.right(), top + 120.0)),
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: CARD_RADIUS,
                    se: CARD_RADIUS,
                },
                P750,
            );
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 16))
                .show(ui, |ui| {
                    let variant = pick_variant(&info.variants, draft.quality);
                    let est = info.duration.filter(|_| !info.live).and_then(|d| {
                        let bw = variant.map(|v| v.bandwidth).unwrap_or(0) as f64;
                        let audio = draft
                            .audio
                            .and_then(|i| info.audio.get(i))
                            .and_then(|a| a.bandwidth)
                            .filter(|_| info.protocol == Protocol::Dash)
                            .unwrap_or(0) as f64;
                        (bw > 0.0).then(|| ((bw + audio) * d / 8.0) as u64)
                    });
                    let value = match est {
                        Some(b) => format!("≈ {}", format::bytes(b)),
                        None if info.live => "until stopped".into(),
                        None => "–".into(),
                    };
                    let value_w = ui
                        .painter()
                        .layout_no_wrap(value.clone(), mono(18.0), TX1)
                        .size()
                        .x;
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new((ui.available_width() - value_w - 16.0).max(60.0), 36.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.label(
                                    RichText::new(if info.live {
                                        "Recording"
                                    } else {
                                        "Estimated size"
                                    })
                                    .font(small())
                                    .color(TX2),
                                );
                                let mut parts = Vec::new();
                                if let Some((w, h)) = variant.and_then(|v| v.resolution) {
                                    parts.push(format!("{w}×{h}"));
                                }
                                if let Some(a) = draft.audio.and_then(|i| info.audio.get(i)) {
                                    parts.push(a.name.clone());
                                }
                                if let Some(n) = info.segments.filter(|_| !info.live) {
                                    parts.push(format!("{} segments", format::count(n)));
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(parts.join(" · "))
                                            .font(regular(12.5))
                                            .color(TX1),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(value).font(mono(18.0)).color(TX1));
                        });
                    });
                    ui.add_space(10.0);
                    let label = if draft.wait {
                        "Watch and record"
                    } else if info.live {
                        "Start recording"
                    } else {
                        "Start download"
                    };
                    let w = ui.available_width();
                    let resp = ui.add(
                        Button::primary(label)
                            .icon(if draft.wait {
                                ph::BINOCULARS
                            } else {
                                ph::DOWNLOAD_SIMPLE
                            })
                            .size(Size::Large)
                            .min_width(w)
                            .enabled(!info.drm),
                    );
                    if resp.clicked() {
                        start = true;
                    }
                });
        });
    start
}

fn live_options(ui: &mut Ui, draft: &mut Draft, live: bool) {
    egui::Frame::new()
        .fill(P850)
        .stroke(Stroke::new(1.0, LINE2))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(widgets::icon_text(ph::BROADCAST, 14.0, TX2));
                ui.label(RichText::new("Live options").font(medium(13.0)).color(TX1));
                if !live {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new("not live yet").font(small()).color(TX3));
                    });
                }
            });
            ui.add_space(4.0);
            ui.columns(2, |cols| {
                cols[0].label(RichText::new("Max duration").font(small()).color(TX2));
                let w = cols[0].available_width();
                let resp =
                    widgets::text_field(&mut cols[0], &mut draft.max_duration, "hh:mm:ss", w, true);
                let bad = !draft.max_duration.trim().is_empty()
                    && format::parse_duration(&draft.max_duration).is_none();
                if bad {
                    cols[0].painter().rect_stroke(
                        resp.rect,
                        8,
                        Stroke::new(1.0, BAD),
                        StrokeKind::Inside,
                    );
                }
                cols[1].label(RichText::new("Wait for live").font(small()).color(TX2));
                cols[1].horizontal(|ui| {
                    ui.add_space(2.0);
                    widgets::toggle(ui, &mut draft.wait, "Wait for the stream to go live");
                    ui.label(RichText::new("check on a timer").font(small()).color(TX3));
                });
            });
        });
}

fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(medium(13.0)).color(TX2));
}

fn expand_tilde(text: &str) -> PathBuf {
    match (text.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if text == "~" => dirs::home_dir().unwrap_or_default(),
        _ => PathBuf::from(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str, res: Option<(u64, u64)>, bw: u64, codecs: Option<&str>) -> VariantInfo {
        VariantInfo {
            id: id.into(),
            bandwidth: bw,
            resolution: res,
            codecs: codecs.map(str::to_string),
            frame_rate: None,
            audio_group: None,
        }
    }

    /// The GUI must highlight the variant the engine will actually download.
    #[test]
    fn picks_like_the_engine() {
        let vs = [
            v("a", None, 64_000, Some("mp4a.40.2")),
            v(
                "lo",
                Some((640, 360)),
                800_000,
                Some("avc1.4d401e,mp4a.40.2"),
            ),
            v("hi", Some((1920, 1080)), 6_000_000, None),
            v("hi2", Some((1920, 1080)), 4_000_000, None),
        ];
        let id = |q| pick_variant(&vs, q).map(|v| v.id.as_str());
        assert_eq!(id(QualityPref::Best), Some("hi"));
        assert_eq!(id(QualityPref::Worst), Some("lo"));
        assert_eq!(id(QualityPref::MaxHeight(720)), Some("lo"));
        assert_eq!(id(QualityPref::MaxHeight(100)), None);
        let only_audio = [v("a", None, 64_000, Some("mp4a.40.2"))];
        assert_eq!(
            pick_variant(&only_audio, QualityPref::Best).map(|v| v.id.as_str()),
            Some("a")
        );
    }

    #[test]
    fn audio_defaults_follow_the_engine() {
        let a = |name: &str, lang: &str, default: bool| AudioInfo {
            id: name.into(),
            group: "aud".into(),
            name: name.into(),
            language: Some(lang.into()),
            default,
            bandwidth: None,
            codecs: None,
        };
        let info = StreamInfo {
            protocol: Protocol::Hls,
            live: false,
            variants: vec![],
            audio: vec![a("English", "en", false), a("Svenska", "sv", true)],
            duration: None,
            segments: None,
            encrypted: false,
            drm: false,
        };
        assert_eq!(default_audio(&info, None, ""), Some(1));
        assert_eq!(default_audio(&info, None, "EN"), Some(0));
        assert_eq!(default_audio(&info, None, "fi"), Some(1));
    }

    #[test]
    fn tilde_paths() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand_tilde("~/Videos"), home.join("Videos"));
        }
        assert_eq!(expand_tilde("/srv/v"), PathBuf::from("/srv/v"));
    }
}
