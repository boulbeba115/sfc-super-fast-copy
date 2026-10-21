//! Custom-painted widgets: gradient buttons, toggles, segmented control,
//! progress ring, speed graph.

use std::collections::VecDeque;
use std::f32::consts::{FRAC_PI_2, TAU};

use eframe::egui::{
    self, epaint::Mesh, pos2, text::LayoutJob, vec2, Align, Align2, Color32, CursorIcon, FontId, Painter, Pos2, Rect,
    Response, RichText, Sense, Shape, Stroke, TextFormat, Ui, Vec2,
};

use crate::theme::*;

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// Outline of a rounded rectangle, clockwise from the top-left corner.
fn rounded_points(rect: Rect, r: f32) -> Vec<Pos2> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -FRAC_PI_2),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), FRAC_PI_2),
        (pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI),
    ];
    let mut pts = Vec::with_capacity(40);
    for (c, start) in corners {
        for i in 0..=8 {
            let a = start + FRAC_PI_2 * i as f32 / 8.0;
            pts.push(c + vec2(a.cos(), a.sin()) * r);
        }
    }
    pts
}

/// Rounded rectangle filled with a left-to-right gradient.
pub fn gradient_rect(p: &Painter, rect: Rect, r: f32, a: Color32, b: Color32) {
    let pts = rounded_points(rect, r);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.center(), lerp(a, b, 0.5));
    for pt in &pts {
        mesh.colored_vertex(*pt, lerp(a, b, (pt.x - rect.left()) / rect.width()));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(mesh));
}

pub fn dashed_rect(p: &Painter, rect: Rect, r: f32, stroke: Stroke) {
    let mut pts = rounded_points(rect, r);
    pts.push(pts[0]);
    p.extend(Shape::dashed_line(&pts, stroke, 7.0, 5.0));
}

/// Icon glyph followed by text, both vertically centered.
pub fn icon_text(icon: char, text: &str, size: f32, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |family| TextFormat { font_id: FontId::new(size, family), color, valign: Align::Center, ..Default::default() };
    job.append(&icon.to_string(), 0.0, fmt(icons()));
    if !text.is_empty() {
        job.append(text, size * 0.6, fmt(semibold()));
    }
    job
}

pub fn icon_label(icon: char, size: f32, color: Color32) -> RichText {
    RichText::new(icon.to_string()).family(icons()).size(size).color(color)
}

/// The big gradient call-to-action button.
pub fn primary_button(ui: &mut Ui, icon: char, text: &str, size: Vec2, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover = enabled && resp.hovered();
    let press = enabled && resp.is_pointer_button_down_on();
    let p = ui.painter();
    if enabled {
        let (a, b) = if hover { (lerp(ACCENT, Color32::WHITE, 0.12), lerp(ACCENT2, Color32::WHITE, 0.12)) } else { (ACCENT, ACCENT2) };
        let glow = if hover { 0.35 } else { 0.2 };
        gradient_rect(p, rect.expand(4.0).translate(vec2(0.0, 3.0)), 16.0, a.gamma_multiply(glow), b.gamma_multiply(glow));
        gradient_rect(p, if press { rect.shrink(1.5) } else { rect }, 12.0, a, b);
    } else {
        p.rect(rect, 12.0, CARD_HI, stroke(1.0, BORDER));
    }
    let color = if enabled { Color32::WHITE } else { MUTED };
    let galley = ui.fonts(|f| f.layout_job(icon_text(icon, text, 16.0, color)));
    p.galley(rect.center() - galley.size() / 2.0, galley, color);
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Secondary button with an icon.
pub fn ghost_button(ui: &mut Ui, icon: char, text: &str) -> Response {
    ui.add(egui::Button::new(icon_text(icon, text, 14.0, TEXT)).min_size(vec2(0.0, 38.0)).rounding(10.0))
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// Small square icon-only button.
pub fn icon_button(ui: &mut Ui, icon: char, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 8.0, CARD_HI);
    }
    let color = if resp.hovered() { TEXT } else { MUTED };
    p.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::new(15.0, icons()), color);
    resp.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand)
}

pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let p = ui.painter();
    if t > 0.0 {
        gradient_rect(p, rect, 11.0, lerp(TRACK, ACCENT, t), lerp(TRACK, ACCENT2, t));
    }
    if t < 1.0 {
        p.rect(rect, 11.0, TRACK.gamma_multiply(1.0 - t), stroke(1.0, BORDER.gamma_multiply(1.0 - t)));
    }
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    p.circle_filled(pos2(x, rect.center().y), 8.0, lerp(MUTED, Color32::WHITE, t));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Toggle with a title and a muted description; clicking the text toggles too.
pub fn toggle_row(ui: &mut Ui, on: &mut bool, title: &str, sub: &str) -> Response {
    ui.horizontal(|ui| {
        let mut resp = toggle(ui, on);
        ui.add_space(4.0);
        let text = ui
            .vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(title).color(TEXT));
                ui.label(RichText::new(sub).size(12.0).color(MUTED));
            })
            .response
            .interact(Sense::click());
        if text.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        resp
    })
    .inner
}

/// Row of mutually exclusive options; returns true when the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, id: &str, value: &mut T, opts: &[(T, &str)]) -> bool {
    let font = FontId::new(13.5, semibold());
    let widths: Vec<f32> = opts
        .iter()
        .map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), TEXT).size().x + 30.0)
        .collect();
    let h = 36.0;
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 8.0, h), Sense::hover());
    ui.painter().rect(rect, 11.0, WELL, stroke(1.0, BORDER));
    let mut changed = false;
    let mut x = rect.left() + 4.0;
    for (i, (v, label)) in opts.iter().enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.top() + 4.0), vec2(widths[i], h - 8.0));
        x += widths[i];
        let resp = ui.interact(seg, ui.id().with((id, i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() && *value != *v {
            *value = *v;
            changed = true;
        }
        let selected = *value == *v;
        let p = ui.painter();
        if selected {
            gradient_rect(p, seg, 8.0, ACCENT, ACCENT2);
        } else if resp.hovered() {
            p.rect_filled(seg, 8.0, CARD_HI);
        }
        let color = if selected { Color32::WHITE } else if resp.hovered() { TEXT } else { MUTED };
        p.text(seg.center(), Align2::CENTER_CENTER, *label, font.clone(), color);
    }
    changed
}

pub fn card<R>(ui: &mut Ui, size: Vec2, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .stroke(stroke(1.0, BORDER))
        .rounding(16.0)
        .inner_margin(egui::Margin::same(18.0))
        .show(ui, |ui| {
            ui.set_width(size.x - 36.0);
            ui.set_min_height((size.y - 36.0).max(0.0));
            // A frame inherits its parent's layout; cards always stack vertically.
            ui.vertical(add).inner
        })
        .inner
}

/// Small spaced-out uppercase heading inside cards.
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(MUTED).strong().extra_letter_spacing(1.2));
}

/// Rounded label with a colored dot, e.g. the status in the header.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    let font = FontId::new(12.5, semibold());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = vec2(galley.size().x + 32.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect(rect, 13.0, color.gamma_multiply(0.12), stroke(1.0, color.gamma_multiply(0.35)));
    p.circle_filled(pos2(rect.left() + 13.0, rect.center().y), 3.5, color);
    p.galley(pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

pub fn badge(ui: &mut Ui, text: &str) {
    let font = FontId::new(11.5, semibold());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 20.0), Sense::hover());
    gradient_rect(ui.painter(), rect, 10.0, ACCENT, ACCENT2);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// Thin horizontal bar, e.g. disk usage.
pub fn bar(ui: &mut Ui, frac: f32, height: f32, a: Color32, b: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, height / 2.0, TRACK);
    let frac = frac.clamp(0.0, 1.0);
    if frac > 0.0 {
        let fill = Rect::from_min_size(rect.min, vec2((rect.width() * frac).max(height), height));
        gradient_rect(p, fill, height / 2.0, a, b);
    }
}

