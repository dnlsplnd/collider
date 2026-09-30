//! Components from the design spec (section 5), painted directly so every state gets the
//! exact colours: buttons, badges, progress bars, tiles, segmented controls, toggles, tabs.

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Galley, Margin, Pos2, Rect, Response,
    Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
};
use std::sync::Arc;

use crate::theme::*;

fn galley(ui: &Ui, text: impl Into<String>, font: FontId) -> Arc<Galley> {
    ui.painter()
        .layout_no_wrap(text.into(), font, Color32::PLACEHOLDER)
}

/// Text with extra letter spacing (labels and badges are tracked out in the spec).
pub fn spaced(text: &str, font: FontId, color: Color32, spacing: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font,
            color,
            extra_letter_spacing: spacing,
            ..Default::default()
        },
    );
    job
}

/// Paint a keyboard-focus ring outside `rect`.
pub fn focus_ring(ui: &Ui, rect: Rect, radius: u8) {
    ui.painter().rect_stroke(
        rect.expand(2.0),
        radius + 2,
        Stroke::new(2.0, FOCUS),
        StrokeKind::Outside,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

pub struct Button<'a> {
    kind: Kind,
    size: Size,
    icon: Option<&'a str>,
    fill_icon: bool,
    label: &'a str,
    enabled: bool,
    min_width: f32,
    /// Accessible name for icon-only buttons.
    a11y: Option<&'a str>,
}

impl<'a> Button<'a> {
    pub fn new(kind: Kind, label: &'a str) -> Self {
        Self {
            kind,
            size: Size::Medium,
            icon: None,
            fill_icon: false,
            label,
            enabled: true,
            min_width: 0.0,
            a11y: None,
        }
    }
    /// The accessible name of an icon-only button.
    pub fn a11y(mut self, name: &'a str) -> Self {
        self.a11y = Some(name);
        self
    }
    pub fn primary(label: &'a str) -> Self {
        Self::new(Kind::Primary, label)
    }
    pub fn secondary(label: &'a str) -> Self {
        Self::new(Kind::Secondary, label)
    }
    pub fn ghost(label: &'a str) -> Self {
        Self::new(Kind::Ghost, label)
    }
    pub fn danger(label: &'a str) -> Self {
        Self::new(Kind::Danger, label)
    }
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }
    /// Use the filled icon variant (Stop, Pause).
    pub fn filled(mut self) -> Self {
        self.fill_icon = true;
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (height, pad, text_font, icon_size) = match self.size {
            Size::Small => (28.0, 10.0, strong(12.5), 14.0),
            Size::Medium => (32.0, 13.0, button(), 15.0),
            Size::Large => (40.0, 18.0, strong(14.0), 17.0),
        };
        let label = (!self.label.is_empty()).then(|| galley(ui, self.label, text_font));
        let icon = self.icon.map(|i| {
            let font = if self.fill_icon {
                icon_fill(icon_size)
            } else {
                icon(icon_size)
            };
            galley(ui, i, font)
        });
        let gap = if label.is_some() && icon.is_some() {
            7.0
        } else {
            0.0
        };
        let content = label.as_ref().map_or(0.0, |g| g.size().x)
            + icon.as_ref().map_or(0.0, |g| g.size().x)
            + gap;
        let width = if label.is_none() {
            height
        } else {
            content + 2.0 * pad
        }
        .max(self.min_width);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), sense);
        let name = self.a11y.unwrap_or(self.label);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, self.enabled, name)
        });
        if ui.is_rect_visible(rect) {
            let down = resp.is_pointer_button_down_on();
            let hover = resp.hovered();
            let none = Color32::TRANSPARENT;
            let (fill, stroke, text) = if !self.enabled {
                match self.kind {
                    Kind::Ghost => (none, none, TX_DIS),
                    _ => (P800, LINE2, TX_DIS),
                }
            } else {
                match self.kind {
                    Kind::Primary => (
                        if down {
                            AC_LO
                        } else if hover {
                            AC_HI
                        } else {
                            AC
                        },
                        AC_LO,
                        ON_AC,
                    ),
                    Kind::Secondary => (
                        if down {
                            P650
                        } else if hover {
                            P700
                        } else {
                            P750
                        },
                        P400,
                        TX1,
                    ),
                    Kind::Ghost => (
                        if down {
                            P650
                        } else if hover {
                            P700
                        } else {
                            none
                        },
                        none,
                        if hover { TX1 } else { TX2 },
                    ),
                    Kind::Danger => (
                        if down {
                            Color32::from_rgb(0x33, 0x40, 0x5A)
                        } else if hover {
                            BAD_SOFT
                        } else {
                            none
                        },
                        BAD_LINE,
                        BAD,
                    ),
                }
            };
            let painter = ui.painter();
            painter.rect(rect, 8, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
            let mut x = rect.center().x - content / 2.0;
            let cy = rect.center().y;
            if let Some(g) = icon {
                let w = g.size().x;
                painter.galley(Pos2::new(x, cy - g.size().y / 2.0), g, text);
                x += w + gap;
            }
            if let Some(g) = label {
                painter.galley(Pos2::new(x, cy - g.size().y / 2.0), g, text);
            }
            if resp.has_focus() {
                focus_ring(ui, rect, 8);
            }
        }
        if self.enabled {
            resp.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            resp
        }
    }
}

