//! Hand-drawn widgets that give the app its look.

use crate::theme::{self, *};
use eframe::egui::{
    self, pos2, text::LayoutJob, text::TextWrapping, vec2, Align2, Color32, CornerRadius, FontId,
    Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Accent,
    Ghost,
    Danger,
}

pub fn card(painter: &Painter, rect: Rect) {
    painter.rect(rect, CornerRadius::same(12), SURFACE, Stroke::new(1.0_f32, LINE), StrokeKind::Inside);
}

pub fn caption(painter: &Painter, pos: Pos2, text: &str) {
    painter.text(pos, Align2::LEFT_CENTER, text, theme::semibold(11.0), FAINT);
}

/// Single-line text truncated with an ellipsis. `pos` is the left-centre anchor.
pub fn text_clip(painter: &Painter, pos: Pos2, text: &str, font: FontId, color: Color32, max_w: f32) -> Rect {
    if max_w <= 4.0 {
        return Rect::NOTHING;
    }
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(max_w);
    let g = painter.layout_job(job);
    let r = Rect::from_min_size(pos2(pos.x, pos.y - g.size().y / 2.0), g.size());
    painter.galley(r.min, g, color);
    r
}

pub fn text_right(painter: &Painter, right_center: Pos2, text: &str, font: FontId, color: Color32) {
    painter.text(right_center, Align2::RIGHT_CENTER, text, font, color);
}

pub fn bar(painter: &Painter, rect: Rect, fraction: f32, color: Color32) {
    let cr = CornerRadius::same((rect.height() / 2.0) as u8);
    painter.rect_filled(rect, cr, SURFACE3);
    if fraction > 0.0 {
        let w = (rect.width() * fraction.clamp(0.0, 1.0)).max(rect.height());
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(w, rect.height())), cr, color);
    }
}

pub fn button(ui: &mut Ui, kind: Kind, glyph: Option<char>, text: &str, min_width: f32) -> Response {
    let font = theme::semibold(13.5);
    let fg = if kind == Kind::Ghost { TEXT } else { Color32::WHITE };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, fg);
    let icon_w = if glyph.is_some() { 22.0 } else { 0.0 };
    let size = vec2((galley.size().x + icon_w + 30.0).max(min_width), 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hot = resp.hovered();
    let (fill, stroke) = match kind {
        Kind::Accent => (if hot { ACCENT_HOVER } else { ACCENT }, Stroke::NONE),
        Kind::Danger => (if hot { DANGER_HOVER } else { DANGER }, Stroke::NONE),
        Kind::Ghost => (if hot { SURFACE3 } else { SURFACE2 }, Stroke::new(1.0_f32, LINE)),
    };
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(10), fill, stroke, StrokeKind::Inside);
    let total = icon_w + galley.size().x;
    let mut x = rect.center().x - total / 2.0;
    if let Some(g) = glyph {
        p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, g, theme::icons(14.0), fg);
        x += icon_w;
    }
    p.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn icon_button(ui: &mut Ui, glyph: char, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), SURFACE3);
    }
    p.text(rect.center(), Align2::CENTER_CENTER, glyph, theme::icons(13.0), if enabled { TEXT } else { FAINT });
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// An animated "unknown progress" bar.
pub fn busy_bar(painter: &Painter, rect: Rect, time: f64) {
    let cr = CornerRadius::same((rect.height() / 2.0) as u8);
    painter.rect_filled(rect, cr, SURFACE3);
    let seg = rect.width() * 0.3;
    let t = (time % 1.6 / 1.6) as f32;
    let x = rect.left() - seg + t * (rect.width() + seg);
    let r = Rect::from_min_max(pos2(x.max(rect.left()), rect.top()), pos2((x + seg).min(rect.right()), rect.bottom()));
    if r.width() > 0.0 {
        painter.rect_filled(r, cr, ACCENT);
    }
}
