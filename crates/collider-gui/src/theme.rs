//! The "Deep Prussian" design system: palette, fonts, text sizes and the egui style.
//! See `docs/design/deep-prussian/spec.md`.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke,
    Theme, Vec2,
};

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

// Surfaces: one hue, stepped in lightness.
pub const P950: Color32 = hex(0x001726);
pub const P900: Color32 = hex(0x001F33);
pub const P850: Color32 = hex(0x00263F);
pub const P800: Color32 = hex(0x003153);
pub const P750: Color32 = hex(0x073A60);
pub const P700: Color32 = hex(0x0E4570);
pub const P650: Color32 = hex(0x175181);
pub const P600: Color32 = hex(0x1F5E93);
pub const P500: Color32 = hex(0x2E6FA6);
pub const P400: Color32 = hex(0x5A90BF);
pub const P300: Color32 = hex(0x7FB0DA);
pub const P200: Color32 = hex(0xA6C6E2);
pub const LINE: Color32 = hex(0x0F4169);
pub const LINE2: Color32 = hex(0x164D78);
pub const JOB_HOVER: Color32 = hex(0x04375C);
pub const SPARK_FILL: Color32 = hex(0x0B4068);

// Text.
pub const TX1: Color32 = hex(0xEEF5FA);
pub const TX2: Color32 = hex(0xB4CADC);
pub const TX3: Color32 = hex(0x97B3CB);
pub const TX_DIS: Color32 = hex(0x6F8FAA);

// Brass: the primary action and the live-recording state only.
pub const AC: Color32 = hex(0xD9A441);
pub const AC_HI: Color32 = hex(0xE6B656);
pub const AC_LO: Color32 = hex(0xC4902F);
pub const ON_AC: Color32 = hex(0x00182A);
pub const AC_SOFT: Color32 = hex(0x234350);
pub const AC_LINE: Color32 = hex(0x62654B);
pub const LIVE_SEP: Color32 = hex(0xA07A30);

// Semantic colours, used as text or soft (pre-mixed) backgrounds.
pub const OK: Color32 = hex(0x4CC38A);
pub const OK_SOFT: Color32 = hex(0x0C485C);
pub const WARN: Color32 = hex(0xF0955A);
pub const WARN_SOFT: Color32 = hex(0x264154);
pub const BAD: Color32 = hex(0xF4887D);
pub const BAD_SOFT: Color32 = hex(0x273E57);
pub const BAD_LINE: Color32 = hex(0xB06C72);
pub const INFO: Color32 = hex(0x6CC6E6);
pub const INFO_SOFT: Color32 = hex(0x11496B);
pub const FOCUS: Color32 = hex(0x8CC4F2);

const INTER: &str = "inter";
const INTER_MEDIUM: &str = "inter-medium";
const INTER_SEMIBOLD: &str = "inter-semibold";
const MONO: &str = "jetbrains-mono";
const PHOSPHOR: &str = "phosphor";
const PHOSPHOR_FILL: &str = "phosphor-fill";
const ICONS: &str = "icons";
const ICONS_FILL: &str = "icons-fill";

/// Inter (three weights), JetBrains Mono and Phosphor icons.
///
/// Icons get families of their own with Phosphor first: Inter defines glyphs at some of the
/// private-use codepoints Phosphor uses, so an icon inside Inter text would render as a
/// letter. Icons are therefore always drawn with [`icon`] or [`icon_fill`], never mixed into
/// a text string.
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let data: [(&str, &'static [u8]); 6] = [
        (INTER, include_bytes!("../assets/fonts/Inter-Regular.ttf")),
        (
            INTER_MEDIUM,
            include_bytes!("../assets/fonts/Inter-Medium.ttf"),
        ),
        (
            INTER_SEMIBOLD,
            include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
        ),
        (
            MONO,
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
        ),
        (PHOSPHOR, egui_phosphor::Variant::Regular.font_bytes()),
        (PHOSPHOR_FILL, egui_phosphor::Variant::Fill.font_bytes()),
    ];
    for (name, bytes) in data {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // egui's bundled fonts stay as the last fallback (emoji and rare scripts).
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    let family = |keys: &[&str]| {
        let mut out: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        out.extend(fallback.iter().cloned());
        out
    };
    fonts
        .families
        .insert(FontFamily::Proportional, family(&[INTER]));
    fonts
        .families
        .insert(FontFamily::Monospace, family(&[MONO]));
    for name in [INTER_MEDIUM, INTER_SEMIBOLD] {
        fonts
            .families
            .insert(FontFamily::Name(name.into()), family(&[name]));
    }
    fonts
        .families
        .insert(FontFamily::Name(ICONS.into()), family(&[PHOSPHOR, INTER]));
    fonts.families.insert(
        FontFamily::Name(ICONS_FILL.into()),
        family(&[PHOSPHOR_FILL, INTER]),
    );
    fonts
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(INTER_MEDIUM.into()))
}

