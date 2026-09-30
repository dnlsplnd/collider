//! The Settings page. Every change is saved automatically.

use egui::{Align, Layout, RichText, Stroke, Ui, Vec2};
use egui_phosphor::regular as ph;

use crate::app::App;
use crate::format;
use crate::settings::{Container, Header, QualityPref, Settings};
use crate::theme::*;
use crate::widgets::{self, Button, Size};

const LABEL_W: f32 = 190.0;

pub fn page(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new("Settings").font(heading()).color(TX1));
            let path = app
                .settings_path
                .as_deref()
                .map(format::tilde)
                .unwrap_or_else(|| "nowhere (not persisted)".into());
            ui.label(
                RichText::new(format!("Saved automatically to {path}"))
                    .font(regular(13.0))
                    .color(TX3),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(Button::ghost("Reset to defaults").icon(ph::ARROW_COUNTER_CLOCKWISE))
                .on_hover_text("Headers are kept")
                .clicked()
            {
                let headers = std::mem::take(&mut app.settings.headers);
                app.settings = Settings {
                    headers,
                    ..Settings::default()
                };
            }
        });
    });
    ui.add_space(14.0);

    let narrow = ui.available_width() < 1040.0;
    let gap = 16.0;
    let full = ui.available_width();
    let left_w = if narrow { full } else { (full - gap) * 0.55 };
    let right_w = if narrow { full } else { full - gap - left_w };
    let mut detect = false;
    if narrow {
        ui.spacing_mut().item_spacing.y = gap;
        defaults(ui, &mut app.settings);
        detect = tools(ui, app.ffmpeg_status());
        network(ui, &mut app.settings);
        appearance(ui, &mut app.settings);
    } else {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(left_w);
                    ui.spacing_mut().item_spacing.y = gap;
                    defaults(ui, &mut app.settings);
                    detect = tools(ui, app.ffmpeg_status());
                },
            );
            ui.add_space(gap - ui.spacing().item_spacing.x);
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(right_w);
                    ui.spacing_mut().item_spacing.y = gap;
                    network(ui, &mut app.settings);
                    appearance(ui, &mut app.settings);
                },
            );
        });
    }
    if detect {
        *app.ffmpeg.lock().unwrap_or_else(|e| e.into_inner()) = None;
        app.detect_ffmpeg(&ctx);
    }
}

/// A settings row: label (+ help) on the left, controls on the right, a rule below.
fn row(ui: &mut Ui, label: &str, help: &str, last: bool, controls: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        ui.allocate_ui_with_layout(
            Vec2::new(LABEL_W, 40.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(LABEL_W);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(label).font(body_strong()).color(TX1));
                if !help.is_empty() {
                    ui.label(RichText::new(help).font(small()).color(TX3));
                }
            },
        );
        controls(ui);
    });
    if !last {
        let r = ui.cursor();
        ui.painter()
            .hline(r.x_range(), r.top() + 2.0, Stroke::new(1.0, LINE));
        ui.add_space(6.0);
    }
}

fn card_title_row(ui: &mut Ui, icon_str: &str, title: &str, subtitle: &str) {
    widgets::card_header(
        ui,
        icon_str,
        title,
        (!subtitle.is_empty()).then_some(subtitle),
        |_| {},
    );
    ui.add_space(4.0);
}

