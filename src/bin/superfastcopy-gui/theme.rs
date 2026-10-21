//! Colors, fonts and the egui style.
//!
//! Uses the fonts every Windows install has: Segoe UI for text and Segoe
//! Fluent Icons / MDL2 Assets for icons, falling back to egui's built-in
//! fonts if they're missing.

use std::path::PathBuf;

use eframe::egui::{self, vec2, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x0E, 0x10, 0x16);
pub const CARD: Color32 = Color32::from_rgb(0x16, 0x19, 0x22);
pub const CARD_HI: Color32 = Color32::from_rgb(0x1E, 0x22, 0x2D);
pub const WELL: Color32 = Color32::from_rgb(0x10, 0x13, 0x1A);
pub const BORDER: Color32 = Color32::from_rgb(0x28, 0x2D, 0x3A);
pub const TRACK: Color32 = Color32::from_rgb(0x23, 0x28, 0x34);
pub const TEXT: Color32 = Color32::from_rgb(0xE9, 0xEC, 0xF3);
pub const MUTED: Color32 = Color32::from_rgb(0x8A, 0x92, 0xA6);
pub const ACCENT: Color32 = Color32::from_rgb(0x7C, 0x5C, 0xFF);
pub const ACCENT2: Color32 = Color32::from_rgb(0x00, 0xC6, 0xFF);
pub const GOOD: Color32 = Color32::from_rgb(0x2E, 0xD5, 0x73);
pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xB4, 0x3C);
pub const BAD: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x7A);

/// Segoe Fluent Icons / MDL2 Assets code points.
pub mod icon {
    pub const FOLDER: char = '\u{E8B7}';
    pub const FILE: char = '\u{E8A5}';
    pub const NEW_FOLDER: char = '\u{E8F4}';
    pub const ADD: char = '\u{E710}';
    pub const PASTE: char = '\u{E77F}';
    pub const COPY: char = '\u{E8C8}';
    pub const DELETE: char = '\u{E74D}';
    pub const CLOSE: char = '\u{E711}';
    pub const FORWARD: char = '\u{E72A}';
    pub const BOLT: char = '\u{E945}';
    pub const CHECK: char = '\u{E73E}';
    pub const ERROR: char = '\u{E783}';
    pub const WARNING: char = '\u{E7BA}';
    pub const OPEN: char = '\u{E838}';
    pub const DROP: char = '\u{E896}';
    pub const REFRESH: char = '\u{E72C}';
    pub const DRIVE: char = '\u{EDA2}';
    pub const STOP: char = '\u{E71A}';
    pub const RECENT: char = '\u{E823}';
    pub const STOPWATCH: char = '\u{E916}';
}

/// Stroke::new is generic over the width, so plain float literals need a type.
pub fn stroke(width: f32, color: Color32) -> Stroke {
    Stroke::new(width, color)
}

pub fn icons() -> FontFamily {
    FontFamily::Name("icons".into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    let mut load = |key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts.font_data.insert(key.into(), FontData::from_owned(bytes));
                true
            }
            Err(_) => false,
        }
    };
    let has_segoe = load("segoe", "segoeui.ttf");
    let has_semibold = load("segoe-sb", "seguisb.ttf");
    let has_fluent = load("fluent-icons", "SegoeIcons.ttf");
    let has_mdl2 = load("mdl2-icons", "segmdl2.ttf");

    let proportional = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    if has_segoe {
        proportional.insert(0, "segoe".into());
    }
    let base = proportional.clone();
    let mut sb = Vec::new();
    if has_semibold {
        sb.push("segoe-sb".to_string());
    }
    sb.extend(base.iter().cloned());
    fonts.families.insert(semibold(), sb);
    let mut ic = Vec::new();
    if has_fluent {
        ic.push("fluent-icons".to_string());
    }
    if has_mdl2 {
        ic.push("mdl2-icons".to_string());
    }
    ic.extend(base);
    fonts.families.insert(icons(), ic);
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Body, FontId::proportional(14.5)),
        (TextStyle::Button, FontId::proportional(14.5)),
        (TextStyle::Heading, FontId::new(22.0, semibold())),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(14.0, 7.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.scroll.floating = true;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = WELL;
    v.faint_bg_color = CARD_HI;
    v.hyperlink_color = ACCENT2;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.55);
    v.selection.stroke = stroke(1.0, TEXT);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(10.0);
    v.window_stroke = stroke(1.0, BORDER);
    v.text_cursor.stroke = stroke(2.0, ACCENT2);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.bg_stroke = stroke(1.0, BORDER);
    w.noninteractive.fg_stroke = stroke(1.0, TEXT);
    for (state, fill, border) in [
        (&mut w.inactive, CARD_HI, BORDER),
        (&mut w.hovered, Color32::from_rgb(0x27, 0x2C, 0x3A), ACCENT.gamma_multiply(0.7)),
        (&mut w.active, Color32::from_rgb(0x2E, 0x34, 0x44), ACCENT),
        (&mut w.open, CARD_HI, ACCENT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke(1.0, border);
        state.fg_stroke = stroke(1.5, TEXT);
        state.rounding = Rounding::same(10.0);
        state.expansion = 0.0;
    }
    ctx.set_style(style);
}
