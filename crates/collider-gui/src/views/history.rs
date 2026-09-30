//! The History page: finished jobs from this and earlier sessions.

use egui::{Align, Align2, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use egui_phosphor::regular as ph;

use crate::app::{App, Page};
use crate::format;
use crate::history::Outcome;
use crate::theme::*;
use crate::views::downloads::menu_item;
use crate::widgets::{self, Badge, Button, Size};

enum Action {
    Open(std::path::PathBuf),
    Again(String),
    Copy(String),
    Remove(usize),
    Clear,
}

pub fn page(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new("History").font(heading()).color(TX1));
            let n = app.history.records.len();
            ui.label(
                RichText::new(format!(
                    "{n} finished job{}, newest first",
                    if n == 1 { "" } else { "s" }
                ))
                .font(regular(13.0))
                .color(TX3),
            );
        });
        if !app.history.records.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(Button::ghost("Clear history").icon(ph::TRASH))
                    .on_hover_text("Forget the list; downloaded files are not touched")
                    .clicked()
                {
                    actions.push(Action::Clear);
                }
            });
        }
    });
    ui.add_space(14.0);

    if app.history.records.is_empty() {
        widgets::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.add_space(22.0);
                ui.label(widgets::icon_text(ph::CLOCK_COUNTER_CLOCKWISE, 32.0, P300));
                ui.label(
                    RichText::new("Nothing finished yet")
                        .font(card_title())
                        .color(TX1),
                );
                ui.label(
                    RichText::new(
                        "Completed and failed downloads are listed here, across sessions.",
                    )
                    .font(regular(13.0))
                    .color(TX2),
                );
                ui.add_space(22.0);
            });
        });
        return;
    }

    widgets::card()
        .inner_margin(egui::Margin::symmetric(8, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (index, r) in app.history.records.iter().enumerate().rev() {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 58.0), Sense::hover());
                let p = ui.painter();
                if resp.contains_pointer() {
                    p.rect_filled(rect, 8, P750);
                }
                let (tile_icon, tile_color, tile_fill) = match r.outcome {
                    Outcome::Completed => (ph::CHECK_CIRCLE, OK, OK_SOFT),
                    Outcome::Failed => (ph::WARNING, TX2, P850),
                    Outcome::Unsupported => (ph::SHIELD_WARNING, TX2, P850),
                };
                let tile = Rect::from_min_size(
                    Pos2::new(rect.left() + 10.0, rect.center().y - 17.0),
                    Vec2::splat(34.0),
                );
                p.rect(
                    tile,
                    9,
                    tile_fill,
                    Stroke::new(1.0, LINE2),
                    StrokeKind::Inside,
                );
                p.text(
                    tile.center(),
                    Align2::CENTER_CENTER,
                    tile_icon,
                    icon(17.0),
                    tile_color,
                );
                let x = tile.right() + 14.0;
                p.text(
                    Pos2::new(x, rect.top() + 19.0),
                    Align2::LEFT_CENTER,
                    &r.name,
                    body_strong(),
                    TX1,
                );
                let when = chrono::DateTime::from_timestamp(r.finished, 0)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                let detail = match r.outcome {
                    Outcome::Completed => format!(
                        "{when}  ·  {}  ·  {}",
                        format::bytes(r.bytes),
                        r.files
                            .first()
                            .map(|f| format::tilde(f))
                            .unwrap_or_default()
                    ),
                    _ => format!("{when}  ·  {}", r.detail),
                };
                let max = ((rect.width() - 460.0) / 6.6).max(20.0) as usize;
                p.text(
                    Pos2::new(x, rect.top() + 39.0),
                    Align2::LEFT_CENTER,
                    crate::app::truncate_middle(&detail, max),
                    small(),
                    TX3,
                );
                let (badge, kind) = match r.outcome {
                    Outcome::Completed if r.live => ("Recorded", Badge::Outline),
                    Outcome::Completed => ("Completed", Badge::Ok),
                    Outcome::Failed => ("Failed", Badge::Warn),
                    Outcome::Unsupported => ("Not supported", Badge::Bad),
                };
                let bsize = widgets::badge_size(ui, badge, kind);
                widgets::paint_badge(
                    ui,
                    Pos2::new(rect.right() - 250.0 - bsize.x, rect.center().y - 10.0),
                    badge,
                    kind,
                );
                let actions_rect = Rect::from_min_max(
                    Pos2::new(rect.right() - 236.0, rect.top()),
                    Pos2::new(rect.right() - 8.0, rect.bottom()),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(actions_rect)
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let more = widgets::icon_button(ui, ph::DOTS_THREE, "More", 28.0);
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_min_width(190.0);
                            if menu_item(ui, ph::COPY, "Copy link") {
                                actions.push(Action::Copy(r.url.clone()));
                            }
                            if menu_item(ui, ph::X, "Remove from history") {
                                actions.push(Action::Remove(index));
                            }
                        });
                        if ui
                            .add(
                                Button::secondary("Again")
                                    .icon(ph::ARROW_CLOCKWISE)
                                    .size(Size::Small),
                            )
                            .on_hover_text("Probe this link again")
                            .clicked()
                        {
                            actions.push(Action::Again(r.url.clone()));
                        }
                        if let Some(f) = r.files.first() {
                            if ui
                                .add(
                                    Button::secondary("Open folder")
                                        .icon(ph::FOLDER_OPEN)
                                        .size(Size::Small),
                                )
                                .clicked()
                            {
                                actions.push(Action::Open(f.clone()));
                            }
                        }
                    },
                );
            }
        });

    for a in actions {
        match a {
            Action::Open(p) => app.open_folder(&p),
            Action::Again(url) => {
                app.url = url;
                app.start_probe(&ctx);
            }
            Action::Copy(url) => ctx.copy_text(url),
            Action::Remove(i) => app.history.remove(i),
            Action::Clear => app.history.clear(),
        }
    }
    if app.page != Page::History {
        ui.ctx().request_repaint();
    }
}