fn defaults(ui: &mut Ui, s: &mut Settings) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_title_row(
            ui,
            ph::SLIDERS_HORIZONTAL,
            "Defaults",
            "Applied to every new download",
        );
        row(ui, "Output folder", "", false, |ui| {
            let mut text = format::tilde(&s.output_dir);
            let w = ui.available_width() - 104.0;
            if widgets::text_field(ui, &mut text, "", w, true).changed() {
                s.output_dir = expand(&text);
            }
            if ui
                .add(Button::secondary("Browse…").min_width(96.0))
                .clicked()
            {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_directory(&s.output_dir)
                    .pick_folder()
                {
                    s.output_dir = dir;
                }
            }
        });
        row(ui, "Quality", "Can be changed per download", false, |ui| {
            let mut mode = match s.quality {
                QualityPref::Best => 0,
                QualityPref::Worst => 1,
                QualityPref::MaxHeight(_) => 2,
            };
            let mut h = match s.quality {
                QualityPref::MaxHeight(h) => h,
                _ => 1080,
            };
            if widgets::segmented(
                ui,
                "default-quality",
                &mut mode,
                &[(0, "Best"), (1, "Worst"), (2, "Max height")],
            ) {
                s.quality = match mode {
                    0 => QualityPref::Best,
                    1 => QualityPref::Worst,
                    _ => QualityPref::MaxHeight(h),
                };
            }
            ui.add_enabled_ui(mode == 2, |ui| {
                egui::ComboBox::from_id_salt("default-height")
                    .width(96.0)
                    .selected_text(RichText::new(format!("{h}p")).font(mono(12.5)))
                    .show_ui(ui, |ui| {
                        for option in [2160, 1440, 1080, 720, 480, 360, 240] {
                            if ui
                                .selectable_value(&mut h, option, format!("{option}p"))
                                .changed()
                            {
                                s.quality = QualityPref::MaxHeight(h);
                            }
                        }
                    });
            });
        });
        row(
            ui,
            "Audio language",
            "Else the stream's default",
            false,
            |ui| {
                widgets::text_field(ui, &mut s.audio_lang, "e.g. en, sv, English", 200.0, false);
            },
        );
        row(ui, "Container", "", false, |ui| {
            widgets::segmented(
                ui,
                "default-container",
                &mut s.container,
                &[(Container::Mp4, "mp4"), (Container::Mkv, "mkv")],
            );
            ui.label(
                RichText::new("Remuxed losslessly by ffmpeg")
                    .font(small())
                    .color(TX3),
            );
        });
        row(
            ui,
            "Parallel segments",
            "1 to 32 at once, per job",
            false,
            |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(80.0);
                ui.add(egui::Slider::new(&mut s.concurrency, 1..=32).show_value(false));
                ui.label(
                    RichText::new(s.concurrency.to_string())
                        .font(mono(13.0))
                        .color(TX1),
                );
            },
        );
        row(
            ui,
            "Retries",
            "Backoff on network errors, 429, 5xx",
            false,
            |ui| {
                stepper(ui, &mut s.retries, 0, 20);
            },
        );
        row(
            ui,
            "Stall timeout",
            "Give up when no data arrives",
            false,
            |ui| {
                number(ui, &mut s.timeout_secs, 5, 600, "seconds");
            },
        );
        row(
            ui,
            "Watch interval",
            "How often a watched link is checked",
            true,
            |ui| {
                number(ui, &mut s.watch_interval_secs, 5, 3600, "seconds");
            },
        );
    });
}

fn stepper(ui: &mut Ui, value: &mut u32, min: u32, max: u32) {
    ui.spacing_mut().item_spacing.x = 4.0;
    if ui
        .add(
            Button::secondary("")
                .icon(ph::MINUS)
                .a11y("Fewer retries")
                .enabled(*value > min),
        )
        .clicked()
    {
        *value -= 1;
    }
    ui.add_sized(
        [44.0, 32.0],
        egui::Label::new(RichText::new(value.to_string()).font(mono(13.0)).color(TX1)),
    );
    if ui
        .add(
            Button::secondary("")
                .icon(ph::PLUS)
                .a11y("More retries")
                .enabled(*value < max),
        )
        .clicked()
    {
        *value += 1;
    }
}

fn number(ui: &mut Ui, value: &mut u64, min: u64, max: u64, unit: &str) {
    ui.add(egui::DragValue::new(value).range(min..=max).speed(1.0))
        .on_hover_text(format!("{min} to {max}"));
    ui.label(RichText::new(unit).font(small()).color(TX3));
}