/// Circular progress. `spin` (seconds) draws an indeterminate spinner instead.
pub fn ring(p: &Painter, c: Pos2, r: f32, width: f32, frac: f32, spin: Option<f64>, a: Color32, b: Color32) {
    p.circle_stroke(c, r, stroke(width, TRACK));
    let (start, sweep) = match spin {
        Some(t) => ((t * 4.0 % TAU as f64) as f32, 0.3 * TAU),
        None => (0.0, frac.clamp(0.0, 1.0) * TAU),
    };
    if sweep < 0.001 {
        return;
    }
    let pt = |ang: f32| c + vec2((ang - FRAC_PI_2).cos(), (ang - FRAC_PI_2).sin()) * r;
    let color = |t: f32| if spin.is_some() { lerp(a, b, t) } else { lerp(a, b, t * sweep / TAU) };
    let n = ((sweep / TAU) * 180.0).ceil().max(3.0) as usize;
    for (w, alpha) in [(width * 2.4, 0.10), (width, 1.0)] {
        for i in 0..n {
            let t0 = i as f32 / n as f32;
            let t1 = ((i as f32 + 1.4) / n as f32).min(1.0);
            p.line_segment([pt(start + sweep * t0), pt(start + sweep * t1)], stroke(w, color(t0).gamma_multiply(alpha)));
        }
    }
    // Round caps, except on a closed ring where they'd show as a dot.
    if sweep < TAU - 0.01 {
        p.circle_filled(pt(start), width / 2.0, color(0.0));
        p.circle_filled(pt(start + sweep), width / 2.0, color(1.0));
    }
}

/// Area chart of recent speed samples, newest on the right.
pub fn sparkline(ui: &mut Ui, data: &VecDeque<f32>, capacity: usize, height: f32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 10.0, WELL);
    p.text(rect.left_top() + vec2(10.0, 6.0), Align2::LEFT_TOP, label, FontId::proportional(10.5), MUTED.gamma_multiply(0.8));
    for i in 1..4 {
        let y = rect.top() + rect.height() * i as f32 / 4.0;
        p.line_segment([pos2(rect.left() + 8.0, y), pos2(rect.right() - 8.0, y)], stroke(1.0, TRACK.gamma_multiply(0.6)));
    }
    if data.len() < 2 {
        return;
    }
    let inner = rect.shrink2(vec2(2.0, 6.0));
    let max = data.iter().copied().fold(1.0f32, f32::max) * 1.15;
    let step = inner.width() / (capacity.max(2) - 1) as f32;
    let pts: Vec<Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(inner.right() - (data.len() - 1 - i) as f32 * step, inner.bottom() - inner.height() * v / max))
        .collect();

    let mut mesh = Mesh::default();
    for (i, pt) in pts.iter().enumerate() {
        let c = lerp(ACCENT, ACCENT2, (pt.x - rect.left()) / rect.width());
        mesh.colored_vertex(*pt, c.gamma_multiply(0.38));
        mesh.colored_vertex(pos2(pt.x, rect.bottom()), Color32::TRANSPARENT);
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    p.add(Shape::mesh(mesh));
    for w in pts.windows(2) {
        let c = lerp(ACCENT, ACCENT2, (w[1].x - rect.left()) / rect.width());
        p.line_segment([w[0], w[1]], stroke(2.2, c));
    }
    let last = *pts.last().unwrap();
    p.circle_filled(last, 6.0, ACCENT2.gamma_multiply(0.25));
    p.circle_filled(last, 3.5, ACCENT2);
}

/// Labeled value used in the progress card.
pub fn stat(ui: &mut Ui, width: f32, icon: char, label: &str, value: &str, sub: &str) {
    egui::Frame::none().fill(WELL).rounding(12.0).inner_margin(egui::Margin::symmetric(12.0, 10.0)).show(ui, |ui| {
        ui.set_width(width - 24.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(icon_label(icon, 12.0, MUTED));
                ui.label(RichText::new(label).size(11.5).color(MUTED).extra_letter_spacing(0.6));
            });
            ui.label(RichText::new(value).family(semibold()).size(16.0).color(TEXT));
            ui.label(RichText::new(if sub.is_empty() { " " } else { sub }).size(11.5).color(MUTED));
        });
    });
}

/// Shortens a path from the left to roughly `max` characters.
pub fn elide_start(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max || max < 4 {
        s.to_string()
    } else {
        format!("…{}", s.chars().skip(n - max + 1).collect::<String>())
    }
}