/// A square, borderless icon button (32 px, 28 in dense rows).
pub fn icon_button(ui: &mut Ui, icon_str: &str, tooltip: &str, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tooltip));
    if ui.is_rect_visible(rect) {
        let hover = resp.hovered();
        if resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, 8, P650);
        } else if hover {
            ui.painter().rect_filled(rect, 8, P700);
        }
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon_str,
            icon(size * 0.53),
            if hover { TX1 } else { TX2 },
        );
        if resp.has_focus() {
            focus_ring(ui, rect, 8);
        }
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    Proto,
    Outline,
    Live,
    Ok,
    Warn,
    Bad,
    Info,
    Accent,
    Soon,
}

fn badge_job(icon_str: Option<&str>, text: &str, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    if let Some(i) = icon_str {
        job.append(
            i,
            0.0,
            TextFormat {
                font_id: icon(11.5),
                color,
                valign: egui::Align::Center,
                ..Default::default()
            },
        );
    }
    job.append(
        &text.to_uppercase(),
        if icon_str.is_some() { 4.0 } else { 0.0 },
        TextFormat {
            font_id: badge(),
            color,
            extra_letter_spacing: 0.6,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job
}

pub fn badge_size(ui: &Ui, text: &str, kind: Badge) -> Vec2 {
    badge_icon_size(ui, None, text, kind)
}

pub fn badge_icon_size(ui: &Ui, icon_str: Option<&str>, text: &str, kind: Badge) -> Vec2 {
    let g = ui.painter().layout_job(badge_job(icon_str, text, TX1));
    let dot = if kind == Badge::Live { 11.0 } else { 0.0 };
    Vec2::new(g.size().x + 14.0 + dot, 20.0)
}

/// Paint a badge with its top-left corner at `pos`; returns its rect.
pub fn paint_badge(ui: &Ui, pos: Pos2, text: &str, kind: Badge) -> Rect {
    paint_badge_icon(ui, pos, None, text, kind)
}

pub fn paint_badge_icon(
    ui: &Ui,
    pos: Pos2,
    icon_str: Option<&str>,
    text: &str,
    kind: Badge,
) -> Rect {
    let (fill, stroke, color) = match kind {
        Badge::Proto => (P650, P500, TX1),
        Badge::Outline => (Color32::TRANSPARENT, P400, TX2),
        Badge::Live => (AC, AC_LO, ON_AC),
        Badge::Ok => (OK_SOFT, OK_SOFT, OK),
        Badge::Warn => (WARN_SOFT, WARN_SOFT, WARN),
        Badge::Bad => (BAD_SOFT, BAD_SOFT, BAD),
        Badge::Info => (INFO_SOFT, INFO_SOFT, INFO),
        Badge::Accent => (AC_SOFT, AC_SOFT, AC),
        Badge::Soon => (P750, P750, TX2),
    };
    let g = ui.painter().layout_job(badge_job(icon_str, text, color));
    let dot = if kind == Badge::Live { 11.0 } else { 0.0 };
    let rect = Rect::from_min_size(pos, Vec2::new(g.size().x + 14.0 + dot, 20.0));
    let p = ui.painter();
    p.rect(rect, 5, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    if kind == Badge::Live {
        p.circle_filled(Pos2::new(rect.left() + 10.0, rect.center().y), 3.0, ON_AC);
    }
    p.galley(
        Pos2::new(rect.left() + 7.0 + dot, rect.center().y - g.size().y / 2.0),
        g,
        color,
    );
    rect
}

/// A badge placed in the layout.
pub fn badge_ui(ui: &mut Ui, text: &str, kind: Badge) -> Response {
    badge_icon_ui(ui, None, text, kind)
}

pub fn badge_icon_ui(ui: &mut Ui, icon_str: Option<&str>, text: &str, kind: Badge) -> Response {
    let size = badge_icon_size(ui, icon_str, text, kind);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_badge_icon(ui, rect.min, icon_str, text, kind);
    }
    resp
}

/// A 6 px bar: `parts` are consecutive (fraction, colour) runs over a `P700` track.
pub fn paint_progress(ui: &Ui, rect: Rect, parts: &[(f32, Color32)]) {
    let p = ui.painter();
    let r = CornerRadius::same((rect.height() / 2.0) as u8);
    p.rect_filled(rect, r, P700);
    let mut x = rect.left();
    for (i, &(frac, color)) in parts.iter().enumerate() {
        let w = rect.width() * frac.clamp(0.0, 1.0);
        if w <= 0.0 {
            continue;
        }
        let seg = Rect::from_min_max(
            Pos2::new(x, rect.top()),
            Pos2::new((x + w).min(rect.right()), rect.bottom()),
        );
        // Only the outer ends are rounded, so the runs join seamlessly.
        let mut cr = r;
        if i > 0 {
            cr.nw = 0;
            cr.sw = 0;
        }
        if i + 1 < parts.len() && parts[i + 1].0 > 0.0 {
            cr.ne = 0;
            cr.se = 0;
        }
        p.rect_filled(seg, cr, color);
        x += w;
    }
}

/// The live strip: brass cells with darker separators and a pale in-flight tip.
pub fn paint_live_strip(ui: &Ui, rect: Rect, in_flight: bool) {
    let p = ui.painter();
    let r = CornerRadius::same((rect.height() / 2.0) as u8);
    p.rect_filled(rect, r, AC);
    let mut x = rect.left() + 5.0;
    while x < rect.right() - 4.0 {
        p.vline(x, rect.y_range().shrink(0.5), Stroke::new(1.0, LIVE_SEP));
        x += 5.0;
    }
    if in_flight {
        let tip = Rect::from_min_max(Pos2::new(rect.right() - 7.0, rect.top()), rect.max);
        let mut cr = r;
        cr.nw = 0;
        cr.sw = 0;
        p.rect_filled(tip, cr, P200);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Live,
    Download,
    Resume,
    Watch,
    Done,
    Neutral,
    Paused,
    Busy,
}

/// The 40 px state tile at the start of a job row.
pub fn paint_tile(ui: &Ui, rect: Rect, tile: Tile, icon_str: &str) {
    let (fill, stroke, color) = match tile {
        Tile::Live => (AC_SOFT, AC_LINE, AC),
        Tile::Download | Tile::Resume | Tile::Paused | Tile::Busy => (P750, LINE2, TX1),
        Tile::Watch => (INFO_SOFT, INFO_SOFT, INFO),
        Tile::Done => (OK_SOFT, OK_SOFT, OK),
        Tile::Neutral => (P850, LINE2, TX2),
    };
    let p = ui.painter();
    p.rect(rect, 10, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon_str,
        icon(20.0),
        color,
    );
}

/// Segmented control; returns true when the value changed. `id_salt` must be unique on
/// the page.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    id_salt: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let font = medium(13.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, l)| galley(ui, *l, font.clone()).size().x + 24.0)
        .collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, 32.0), Sense::hover());
    let mut changed = false;
    ui.painter()
        .rect(rect, 8, P850, Stroke::new(1.0, P400), StrokeKind::Inside);
    let mut x = rect.left() + 2.0;
    for (i, ((v, label), w)) in options.iter().zip(&widths).enumerate() {
        let item = Rect::from_min_size(Pos2::new(x, rect.top() + 2.0), Vec2::new(*w, 28.0));
        let resp = ui
            .interact(item, egui::Id::new((id_salt, i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let selected = *value == *v;
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, *label)
        });
        if selected {
            ui.painter()
                .rect(item, 6, P600, Stroke::new(1.0, P200), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(item, 6, P700);
        }
        ui.painter().text(
            item.center(),
            Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if selected || resp.hovered() { TX1 } else { TX2 },
        );
        if resp.clicked() && !selected {
            *value = *v;
            changed = true;
        }
        x += w;
    }
    changed
}

/// A 36 x 20 toggle switch named `label` for assistive technology; returns true when
/// flipped.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(36.0, 20.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, label));
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let (fill, stroke, knob) = if *on {
        (AC, AC_LO, ON_AC)
    } else {
        (P850, P400, TX2)
    };
    let p = ui.painter();
    p.rect(rect, 10, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
    p.circle_filled(Pos2::new(x, rect.center().y), 7.0, knob);
    if resp.has_focus() {
        focus_ring(ui, rect, 10);
    }
    if resp.clicked() {
        *on = !*on;
        true
    } else {
        false
    }
}

/// Filter tabs with counts; returns the clicked index.
pub fn tabs(ui: &mut Ui, labels: &[(&str, usize)], active: usize) -> Option<usize> {
    let label_font = medium(13.0);
    let count_font = mono(11.5);
    let items: Vec<(Arc<Galley>, Arc<Galley>)> = labels
        .iter()
        .map(|(l, n)| {
            (
                galley(ui, *l, label_font.clone()),
                galley(ui, n.to_string(), count_font.clone()),
            )
        })
        .collect();
    let widths: Vec<f32> = items
        .iter()
        .map(|(l, c)| l.size().x + c.size().x + 6.0 + 24.0)
        .collect();
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(widths.iter().sum::<f32>() + 6.0, 36.0),
        Sense::hover(),
    );
    ui.painter()
        .rect(rect, 9, P900, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let mut clicked = None;
    let mut x = rect.left() + 3.0;
    for (i, ((l, c), w)) in items.into_iter().zip(&widths).enumerate() {
        let item = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), Vec2::new(*w, 30.0));
        let resp = ui
            .interact(item, ui.id().with(("tab", i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let on = i == active;
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, labels[i].0)
        });
        if on {
            ui.painter()
                .rect(item, 7, P700, Stroke::new(1.0, P400), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(item, 7, P800);
        }
        let lw = l.size().x;
        let y = item.center().y;
        let x0 = item.left() + 12.0;
        ui.painter().galley(
            Pos2::new(x0, y - l.size().y / 2.0),
            l,
            if on { TX1 } else { TX2 },
        );
        ui.painter()
            .galley(Pos2::new(x0 + lw + 6.0, y - c.size().y / 2.0), c, TX3);
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    clicked
}

/// `ACTIVE  4 ──────────  [action]`; returns true when the action was clicked.
pub fn section_header(ui: &mut Ui, label: &str, count: usize, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(spaced(&label.to_uppercase(), self::label(), TX3, 0.9));
        ui.label(
            egui::RichText::new(count.to_string())
                .font(mono(11.5))
                .color(TX3),
        );
        let action_w = action.map_or(0.0, |a| galley(ui, a, medium(12.5)).size().x + 20.0);
        let rule_w = (ui.available_width() - action_w - 8.0).max(0.0);
        let (r, _) = ui.allocate_exact_size(Vec2::new(rule_w, 16.0), Sense::hover());
        ui.painter()
            .hline(r.x_range(), r.center().y, Stroke::new(1.0, LINE));
        if let Some(a) = action {
            let resp = ui.add(
                egui::Label::new(egui::RichText::new(a).font(medium(12.5)).color(TX2))
                    .sense(Sense::click()),
            );
            if resp.hovered() {
                ui.painter().hline(
                    resp.rect.x_range(),
                    resp.rect.bottom(),
                    Stroke::new(1.0, TX2),
                );
            }
            clicked = resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
        }
    });
    clicked
}