/// Returns true when Detect was pressed.
fn tools(ui: &mut Ui, ffmpeg: Option<crate::jobs::Ffmpeg>) -> bool {
    let mut detect = false;
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_title_row(ui, ph::TERMINAL_WINDOW, "Tools", "");
        row(ui, "ffmpeg", "The first ffmpeg on PATH", true, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let path = match &ffmpeg {
                        Some(f) => f
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "not found".into()),
                        None => "looking…".into(),
                    };
                    let w = ui.available_width() - 104.0;
                    let mut text = path;
                    ui.add_enabled_ui(false, |ui| {
                        widgets::text_field(ui, &mut text, "", w, true);
                    });
                    if ui
                        .add(
                            Button::secondary("Detect")
                                .icon(ph::ARROWS_CLOCKWISE)
                                .min_width(96.0),
                        )
                        .clicked()
                    {
                        detect = true;
                    }
                });
                let (fill, color, icon_str, text) = match &ffmpeg {
                    Some(crate::jobs::Ffmpeg {
                        path: Some(_),
                        version,
                    }) => (
                        OK_SOFT,
                        OK,
                        ph::CHECK_CIRCLE,
                        format!(
                            "ffmpeg {} found: downloads are remuxed into one mp4 or mkv file \
                             (stream copy, no re-encoding).",
                            version.as_deref().unwrap_or("")
                        ),
                    ),
                    Some(_) => (
                        WARN_SOFT,
                        WARN,
                        ph::WARNING,
                        "ffmpeg not found: downloads are saved as raw .ts or .mp4 stream \
                         files. Install ffmpeg to get one playable file per download."
                            .into(),
                    ),
                    None => (P750, TX2, ph::INFO, "Looking for ffmpeg…".into()),
                };
                egui::Frame::new()
                    .fill(fill)
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(widgets::icon_text(icon_str, 15.0, color));
                            ui.label(RichText::new(text).font(small()).color(TX1));
                        });
                    });
            });
        });
    });
    detect
}

fn network(ui: &mut Ui, s: &mut Settings) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_title_row(ui, ph::GLOBE, "Network", "");
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Custom HTTP headers")
                    .font(body_strong())
                    .color(TX1),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(Button::ghost("Add header").icon(ph::PLUS).size(Size::Small))
                    .clicked()
                {
                    s.headers.push(Header {
                        enabled: true,
                        name: String::new(),
                        value: String::new(),
                    });
                }
            });
        });
        ui.label(
            RichText::new(
                "Sent with every request: manifests, segments and keys. Use them for a \
                 Referer, a Cookie or an Authorization header the site requires.",
            )
            .font(small())
            .color(TX3),
        );
        ui.add_space(6.0);
        if s.headers.is_empty() {
            ui.label(RichText::new("No headers.").font(small()).color(TX2));
        }
        let mut remove = None;
        for (i, h) in s.headers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                widgets::checkbox(ui, &mut h.enabled, "Send this header");
                let w = ui.available_width() - 40.0;
                ui.push_id(("header", i), |ui| {
                    ui.horizontal(|ui| {
                        widgets::text_field(ui, &mut h.name, "Name", w * 0.36, true);
                        widgets::text_field(ui, &mut h.value, "Value", w * 0.64 - 8.0, true);
                    });
                });
                if widgets::icon_button(ui, ph::TRASH, "Remove header", 30.0).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            s.headers.remove(i);
        }
    });
}

fn appearance(ui: &mut Ui, s: &mut Settings) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_title_row(ui, ph::PALETTE, "Appearance", "");
        row(ui, "Theme", "", false, |ui| {
            widgets::badge_ui(ui, "Prussian Dark", widgets::Badge::Accent);
        });
        row(ui, "UI scale", "Also Ctrl + and Ctrl −", true, |ui| {
            let mut scale = (s.ui_scale * 100.0).round() as u32;
            egui::ComboBox::from_id_salt("ui-scale")
                .width(96.0)
                .selected_text(RichText::new(format!("{scale} %")).font(mono(12.5)))
                .show_ui(ui, |ui| {
                    for option in [90, 100, 110, 125, 150, 175] {
                        ui.selectable_value(&mut scale, option, format!("{option} %"));
                    }
                });
            s.ui_scale = scale as f32 / 100.0;
        });
    });
}

fn expand(text: &str) -> std::path::PathBuf {
    match (text.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => std::path::PathBuf::from(text),
    }
}