pub fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(INTER_SEMIBOLD.into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Phosphor icons (regular weight).
pub fn icon(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// Phosphor icons (fill weight).
pub fn icon_fill(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS_FILL.into()))
}

// The type scale (spec section 3).
pub fn heading() -> FontId {
    strong(22.0)
}
pub fn title() -> FontId {
    strong(18.0)
}
pub fn card_title() -> FontId {
    strong(14.5)
}
pub fn body() -> FontId {
    regular(13.5)
}
pub fn body_strong() -> FontId {
    strong(13.0)
}
pub fn button() -> FontId {
    strong(13.0)
}
pub fn small() -> FontId {
    regular(12.0)
}
pub fn label() -> FontId {
    strong(11.0)
}
pub fn badge() -> FontId {
    strong(10.5)
}
pub fn mono_body() -> FontId {
    mono(12.5)
}
pub fn mono_large() -> FontId {
    mono(19.0)
}
pub fn mono_small() -> FontId {
    mono(11.5)
}

pub const CARD_RADIUS: u8 = 12;

pub fn card_shadow() -> Shadow {
    Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_rgba_unmultiplied(0, 10, 20, 72),
    }
}

/// Install fonts and the Deep Prussian style (spec section 8). The app is dark only.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |st| {
        let v = &mut st.visuals;
        v.dark_mode = true;
        v.panel_fill = P850;
        v.window_fill = P800;
        v.extreme_bg_color = P850;
        v.faint_bg_color = P750;
        v.code_bg_color = P950;
        v.window_stroke = Stroke::new(1.0, LINE2);
        v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.menu_corner_radius = CornerRadius::same(8);
        v.window_shadow = card_shadow();
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 10, 20, 115),
        };
        v.selection.bg_fill = P600;
        v.selection.stroke = Stroke::new(1.0, P200);
        v.hyperlink_color = P200;
        v.warn_fg_color = WARN;
        v.error_fg_color = BAD;
        v.text_cursor.stroke = Stroke::new(2.0, FOCUS);
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;
        v.striped = false;
        v.override_text_color = None;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = P800;
        w.noninteractive.weak_bg_fill = P800;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TX2);
        w.inactive.bg_fill = P750;
        w.inactive.weak_bg_fill = P750;
        w.inactive.bg_stroke = Stroke::new(1.0, P400);
        w.inactive.fg_stroke = Stroke::new(1.0, TX1);
        w.hovered.bg_fill = P700;
        w.hovered.weak_bg_fill = P700;
        w.hovered.bg_stroke = Stroke::new(1.0, P300);
        w.hovered.fg_stroke = Stroke::new(1.5, TX1);
        w.hovered.expansion = 0.0;
        w.active.bg_fill = P650;
        w.active.weak_bg_fill = P650;
        w.active.bg_stroke = Stroke::new(1.0, P200);
        w.active.fg_stroke = Stroke::new(1.5, TX1);
        w.active.expansion = 0.0;
        w.open.bg_fill = P700;
        w.open.weak_bg_fill = P700;
        w.open.bg_stroke = Stroke::new(1.0, P400);
        w.open.fg_stroke = Stroke::new(1.0, TX1);
        for s in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            s.corner_radius = CornerRadius::same(8);
        }

        let sp = &mut st.spacing;
        sp.item_spacing = Vec2::new(8.0, 8.0);
        sp.button_padding = Vec2::new(13.0, 7.0);
        sp.interact_size = Vec2::new(32.0, 32.0);
        sp.slider_width = 240.0;
        sp.icon_width = 16.0;
        sp.icon_spacing = 8.0;
        sp.menu_margin = Margin::same(6);
        sp.window_margin = Margin::symmetric(18, 16);
        sp.combo_height = 320.0;

        use egui::TextStyle;
        st.text_styles = [
            (TextStyle::Heading, heading()),
            (TextStyle::Body, body()),
            (TextStyle::Button, button()),
            (TextStyle::Small, small()),
            (TextStyle::Monospace, mono_body()),
        ]
        .into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.x contrast ratio.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let ch = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let (x, y) = (lum(a) + 0.05, lum(b) + 0.05);
        x.max(y) / x.min(y)
    }

    /// The pairs the spec relies on keep their contrast (spec section 7).
    #[test]
    fn text_pairs_meet_aa() {
        for (fg, bg) in [
            (TX1, P800),
            (TX2, P800),
            (TX3, P800),
            (TX3, P700),
            (ON_AC, AC),
            (AC, P800),
            (OK, OK_SOFT),
            (BAD, BAD_SOFT),
            (INFO, INFO_SOFT),
            (AC, AC_SOFT),
        ] {
            assert!(contrast(fg, bg) >= 4.5, "{fg:?} on {bg:?}");
        }
        assert!(contrast(P400, P800) >= 3.0, "component boundary");
    }
}