/// The card surface: `P800`, 1 px `LINE`, radius 12 and the card shadow.
pub fn card() -> Frame {
    Frame::new()
        .fill(P800)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CARD_RADIUS)
        .shadow(card_shadow())
        .inner_margin(Margin::symmetric(18, 16))
}

/// A card header: icon + title (+ muted subtitle), with content on the right.
pub fn card_header(
    ui: &mut Ui,
    icon_str: &str,
    title: &str,
    subtitle: Option<&str>,
    right: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.label(egui::RichText::new(icon_str).font(icon(17.0)).color(TX2));
        ui.label(egui::RichText::new(title).font(card_title()).color(TX1));
        if let Some(s) = subtitle {
            ui.label(egui::RichText::new(s).font(small()).color(TX3));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// A single-line text field in the spec's input style.
pub fn text_field(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    width: f32,
    mono_font: bool,
) -> Response {
    let font = if mono_font { mono(13.0) } else { body() };
    let enabled = ui.is_enabled();
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(egui::RichText::new(hint).color(TX3).font(font.clone()))
            .font(font)
            .text_color(if enabled { TX1 } else { TX2 })
            .desired_width(width - 22.0)
            .min_size(Vec2::new(width, 34.0))
            .vertical_align(egui::Align::Center)
            .frame(
                Frame::new()
                    .fill(if enabled { P850 } else { P800 })
                    .stroke(Stroke::new(1.0, if enabled { P400 } else { LINE2 }))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(10, 0)),
            ),
    );
    if resp.has_focus() {
        ui.painter()
            .rect_stroke(resp.rect, 8, Stroke::new(2.0, FOCUS), StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter()
            .rect_stroke(resp.rect, 8, Stroke::new(1.0, P300), StrokeKind::Inside);
    }
    resp
}

/// A 16 px checkbox (spec: square, `P400` border; checked `P300` with a dark check).
pub fn checkbox(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, label));
    let bx = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
    let p = ui.painter();
    if *on {
        p.rect_filled(bx, 4, P300);
        p.text(
            bx.center(),
            Align2::CENTER_CENTER,
            egui_phosphor::regular::CHECK,
            icon(12.0),
            P950,
        );
    } else {
        let border = if resp.hovered() { P300 } else { P400 };
        p.rect(bx, 4, P850, Stroke::new(1.0, border), StrokeKind::Inside);
    }
    if resp.has_focus() {
        focus_ring(ui, bx, 4);
    }
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// `← Downloads / current`; returns true when the way back was clicked.
pub fn breadcrumb(ui: &mut Ui, current: &str) -> bool {
    let mut back = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let resp = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(icon_text(egui_phosphor::regular::ARROW_LEFT, 14.0, TX2));
                ui.label(
                    egui::RichText::new("Downloads")
                        .font(medium(13.5))
                        .color(TX2),
                );
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.hovered() {
            ui.painter().hline(
                resp.rect.x_range(),
                resp.rect.bottom() + 1.0,
                Stroke::new(1.0, TX2),
            );
        }
        back = resp.clicked();
        ui.label(egui::RichText::new("/").font(regular(13.5)).color(TX3));
        ui.label(egui::RichText::new(current).font(medium(13.5)).color(TX1));
    });
    back
}

pub fn icon_text(icon_str: &str, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(icon_str).font(icon(size)).color(color)
}

pub fn spinner(ui: &mut Ui, size: f32) {
    ui.add(egui::Spinner::new().size(size).color(INFO));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text field is exactly as wide as asked (its frame margin included).
    #[test]
    fn text_field_width_is_exact() {
        let mut widths = Vec::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let mut text = String::from("hello");
            for w in [120.0, 200.0, 333.0] {
                let r = text_field(ui, &mut text, "hint", w, false);
                widths.push((w, r.rect.width(), r.rect.height()));
            }
        });
        harness.run();
        drop(harness);
        assert!(widths.len() >= 3, "nothing measured");
        for (want, got, h) in widths.iter().rev().take(3) {
            assert!((want - got).abs() < 0.5, "asked {want}, got {got}");
            assert!((h - 34.0).abs() < 0.5, "height {h}");
        }
    }
}
