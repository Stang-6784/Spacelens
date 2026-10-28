//! Colours, fonts and egui styling.

use crate::categories::Category;
use eframe::egui::{
    self, epaint::Shadow, style::ScrollStyle, Color32, CornerRadius, FontData, FontDefinitions,
    FontFamily, FontId, Margin, Stroke, Vec2,
};
use std::sync::Arc;

pub const RGB_BG: [u8; 3] = [14, 16, 20];
pub const RGB_TEXT: [u8; 3] = [232, 235, 241];
pub const RGB_BORDER: [u8; 3] = [40, 44, 56];

pub const BG: Color32 = Color32::from_rgb(14, 16, 20);
pub const SURFACE: Color32 = Color32::from_rgb(21, 24, 32);
pub const SURFACE2: Color32 = Color32::from_rgb(28, 32, 43);
pub const SURFACE3: Color32 = Color32::from_rgb(38, 43, 57);
pub const LINE: Color32 = Color32::from_rgb(35, 40, 53);
pub const TEXT: Color32 = Color32::from_rgb(232, 235, 241);
pub const MUTED: Color32 = Color32::from_rgb(146, 154, 172);
pub const FAINT: Color32 = Color32::from_rgb(98, 106, 124);
pub const ACCENT: Color32 = Color32::from_rgb(124, 108, 255);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(143, 129, 255);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(38, 36, 70);
pub const SELECT: Color32 = Color32::from_rgb(40, 42, 72);
pub const HOVER: Color32 = Color32::from_rgb(29, 33, 45);
pub const DANGER: Color32 = Color32::from_rgb(229, 72, 77);
pub const DANGER_HOVER: Color32 = Color32::from_rgb(240, 92, 97);
pub const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
pub const FOLDER: Color32 = Color32::from_rgb(165, 180, 252);

pub fn category_color(c: Category) -> Color32 {
    match c {
        Category::Other => Color32::from_rgb(148, 163, 184),
        Category::Video => Color32::from_rgb(251, 113, 133),
        Category::Image => Color32::from_rgb(251, 191, 36),
        Category::Audio => Color32::from_rgb(52, 211, 153),
        Category::Archive => Color32::from_rgb(192, 132, 252),
        Category::Document => Color32::from_rgb(96, 165, 250),
        Category::Code => Color32::from_rgb(34, 211, 238),
        Category::Application => Color32::from_rgb(251, 146, 60),
        Category::Game => Color32::from_rgb(163, 230, 53),
        Category::System => Color32::from_rgb(100, 116, 139),
    }
}

/// Background of a folder frame in the treemap; slightly lighter the deeper it is.
pub fn folder_frame(depth: u16) -> Color32 {
    let d = depth.min(6) as u8 * 5;
    Color32::from_rgb(30 + d, 34 + d, 46 + d)
}

pub fn shade(c: Color32, f: f32) -> Color32 {
    let s = |v: u8| (v as f32 * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(s(c.r()), s(c.g()), s(c.b()))
}

// Fluent icon glyphs (Segoe Fluent Icons / Segoe MDL2 Assets).
pub mod icon {
    pub const SEARCH: char = '\u{E721}';
    pub const FOLDER: char = '\u{E8B7}';
    pub const FOLDER_OPEN: char = '\u{E838}';
    pub const FILE: char = '\u{E8A5}';
    pub const CHEVRON_RIGHT: char = '\u{E76C}';
    pub const CHEVRON_DOWN: char = '\u{E70D}';
    pub const DRIVE: char = '\u{EDA2}';
    pub const USB: char = '\u{E88E}';
    pub const NETWORK: char = '\u{E8CE}';
    pub const DISC: char = '\u{E958}';
    pub const REFRESH: char = '\u{E72C}';
    pub const STOP: char = '\u{E711}';
    pub const BOLT: char = '\u{E945}';
    pub const UP: char = '\u{E74A}';
    pub const OPEN: char = '\u{E8E5}';
    pub const COPY: char = '\u{E8C8}';
    pub const DELETE: char = '\u{E74D}';
    pub const ZOOM: char = '\u{E71E}';
    pub const CHART: char = '\u{E9D2}';
    pub const INFO: char = '\u{E946}';
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
pub fn icons(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut load = |key: &str, files: &[&str]| -> bool {
        for f in files {
            if let Ok(bytes) = std::fs::read(format!("C:\\Windows\\Fonts\\{f}")) {
                fonts.font_data.insert(key.into(), Arc::new(FontData::from_owned(bytes)));
                return true;
            }
        }
        false
    };
    let has_regular = load("ui", &["segoeui.ttf"]);
    let has_semibold = load("ui-semibold", &["seguisb.ttf", "segoeuib.ttf"]);
    let has_icons = load("fluent", &["SegoeIcons.ttf", "segmdl2.ttf"]);
    // Fallbacks so file names in any script render (all ship with Windows 10/11).
    let fallbacks: Vec<String> = [
        ("thai", &["LeelawUI.ttf", "leelawad.ttf"][..]),
        ("indic", &["Nirmala.ttc", "Nirmala.ttf"][..]),
        ("japanese", &["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"][..]),
        ("chinese", &["msyh.ttc", "simsun.ttc"][..]),
        ("korean", &["malgun.ttf"][..]),
        ("african", &["ebrima.ttf"][..]),
        ("symbols", &["seguisym.ttf"][..]),
        ("emoji", &["seguiemj.ttf"][..]),
    ]
    .iter()
    .filter(|(key, files)| load(key, files))
    .map(|(key, _)| key.to_string())
    .collect();

    let mut regular_family = Vec::new();
    if has_regular {
        regular_family.push("ui".to_string());
    }
    regular_family.extend(defaults.iter().cloned());
    regular_family.extend(fallbacks);
    if has_icons {
        regular_family.push("fluent".into());
    }
    let mut semibold_family = Vec::new();
    if has_semibold {
        semibold_family.push("ui-semibold".to_string());
    }
    semibold_family.extend(regular_family.iter().cloned());
    let mut icon_family = Vec::new();
    if has_icons {
        icon_family.push("fluent".to_string());
    }
    icon_family.extend(defaults);

    fonts.families.insert(FontFamily::Proportional, regular_family);
    fonts.families.insert(FontFamily::Name("semibold".into()), semibold_family);
    fonts.families.insert(FontFamily::Name("icons".into()), icon_family);
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE2;
    v.extreme_bg_color = SURFACE2;
    v.faint_bg_color = SURFACE;
    v.override_text_color = Some(TEXT);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 70));
    v.popup_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
    v.window_shadow = Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(160) };
    v.selection.bg_fill = Color32::from_rgb(70, 62, 150);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);
    v.text_cursor.stroke = Stroke::new(1.5_f32, ACCENT_HOVER);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(7);
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke = Stroke::new(1.0_f32, TEXT);
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_fill = SURFACE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = SURFACE3;
    v.widgets.hovered.weak_bg_fill = SURFACE3;
    v.widgets.hovered.bg_fill = SURFACE3;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(48, 54, 70);
    v.widgets.active.bg_fill = Color32::from_rgb(48, 54, 70);
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 7.0);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.scroll = ScrollStyle::floating();
        s.spacing.scroll.bar_width = 8.0;
        s.interaction.selectable_labels = false;
        s.text_styles.insert(egui::TextStyle::Button, regular(13.5));
        s.text_styles.insert(egui::TextStyle::Body, regular(13.5));
    });
}
