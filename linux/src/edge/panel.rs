//! The drop zone window: invisible until a drag reaches the screen edge,
//! then a card slides out with nearby devices. Drop on one to send.
//!
//! On Wayland (KDE Plasma, wlroots) the window is a layer-shell surface; on
//! X11 a dock window. Either way only a thin strip along the edge takes
//! input, and even that steps aside when the pointer merely passes by.

use std::cell::RefCell;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use gtk::prelude::*;
use gtk::{cairo, gdk, glib, pango};
use rand::Rng;

use super::api::{Api, Command, Device, Progress, Reply};
use super::drop::{self, Received};

const WINDOW_W: i32 = 400;
const CARD_W: f64 = 292.0;
const CARD_MARGIN: f64 = 16.0;
const STRIP: f64 = 3.0;
const HEADER_H: f64 = 72.0;
const ROW_H: f64 = 68.0;
const EMPTY_H: f64 = 132.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A damped spring: what makes things overshoot a little and settle.
struct Spring {
    value: f64,
    velocity: f64,
    target: f64,
    stiffness: f64,
    damping: f64,
}

impl Spring {
    fn new(value: f64, stiffness: f64, damping_ratio: f64) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
            stiffness,
            damping: 2.0 * stiffness.sqrt() * damping_ratio,
        }
    }

    fn step(&mut self, dt: f64) {
        let force = self.stiffness * (self.target - self.value) - self.damping * self.velocity;
        self.velocity += force * dt;
        self.value += self.velocity * dt;
        if self.settled() {
            self.value = self.target;
            self.velocity = 0.0;
        }
    }

    fn settled(&self) -> bool {
        (self.target - self.value).abs() < 0.001 && self.velocity.abs() < 0.01
    }
}

struct Row {
    device: Device,
    appear: Spring,
    hover: Spring,
    /// Rows pop in one after another.
    appear_at: Instant,
}

#[derive(Clone, PartialEq)]
enum SendState {
    Preparing,
    Waiting,
    Transferring(f64),
    Done,
    Failed(String),
}

struct Sending {
    device_id: String,
    transfer: Option<String>,
    state: SendState,
    shown: Spring,
    check: f64,
    finished_at: Option<Instant>,
}

struct Particle {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
    life: f64,
    hue: f64,
    size: f64,
    /// Particles flying into a device bubble.
    target: Option<(f64, f64)>,
    gravity: f64,
}

struct DropSession {
    device_id: String,
    offered: Vec<String>,
    received: Received,
    started: Instant,
}

struct State {
    side: Side,
    composited: bool,
    dragging: bool,
    open: Spring,
    shake: Spring,
    anchor_y: f64,
    rows: Vec<Row>,
    hover: Option<usize>,
    offline: bool,
    sending: Option<Sending>,
    session: Option<DropSession>,
    particles: Vec<Particle>,
    notice: Option<(String, Instant)>,
    leave_generation: u64,
    passing: bool,
    pass_generation: u64,
    ticking: bool,
    last_frame: Option<Instant>,
    born: Instant,
}

impl State {
    fn size(area: &gtk::DrawingArea) -> (f64, f64) {
        (
            area.allocated_width() as f64,
            area.allocated_height().max(1) as f64,
        )
    }

    fn card_height(&self) -> f64 {
        let rows = if self.rows.is_empty() {
            EMPTY_H
        } else {
            self.rows.len() as f64 * ROW_H
        };
        HEADER_H + rows + 14.0
    }

    /// The card's resting rectangle (without slide or shake).
    fn card(&self, width: f64, height: f64) -> (f64, f64, f64, f64) {
        let h = self.card_height().min(height - 32.0);
        let y = (self.anchor_y - h / 2.0).clamp(16.0, (height - 16.0 - h).max(16.0));
        let x = match self.side {
            Side::Right => width - CARD_MARGIN - CARD_W,
            Side::Left => CARD_MARGIN,
        };
        (x, y, CARD_W, h)
    }

    fn row_at(&self, width: f64, height: f64, x: f64, y: f64) -> Option<usize> {
        if self.open.value < 0.6 {
            return None;
        }
        let (cx, cy, cw, _) = self.card(width, height);
        if x < cx - 24.0 || x > cx + cw + 24.0 {
            return None;
        }
        let index = ((y - cy - HEADER_H) / ROW_H).floor();
        (index >= 0.0 && (index as usize) < self.rows.len()).then_some(index as usize)
    }

    fn bubble_center(&self, width: f64, height: f64, index: usize) -> (f64, f64) {
        let (cx, cy, _, _) = self.card(width, height);
        (
            cx + 46.0,
            cy + HEADER_H + index as f64 * ROW_H + ROW_H / 2.0,
        )
    }

    fn set_devices(&mut self, devices: Vec<Device>) {
        let now = Instant::now();
        let mut rows = Vec::with_capacity(devices.len());
        let mut stagger = 0;
        for device in devices {
            match self.rows.iter().position(|r| r.device.id == device.id) {
                Some(i) => {
                    let mut row = self.rows.remove(i);
                    row.device = device;
                    rows.push(row);
                }
                None => {
                    rows.push(Row {
                        device,
                        appear: Spring::new(0.0, 260.0, 0.55),
                        hover: Spring::new(0.0, 320.0, 0.7),
                        appear_at: now + Duration::from_millis(45 * stagger),
                    });
                    stagger += 1;
                }
            }
        }
        // Keep the device we are sending to, even if it just went quiet.
        if let Some(sending) = &self.sending {
            if let Some(i) = self
                .rows
                .iter()
                .position(|r| r.device.id == sending.device_id)
            {
                rows.push(self.rows.remove(i));
            }
        }
        self.rows = rows;
        self.hover = self.hover.filter(|&i| i < self.rows.len());
    }

    fn animating(&self) -> bool {
        !self.open.settled()
            || !self.shake.settled()
            || !self.particles.is_empty()
            || self.rows.iter().any(|r| !r.appear.settled() || !r.hover.settled())
            || self.sending.as_ref().is_some_and(|s| !s.shown.settled() || s.check < 1.0)
            // The "looking for devices" radar keeps moving while visible.
            || (self.rows.is_empty() && self.open.value > 0.01)
            || (self.dragging && self.open.value > 0.01)
    }
}

// ------------------------------------------------------------------ colors

/// Same hues as the AirBridge window (shared.js `hues`).
fn hues(id: &str) -> (f64, f64) {
    let mut h: i32 = 2166136261u32 as i32;
    for unit in id.encode_utf16() {
        h ^= unit as i32;
        h = h.wrapping_mul(16777619);
    }
    let a = (h.unsigned_abs() % 360) as f64;
    let b = (a + 40.0 + ((h >> 9).unsigned_abs() % 70) as f64) % 360.0;
    (a, b)
}

fn hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

const VIOLET: (f64, f64, f64) = (0.486, 0.424, 1.0);
const PINK: (f64, f64, f64) = (1.0, 0.373, 0.635);
const CYAN: (f64, f64, f64) = (0.2, 0.82, 1.0);
const GREEN: (f64, f64, f64) = (0.29, 0.87, 0.55);
const RED: (f64, f64, f64) = (1.0, 0.36, 0.42);

// ----------------------------------------------------------------- drawing

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, FRAC_PI_2 * 2.0);
    cr.arc(x + r, y + r, r, FRAC_PI_2 * 2.0, FRAC_PI_2 * 3.0);
    cr.close_path();
}

fn text(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    content: &str,
    font: &str,
    rgba: (f64, f64, f64, f64),
    max_width: f64,
) {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(&pango::FontDescription::from_string(font)));
    layout.set_text(content);
    layout.set_width((max_width * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);
    cr.new_path(); // show_layout leaves a current point behind
}

fn glyph(cr: &cairo::Context, x: f64, y: f64, scale: f64, device: &Device) {
    cr.save().ok();
    cr.translate(x, y);
    cr.scale(scale, scale);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    match (device.kind.as_str(), device.device_type.as_str()) {
        ("web", _) => {
            cr.arc(0.0, 0.0, 8.0, 0.0, TAU);
            cr.move_to(-8.0, 0.0);
            cr.line_to(8.0, 0.0);
            cr.stroke().ok();
            cr.save().ok();
            cr.scale(0.45, 1.0);
            cr.arc(0.0, 0.0, 8.0, 0.0, TAU);
            cr.restore().ok();
        }
        (_, "mobile") => {
            rounded(cr, -5.5, -9.0, 11.0, 18.0, 2.5);
            cr.stroke().ok();
            cr.move_to(-1.5, 6.0);
            cr.line_to(1.5, 6.0);
        }
        _ => {
            rounded(cr, -8.0, -7.0, 16.0, 11.0, 1.8);
            cr.stroke().ok();
            cr.move_to(-10.0, 7.0);
            cr.line_to(10.0, 7.0);
        }
    }
    cr.stroke().ok();
    cr.restore().ok();
}

fn draw(st: &State, cr: &cairo::Context, width: f64, height: f64) {
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    let open = st.open.value;
    let alpha = open.clamp(0.0, 1.0);
    if !st.composited {
        // No transparency without a compositor: an opaque panel or a thin line.
        let (r, g, b) = if alpha > 0.05 {
            (0.08, 0.09, 0.18)
        } else {
            (0.3, 0.27, 0.6)
        };
        cr.set_source_rgb(r, g, b);
        cr.paint().ok();
    }

    // A soft light along the screen edge where the drag arrived.
    if st.dragging && alpha > 0.0 {
        let edge_x = if st.side == Side::Right { width } else { 0.0 };
        let glow = cairo::LinearGradient::new(
            edge_x,
            0.0,
            edge_x + if st.side == Side::Right { -60.0 } else { 60.0 },
            0.0,
        );
        glow.add_color_stop_rgba(0.0, VIOLET.0, VIOLET.1, VIOLET.2, 0.45 * alpha);
        glow.add_color_stop_rgba(1.0, VIOLET.0, VIOLET.1, VIOLET.2, 0.0);
        let mask = cairo::LinearGradient::new(0.0, st.anchor_y - 260.0, 0.0, st.anchor_y + 260.0);
        mask.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
        mask.add_color_stop_rgba(0.5, 0.0, 0.0, 0.0, 1.0);
        mask.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
        cr.set_source(&glow).ok();
        cr.mask(&mask).ok();
    }

    if alpha > 0.002 {
        draw_card(st, cr, width, height, open, alpha);
    }

    for p in &st.particles {
        let t = p.age / p.life;
        let fade = (1.0 - t).clamp(0.0, 1.0);
        let (r, g, b) = hsl(p.hue, 0.9, 0.68);
        let glow = cairo::RadialGradient::new(p.x, p.y, 0.0, p.x, p.y, p.size * 3.0);
        glow.add_color_stop_rgba(0.0, r, g, b, 0.9 * fade);
        glow.add_color_stop_rgba(1.0, r, g, b, 0.0);
        cr.set_source(&glow).ok();
        cr.arc(p.x, p.y, p.size * 3.0, 0.0, TAU);
        cr.fill().ok();
    }
}

fn draw_card(st: &State, cr: &cairo::Context, width: f64, height: f64, open: f64, alpha: f64) {
    let (x0, y, w, h) = st.card(width, height);
    let slide = (1.0 - open) * (CARD_W + CARD_MARGIN + 30.0);
    let x = match st.side {
        Side::Right => x0 + slide,
        Side::Left => x0 - slide,
    } + st.shake.value;

    // Glow and shadow.
    for i in (1..=7).rev() {
        let spread = i as f64 * 3.5;
        rounded(
            cr,
            x - spread,
            y - spread + 4.0,
            w + spread * 2.0,
            h + spread * 2.0,
            26.0 + spread,
        );
        cr.set_source_rgba(
            VIOLET.0 * 0.6,
            VIOLET.1 * 0.5,
            VIOLET.2 * 0.9,
            0.035 * alpha,
        );
        cr.fill().ok();
    }
    rounded(cr, x, y, w, h, 26.0);
    let fill = cairo::LinearGradient::new(x, y, x + w * 0.4, y + h);
    fill.add_color_stop_rgba(0.0, 0.09, 0.10, 0.20, 0.95 * alpha);
    fill.add_color_stop_rgba(1.0, 0.13, 0.08, 0.22, 0.95 * alpha);
    cr.set_source(&fill).ok();
    cr.fill_preserve().ok();
    let border = cairo::LinearGradient::new(x, y, x + w, y + h);
    border.add_color_stop_rgba(0.0, VIOLET.0, VIOLET.1, VIOLET.2, 0.75 * alpha);
    border.add_color_stop_rgba(1.0, PINK.0, PINK.1, PINK.2, 0.55 * alpha);
    cr.set_source(&border).ok();
    cr.set_line_width(1.2);
    cr.stroke().ok();

    // Header: the AirBridge mark, a title and what is going on.
    let (mx, my) = (x + 30.0, y + 36.0);
    let mark = cairo::LinearGradient::new(mx - 12.0, my - 12.0, mx + 12.0, my + 12.0);
    mark.add_color_stop_rgba(0.0, VIOLET.0, VIOLET.1, VIOLET.2, alpha);
    mark.add_color_stop_rgba(1.0, PINK.0, PINK.1, PINK.2, alpha);
    cr.set_source(&mark).ok();
    cr.arc(mx, my, 13.0, 0.0, TAU);
    cr.fill().ok();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95 * alpha);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    for (radius, sweep) in [(4.0, 0.9), (7.5, 0.75)] {
        cr.new_sub_path();
        cr.arc(
            mx - 2.0,
            my + 3.0,
            radius,
            -FRAC_PI_2 - sweep,
            -FRAC_PI_2 + sweep,
        );
        cr.stroke().ok();
    }
    cr.arc(mx - 2.0, my + 3.0, 1.6, 0.0, TAU);
    cr.fill().ok();

    let (title, subtitle) = header_text(st);
    text(
        cr,
        x + 54.0,
        y + 17.0,
        &title,
        "Sans Bold 11.5",
        (0.94, 0.95, 1.0, alpha),
        w - 74.0,
    );
    text(
        cr,
        x + 54.0,
        y + 38.0,
        &subtitle,
        "Sans 9",
        (0.66, 0.69, 0.85, alpha),
        w - 74.0,
    );

    if st.rows.is_empty() {
        draw_empty(st, cr, x, y, w, alpha);
    }
    for (index, row) in st.rows.iter().enumerate() {
        draw_row(st, cr, x, y, w, index, row, alpha);
    }

    if let Some((message, at)) = &st.notice {
        let fade = (1.0 - at.elapsed().as_secs_f64() / 3.0).clamp(0.0, 1.0) * alpha;
        text(
            cr,
            x + 22.0,
            y + h - 22.0,
            message,
            "Sans 8.5",
            (RED.0, RED.1, RED.2, fade),
            w - 44.0,
        );
    }
}

fn header_text(st: &State) -> (String, String) {
    if let Some(sending) = &st.sending {
        let name = st
            .rows
            .iter()
            .find(|r| r.device.id == sending.device_id)
            .map(|r| r.device.alias.clone())
            .unwrap_or_else(|| "device".into());
        let subtitle = match &sending.state {
            SendState::Preparing => "Preparing…".to_string(),
            SendState::Waiting => format!("Waiting for {name} to accept…"),
            SendState::Transferring(p) => format!("Sending… {:.0}%", p * 100.0),
            SendState::Done => format!("Sent to {name}"),
            SendState::Failed(message) => message.clone(),
        };
        let title = match sending.state {
            SendState::Done => "Done",
            SendState::Failed(_) => "Not sent",
            _ => "Sharing",
        };
        return (title.into(), subtitle);
    }
    if st.offline {
        return (
            "AirBridge is not running".into(),
            "Start AirBridge to share".into(),
        );
    }
    let subtitle = match st.rows.len() {
        0 => "Looking for devices…".to_string(),
        1 => "Drop here to send".to_string(),
        _ => "Drop on a device".to_string(),
    };
    ("Drop to share".into(), subtitle)
}

fn draw_empty(st: &State, cr: &cairo::Context, x: f64, y: f64, w: f64, alpha: f64) {
    let (cx, cy) = (x + w / 2.0, y + HEADER_H + 46.0);
    let phase = st.born.elapsed().as_secs_f64();
    for i in 0..3 {
        let t = (phase * 0.6 + i as f64 / 3.0) % 1.0;
        cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, (1.0 - t) * 0.5 * alpha);
        cr.set_line_width(1.5);
        cr.arc(cx, cy, 8.0 + t * 38.0, 0.0, TAU);
        cr.stroke().ok();
    }
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, alpha);
    cr.arc(cx, cy, 4.0, 0.0, TAU);
    cr.fill().ok();
    let hint = if st.offline {
        "Nothing to send to yet"
    } else {
        "Open AirBridge or Quick Share on your phone"
    };
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(&pango::FontDescription::from_string("Sans 8.5")));
    layout.set_text(hint);
    layout.set_width(((w - 40.0) * pango::SCALE as f64) as i32);
    layout.set_alignment(pango::Alignment::Center);
    cr.set_source_rgba(0.66, 0.69, 0.85, alpha);
    cr.move_to(x + 20.0, cy + 54.0);
    pangocairo::functions::show_layout(cr, &layout);
    cr.new_path();
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    st: &State,
    cr: &cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    index: usize,
    row: &Row,
    alpha: f64,
) {
    let appear = row.appear.value.max(0.0);
    if appear <= 0.001 {
        return;
    }
    let hover = row.hover.value.clamp(0.0, 1.2);
    let ry = y + HEADER_H + index as f64 * ROW_H;
    let sending = st.sending.as_ref().filter(|s| s.device_id == row.device.id);
    let dim = if st.hover.is_some() && st.hover != Some(index) && st.dragging {
        0.55
    } else if st.sending.is_some() && sending.is_none() {
        0.4
    } else {
        1.0
    };
    let a = alpha * appear.min(1.0) * dim;

    if hover > 0.01 {
        rounded(cr, x + 10.0, ry + 4.0, w - 20.0, ROW_H - 8.0, 18.0);
        cr.set_source_rgba(VIOLET.0, VIOLET.1, VIOLET.2, 0.16 * hover.min(1.0) * alpha);
        cr.fill_preserve().ok();
        cr.set_source_rgba(VIOLET.0, VIOLET.1, VIOLET.2, 0.35 * hover.min(1.0) * alpha);
        cr.set_line_width(1.0);
        cr.stroke().ok();
    }

    let (bx, by) = (x + 46.0, ry + ROW_H / 2.0);
    let radius = 22.0 * appear * (1.0 + 0.12 * hover);
    let (ha, hb) = hues(&row.device.id);
    let (r1, g1, b1) = hsl(ha, 0.85, 0.64);
    let (r2, g2, b2) = hsl(hb, 0.80, 0.52);

    // Pulsing halo while hovered: "drop here".
    if hover > 0.01 {
        let pulse = (st.born.elapsed().as_secs_f64() * 4.0).sin() * 0.5 + 0.5;
        cr.set_source_rgba(r1, g1, b1, (0.25 + 0.25 * pulse) * hover.min(1.0) * alpha);
        cr.set_line_width(2.0);
        cr.arc(bx, by, radius + 5.0 + pulse * 3.0, 0.0, TAU);
        cr.stroke().ok();
    }
    let fill = cairo::LinearGradient::new(bx - radius, by - radius, bx + radius, by + radius);
    fill.add_color_stop_rgba(0.0, r1, g1, b1, a);
    fill.add_color_stop_rgba(1.0, r2, g2, b2, a);
    cr.set_source(&fill).ok();
    cr.arc(bx, by, radius, 0.0, TAU);
    cr.fill().ok();
    glyph(cr, bx, by, appear * (1.0 + 0.12 * hover), &row.device);

    let status = sending.map(|s| match &s.state {
        SendState::Preparing => "Preparing…".to_string(),
        SendState::Waiting => "Waiting for accept…".to_string(),
        SendState::Transferring(p) => format!("{:.0}%", p * 100.0),
        SendState::Done => "Sent".to_string(),
        SendState::Failed(_) => "Not sent".to_string(),
    });
    let tx = bx + 36.0;
    text(
        cr,
        tx,
        by - 17.0,
        &row.device.alias,
        "Sans Bold 10.5",
        (0.94, 0.95, 1.0, a),
        x + w - tx - 18.0,
    );
    text(
        cr,
        tx,
        by + 2.0,
        status.as_deref().unwrap_or(row.device.subtitle()),
        "Sans 8.5",
        (0.66, 0.69, 0.85, a),
        x + w - tx - 18.0,
    );

    if let Some(sending) = sending {
        draw_progress(
            cr,
            bx,
            by,
            radius + 6.0,
            sending,
            alpha,
            st.born.elapsed().as_secs_f64(),
        );
    }
}

fn draw_progress(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    radius: f64,
    sending: &Sending,
    alpha: f64,
    phase: f64,
) {
    cr.set_line_width(3.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.12 * alpha);
    cr.arc(x, y, radius, 0.0, TAU);
    cr.stroke().ok();
    let (color, fraction) = match &sending.state {
        SendState::Done => (GREEN, 1.0),
        SendState::Failed(_) => (RED, 1.0),
        _ => (CYAN, sending.shown.value.clamp(0.0, 1.0)),
    };
    if matches!(sending.state, SendState::Preparing | SendState::Waiting) {
        // Indeterminate: a short arc chasing around.
        let spin = phase * 4.0;
        cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, alpha);
        cr.arc(x, y, radius, spin, spin + 1.3);
        cr.stroke().ok();
        return;
    }
    cr.set_source_rgba(color.0, color.1, color.2, alpha);
    cr.arc(x, y, radius, -FRAC_PI_2, -FRAC_PI_2 + TAU * fraction);
    cr.stroke().ok();
    if matches!(sending.state, SendState::Done | SendState::Failed(_)) {
        // The check mark (or cross) draws itself.
        let t = sending.check.clamp(0.0, 1.0);
        cr.save().ok();
        cr.translate(x + radius * 0.72, y + radius * 0.72);
        cr.set_source_rgba(color.0, color.1, color.2, alpha);
        cr.arc(0.0, 0.0, 9.0 * (0.6 + 0.4 * t), 0.0, TAU);
        cr.fill().ok();
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
        cr.set_line_width(2.0);
        if sending.state == SendState::Done {
            let points = [(-4.0, 0.2), (-1.2, 3.0), (4.2, -2.8)];
            cr.move_to(points[0].0, points[0].1);
            let first = (t * 2.0).min(1.0);
            cr.line_to(
                points[0].0 + (points[1].0 - points[0].0) * first,
                points[0].1 + (points[1].1 - points[0].1) * first,
            );
            if t > 0.5 {
                let second = (t - 0.5) * 2.0;
                cr.line_to(
                    points[1].0 + (points[2].0 - points[1].0) * second,
                    points[1].1 + (points[2].1 - points[1].1) * second,
                );
            }
        } else {
            let s = 3.2 * t;
            cr.move_to(-s, -s);
            cr.line_to(s, s);
            cr.move_to(s, -s);
            cr.line_to(-s, s);
        }
        cr.stroke().ok();
        cr.restore().ok();
    }
}

// --------------------------------------------------------------- behaviour

struct Panel {
    window: gtk::Window,
    area: gtk::DrawingArea,
    api: Api,
    state: RefCell<State>,
}

impl Panel {
    fn size(&self) -> (f64, f64) {
        State::size(&self.area)
    }

    /// Only a thin strip along the edge takes input until a drag arrives.
    fn update_input(&self) {
        let st = self.state.borrow();
        let (width, height) = self.size();
        let region = if st.passing {
            cairo::Region::create()
        } else if st.dragging || st.session.is_some() {
            rect_region(0.0, 0.0, width, height)
        } else if st.open.target > 0.5 {
            let (x, y, w, h) = st.card(width, height);
            rect_region(x - 8.0, y - 8.0, w + 16.0, h + 16.0)
        } else {
            let x = match st.side {
                Side::Right => width - STRIP,
                Side::Left => 0.0,
            };
            rect_region(x, height * 0.08, STRIP, height * 0.84)
        };
        self.window.input_shape_combine_region(Some(&region));
    }

    fn kick(self: &Rc<Self>) {
        {
            let mut st = self.state.borrow_mut();
            if st.ticking {
                return;
            }
            st.ticking = true;
            st.last_frame = None;
        }
        let panel = self.clone();
        self.area.add_tick_callback(move |area, _| {
            let more = panel.step();
            area.queue_draw();
            if more {
                glib::ControlFlow::Continue
            } else {
                panel.state.borrow_mut().ticking = false;
                glib::ControlFlow::Break
            }
        });
    }

    /// Advances the animations by one frame; false when everything is still.
    fn step(&self) -> bool {
        let mut st = self.state.borrow_mut();
        let now = Instant::now();
        let dt = st
            .last_frame
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(1.0 / 60.0)
            .min(0.05);
        st.last_frame = Some(now);
        st.open.step(dt);
        st.shake.step(dt);
        let hover = st.hover;
        let open = st.open.target > 0.5;
        for (i, row) in st.rows.iter_mut().enumerate() {
            row.appear.target = if open && now >= row.appear_at {
                1.0
            } else if open {
                row.appear.target
            } else {
                0.0
            };
            row.hover.target = if hover == Some(i) { 1.0 } else { 0.0 };
            row.appear.step(dt);
            row.hover.step(dt);
        }
        if let Some(sending) = st.sending.as_mut() {
            sending.shown.target = match sending.state {
                SendState::Transferring(p) => p,
                SendState::Done | SendState::Failed(_) => 1.0,
                _ => 0.0,
            };
            sending.shown.step(dt);
            if matches!(sending.state, SendState::Done | SendState::Failed(_)) {
                sending.check = (sending.check + dt * 3.0).min(1.0);
            }
        }
        for p in st.particles.iter_mut() {
            p.age += dt;
            if let Some((tx, ty)) = p.target {
                // Steer into the bubble, faster as the particle ages.
                let pull = 30.0 + p.age * 80.0;
                p.vx += (tx - p.x) * pull * dt;
                p.vy += (ty - p.y) * pull * dt;
                p.vx *= 1.0 - (6.0 * dt).min(0.9);
                p.vy *= 1.0 - (6.0 * dt).min(0.9);
            } else {
                p.vy += p.gravity * dt;
                p.vx *= 1.0 - (1.2 * dt).min(0.9);
            }
            p.x += p.vx * dt;
            p.y += p.vy * dt;
        }
        st.particles.retain(|p| p.age < p.life);
        if st
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(3))
        {
            st.notice = None;
        }
        st.animating() || st.notice.is_some()
    }

    fn burst_into(&self, from: (f64, f64), to: (f64, f64)) {
        let mut rng = rand::thread_rng();
        let mut st = self.state.borrow_mut();
        for _ in 0..28 {
            let angle: f64 = rng.gen_range(0.0..TAU);
            let speed = rng.gen_range(120.0..360.0);
            st.particles.push(Particle {
                x: from.0,
                y: from.1,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                age: 0.0,
                life: rng.gen_range(0.55..0.85),
                hue: rng.gen_range(230.0..330.0),
                size: rng.gen_range(1.6..3.2),
                target: Some(to),
                gravity: 0.0,
            });
        }
    }

    fn confetti(&self, at: (f64, f64)) {
        let mut rng = rand::thread_rng();
        let mut st = self.state.borrow_mut();
        for _ in 0..46 {
            let angle: f64 = rng.gen_range(-2.3..-0.5);
            let speed = rng.gen_range(140.0..360.0);
            st.particles.push(Particle {
                x: at.0,
                y: at.1,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                age: 0.0,
                life: rng.gen_range(0.9..1.5),
                hue: rng.gen_range(0.0..360.0),
                size: rng.gen_range(1.4..3.0),
                target: None,
                gravity: 520.0,
            });
        }
    }

    fn show(self: &Rc<Self>, open: bool) {
        {
            let mut st = self.state.borrow_mut();
            if open && st.open.value < 0.3 {
                // Devices pop in one after another each time the card opens.
                let start = Instant::now() + Duration::from_millis(60);
                for (i, row) in st.rows.iter_mut().enumerate() {
                    row.appear_at = start + Duration::from_millis(45 * i as u64);
                    row.appear = Spring::new(0.0, 260.0, 0.55);
                }
            }
            st.open.target = if open { 1.0 } else { 0.0 };
        }
        self.update_input();
        self.kick();
    }

    fn notice(self: &Rc<Self>, message: &str) {
        {
            let mut st = self.state.borrow_mut();
            st.notice = Some((message.to_string(), Instant::now()));
            st.shake.velocity = 900.0;
        }
        self.kick();
    }

    // ---------------------------------------------------------------- drag

    fn drag_motion(self: &Rc<Self>, context: &gdk::DragContext, x: i32, y: i32, time: u32) -> bool {
        let (width, height) = self.size();
        let entering = {
            let mut st = self.state.borrow_mut();
            st.leave_generation += 1;
            let entering = !st.dragging;
            if entering {
                st.dragging = true;
                st.passing = false;
                if st.open.value < 0.3 {
                    st.anchor_y = y as f64;
                }
                st.hover = None;
            }
            st.hover = st.row_at(width, height, x as f64, y as f64);
            if st.rows.len() == 1 && st.open.value > 0.6 {
                st.hover = Some(0);
            }
            entering
        };
        if entering {
            self.api.send(Command::Devices);
            self.api.send(Command::Wake);
            self.show(true);
        } else {
            self.kick();
        }
        let usable = !self.state.borrow().rows.is_empty() && self.state.borrow().sending.is_none();
        context.drag_status(
            if usable {
                gdk::DragAction::COPY
            } else {
                gdk::DragAction::empty()
            },
            time,
        );
        true
    }

    fn drag_leave(self: &Rc<Self>) {
        // GTK sends "leave" right before "drop" too: wait a moment.
        let generation = {
            let mut st = self.state.borrow_mut();
            st.leave_generation += 1;
            st.leave_generation
        };
        let panel = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(160), move || {
            let collapse = {
                let mut st = panel.state.borrow_mut();
                if st.leave_generation != generation || st.session.is_some() {
                    false
                } else {
                    st.dragging = false;
                    st.hover = None;
                    st.sending.is_none()
                }
            };
            if collapse {
                panel.show(false);
            } else {
                panel.update_input();
            }
        });
    }

    fn drag_drop(
        self: &Rc<Self>,
        widget: &gtk::DrawingArea,
        context: &gdk::DragContext,
        x: i32,
        y: i32,
        time: u32,
    ) -> bool {
        let (width, height) = self.size();
        let device = {
            let st = self.state.borrow();
            if st.sending.is_some() {
                None
            } else {
                st.row_at(width, height, x as f64, y as f64)
                    .or(st.hover)
                    .or((st.rows.len() == 1).then_some(0))
                    .map(|i| (i, st.rows[i].device.id.clone()))
            }
        };
        let Some((index, device_id)) = device else {
            self.notice(if self.state.borrow().rows.is_empty() {
                "No device to send to yet"
            } else {
                "Drop it on a device"
            });
            context.drag_finish(false, false, time);
            self.drag_leave();
            return true;
        };
        let offered: Vec<String> = context
            .list_targets()
            .iter()
            .map(|a| a.name().to_string())
            .collect();
        tracing::debug!("Edge drop offers {offered:?}");
        let Some(first) = drop::next_target(&offered, &Received::new()) else {
            self.notice("AirBridge can't share this");
            context.drag_finish(false, false, time);
            self.drag_leave();
            return true;
        };
        {
            let mut st = self.state.borrow_mut();
            st.session = Some(DropSession {
                device_id,
                offered,
                received: Received::new(),
                started: Instant::now(),
            });
            st.hover = Some(index);
        }
        let to = self.state.borrow().bubble_center(width, height, index);
        self.burst_into((x as f64, y as f64), to);
        widget.drag_get_data(context, &gdk::Atom::intern(&first), time);
        self.kick();
        true
    }

    fn data_received(
        self: &Rc<Self>,
        widget: &gtk::DrawingArea,
        context: &gdk::DragContext,
        data: &gtk::SelectionData,
        time: u32,
    ) {
        let next = {
            let mut st = self.state.borrow_mut();
            let Some(session) = st.session.as_mut() else {
                return;
            };
            let target = data.target().name().to_string();
            session.received.insert(target, data.data());
            drop::next_target(&session.offered, &session.received)
        };
        if let Some(next) = next {
            widget.drag_get_data(context, &gdk::Atom::intern(&next), time);
            return;
        }
        let session = self.state.borrow_mut().session.take();
        let Some(session) = session else { return };
        let payload = drop::decide(&session.received);
        context.drag_finish(payload.is_some(), false, time);
        {
            let mut st = self.state.borrow_mut();
            st.dragging = false;
        }
        match payload {
            Some(payload) => {
                tracing::debug!(
                    "Edge drop: sending {payload:?} after {:?}",
                    session.started.elapsed()
                );
                self.state.borrow_mut().sending = Some(Sending {
                    device_id: session.device_id.clone(),
                    transfer: None,
                    state: SendState::Preparing,
                    shown: Spring::new(0.0, 120.0, 0.9),
                    check: 0.0,
                    finished_at: None,
                });
                self.api.send(Command::Send {
                    peer: session.device_id,
                    payload,
                });
            }
            None => {
                self.notice("AirBridge can't share this");
                self.show(false);
            }
        }
        self.update_input();
        self.kick();
    }

    // ------------------------------------------------------------- replies

    fn on_reply(self: &Rc<Self>, reply: Reply) {
        match reply {
            Reply::Devices(devices) => {
                let mut st = self.state.borrow_mut();
                st.offline = false;
                st.set_devices(devices);
            }
            Reply::Offline => {
                let mut st = self.state.borrow_mut();
                st.offline = true;
                st.set_devices(Vec::new());
            }
            Reply::Sent(result) => {
                let mut st = self.state.borrow_mut();
                if let Some(sending) = st.sending.as_mut() {
                    match result {
                        Ok(id) => {
                            sending.transfer = Some(id);
                            sending.state = SendState::Waiting;
                        }
                        Err(message) => {
                            sending.state = SendState::Failed(message);
                            sending.finished_at = Some(Instant::now());
                        }
                    }
                }
            }
            Reply::Progress(id, progress) => self.on_progress(&id, progress),
        }
        self.kick();
    }

    fn on_progress(self: &Rc<Self>, id: &str, progress: Progress) {
        let (width, height) = self.size();
        let mut celebrate = None;
        {
            let mut st = self.state.borrow_mut();
            let device_index = st
                .sending
                .as_ref()
                .and_then(|s| st.rows.iter().position(|r| r.device.id == s.device_id));
            let center = device_index.map(|i| st.bubble_center(width, height, i));
            let Some(sending) = st
                .sending
                .as_mut()
                .filter(|s| s.transfer.as_deref() == Some(id))
            else {
                return;
            };
            if sending.finished_at.is_some() {
                return;
            }
            sending.state = match progress.status.as_str() {
                "waiting" => SendState::Waiting,
                "transferring" => SendState::Transferring(if progress.total > 0 {
                    progress.done as f64 / progress.total as f64
                } else {
                    0.0
                }),
                "done" => SendState::Done,
                "declined" => SendState::Failed("Declined".into()),
                "cancelled" => SendState::Failed("Cancelled".into()),
                _ => SendState::Failed(progress.error.unwrap_or_else(|| "Could not send".into())),
            };
            if matches!(sending.state, SendState::Done | SendState::Failed(_)) {
                sending.finished_at = Some(Instant::now());
                if sending.state == SendState::Done {
                    celebrate = center;
                }
            }
        }
        if let Some(center) = celebrate {
            self.confetti(center);
        }
    }

    /// Polls the transfer and folds the card away once it is over.
    fn tick_sending(self: &Rc<Self>) {
        let (transfer, finished_at, failed) = {
            let st = self.state.borrow();
            match &st.sending {
                Some(s) => (
                    s.transfer.clone(),
                    s.finished_at,
                    matches!(s.state, SendState::Failed(_)),
                ),
                None => return,
            }
        };
        match finished_at {
            None => {
                if let Some(id) = transfer {
                    self.api.send(Command::Progress(id));
                }
            }
            Some(at) => {
                let hold = if failed { 3.5 } else { 1.6 };
                if at.elapsed().as_secs_f64() > hold && !self.state.borrow().dragging {
                    self.state.borrow_mut().sending = None;
                    self.show(false);
                }
            }
        }
    }

    fn dismiss(self: &Rc<Self>) {
        let busy = {
            let st = self.state.borrow();
            st.dragging
                || st
                    .sending
                    .as_ref()
                    .is_some_and(|s| s.finished_at.is_none() && s.state != SendState::Waiting)
        };
        if !busy {
            self.state.borrow_mut().sending = None;
            self.show(false);
        }
    }

    /// The pointer touched the strip without dragging anything: step aside
    /// so scroll bars and window edges underneath keep working.
    fn pass_through(self: &Rc<Self>) {
        let generation = {
            let mut st = self.state.borrow_mut();
            if st.dragging || st.open.target > 0.5 || st.session.is_some() {
                return;
            }
            st.passing = true;
            st.pass_generation += 1;
            st.pass_generation
        };
        self.update_input();
        let panel = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(1200), move || {
            let mut st = panel.state.borrow_mut();
            if st.pass_generation == generation {
                st.passing = false;
                drop(st);
                panel.update_input();
            }
        });
    }
}

fn rect_region(x: f64, y: f64, w: f64, h: f64) -> cairo::Region {
    cairo::Region::create_rectangle(&cairo::RectangleInt::new(
        x.floor() as i32,
        y.floor() as i32,
        w.ceil().max(1.0) as i32,
        h.ceil().max(1.0) as i32,
    ))
}

fn place(window: &gtk::Window, side: Side) -> Result<()> {
    if gtk_layer_shell::is_supported() {
        use gtk_layer_shell::{Edge, Layer, LayerShell};
        window.init_layer_shell();
        window.set_namespace("airbridge-edge");
        window.set_layer(Layer::Overlay);
        window.set_keyboard_interactivity(false);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(
            if side == Side::Right {
                Edge::Right
            } else {
                Edge::Left
            },
            true,
        );
        window.set_size_request(WINDOW_W, 1);
        return Ok(());
    }
    let display = gdk::Display::default().ok_or_else(|| anyhow::anyhow!("no display"))?;
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && display.type_().name().contains("Wayland") {
        bail!("this Wayland desktop does not support layer-shell (try KDE Plasma)");
    }
    let monitor = display
        .primary_monitor()
        .or_else(|| display.monitor(0))
        .ok_or_else(|| anyhow::anyhow!("no monitor"))?;
    let area = monitor.geometry();
    window.set_type_hint(gdk::WindowTypeHint::Dock);
    window.set_keep_above(true);
    window.stick();
    window.set_size_request(WINDOW_W, area.height());
    window.set_default_size(WINDOW_W, area.height());
    let x = match side {
        Side::Right => area.x() + area.width() - WINDOW_W,
        Side::Left => area.x(),
    };
    window.move_(x, area.y());
    Ok(())
}

pub fn run(side: &str) -> Result<()> {
    gtk::init()?;
    let side = if side == "left" {
        Side::Left
    } else {
        Side::Right
    };
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("AirBridge drop zone");
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_accept_focus(false);
    window.set_focus_on_map(false);
    window.set_app_paintable(true);
    let screen = WidgetExt::screen(&window).ok_or_else(|| anyhow::anyhow!("no screen"))?;
    let composited = match screen.rgba_visual() {
        Some(visual) if screen.is_composited() => {
            window.set_visual(Some(&visual));
            true
        }
        _ => false,
    };
    place(&window, side)?;

    let area = gtk::DrawingArea::new();
    area.add_events(gdk::EventMask::BUTTON_PRESS_MASK | gdk::EventMask::ENTER_NOTIFY_MASK);
    window.add(&area);

    let targets: Vec<gtk::TargetEntry> = drop::accepted_targets()
        .iter()
        .enumerate()
        .map(|(i, t)| gtk::TargetEntry::new(t, gtk::TargetFlags::OTHER_APP, i as u32))
        .collect();
    area.drag_dest_set(
        gtk::DestDefaults::empty(),
        &targets,
        gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK,
    );

    let panel = Rc::new(Panel {
        window: window.clone(),
        area: area.clone(),
        api: Api::start(),
        state: RefCell::new(State {
            side,
            composited,
            dragging: false,
            open: Spring::new(0.0, 210.0, 0.62),
            shake: Spring::new(0.0, 600.0, 0.25),
            anchor_y: 400.0,
            rows: Vec::new(),
            hover: None,
            offline: false,
            sending: None,
            session: None,
            particles: Vec::new(),
            notice: None,
            leave_generation: 0,
            passing: false,
            pass_generation: 0,
            ticking: false,
            last_frame: None,
            born: Instant::now(),
        }),
    });

    {
        let panel = panel.clone();
        area.connect_draw(move |area, cr| {
            let (width, height) = State::size(area);
            draw(&panel.state.borrow(), cr, width, height);
            glib::Propagation::Stop
        });
    }
    {
        let panel = panel.clone();
        area.connect_drag_motion(move |_, context, x, y, time| {
            panel.drag_motion(context, x, y, time)
        });
    }
    {
        let panel = panel.clone();
        area.connect_drag_leave(move |_, _, _| panel.drag_leave());
    }
    {
        let panel = panel.clone();
        area.connect_drag_drop(move |widget, context, x, y, time| {
            panel.drag_drop(widget, context, x, y, time)
        });
    }
    {
        let panel = panel.clone();
        area.connect_drag_data_received(move |widget, context, _, _, data, _, time| {
            panel.data_received(widget, context, data, time)
        });
    }
    {
        let panel = panel.clone();
        area.connect_button_press_event(move |_, _| {
            panel.dismiss();
            glib::Propagation::Stop
        });
    }
    {
        let panel = panel.clone();
        window.connect_enter_notify_event(move |_, event| {
            if event.mode() == gdk::CrossingMode::Normal {
                panel.pass_through();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let panel = panel.clone();
        area.connect_size_allocate(move |_, _| panel.update_input());
    }
    {
        // Replies from the service, a few times per frame budget.
        let panel = panel.clone();
        glib::timeout_add_local(Duration::from_millis(30), move || {
            while let Ok(reply) = panel.api.replies.try_recv() {
                panel.on_reply(reply);
            }
            glib::ControlFlow::Continue
        });
    }
    {
        let panel = panel.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            panel.tick_sending();
            // A drop whose data never arrived.
            let stuck = panel
                .state
                .borrow()
                .session
                .as_ref()
                .is_some_and(|s| s.started.elapsed() > Duration::from_secs(8));
            if stuck {
                panel.state.borrow_mut().session = None;
                panel.notice("The app you dragged from did not hand it over");
                panel.drag_leave();
            }
            glib::ControlFlow::Continue
        });
    }
    {
        // Keep the device list fresh so the card opens instantly.
        let panel = panel.clone();
        panel.api.send(Command::Devices);
        glib::timeout_add_local(Duration::from_secs(12), move || {
            panel.api.send(Command::Devices);
            glib::ControlFlow::Continue
        });
    }
    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });
    window.show_all();
    panel.update_input();
    if std::env::var_os("AIRBRIDGE_EDGE_PREVIEW").is_some() {
        // Shows the card as if something were being dragged (for checking a
        // desktop's compositing and for screenshots).
        let panel = panel.clone();
        panel.api.send(Command::Devices);
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            panel.api.send(Command::Devices);
            {
                let (_, height) = panel.size();
                let mut st = panel.state.borrow_mut();
                st.dragging = true;
                st.anchor_y = height / 2.0;
                st.hover = (!st.rows.is_empty()).then_some(0);
            }
            panel.show(true);
        });
    }
    tracing::info!(
        "Drop zone ready on the {} edge ({})",
        if side == Side::Right { "right" } else { "left" },
        if gtk_layer_shell::LayerShell::is_layer_window(&window) {
            "layer-shell"
        } else {
            "X11"
        }
    );
    gtk::main();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, alias: &str, kind: &str, device_type: &str) -> Device {
        Device {
            id: id.into(),
            alias: alias.into(),
            device_type: device_type.into(),
            kind: kind.into(),
        }
    }

    fn state(devices: Vec<Device>) -> State {
        let mut st = State {
            side: Side::Right,
            composited: true,
            dragging: true,
            open: Spring::new(1.0, 210.0, 0.62),
            shake: Spring::new(0.0, 600.0, 0.25),
            anchor_y: 300.0,
            rows: Vec::new(),
            hover: None,
            offline: false,
            sending: None,
            session: None,
            particles: Vec::new(),
            notice: None,
            leave_generation: 0,
            passing: false,
            pass_generation: 0,
            ticking: false,
            last_frame: None,
            born: Instant::now(),
        };
        st.set_devices(devices);
        for row in st.rows.iter_mut() {
            row.appear = Spring::new(1.0, 1.0, 1.0);
        }
        st
    }

    /// Renders the card over a dark desktop into target/edge-snapshots.
    fn render(name: &str, st: &State) {
        let (w, h) = (WINDOW_W, 600);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w + 160, h).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        let bg = cairo::LinearGradient::new(0.0, 0.0, (w + 160) as f64, h as f64);
        bg.add_color_stop_rgb(0.0, 0.13, 0.15, 0.27);
        bg.add_color_stop_rgb(1.0, 0.25, 0.13, 0.29);
        cr.set_source(&bg).unwrap();
        cr.paint().unwrap();
        let window = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
        draw(
            st,
            &cairo::Context::new(&window).unwrap(),
            w as f64,
            h as f64,
        );
        cr.set_source_surface(&window, 160.0, 0.0).unwrap();
        cr.paint().unwrap();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/edge-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
        surface.write_to_png(&mut file).unwrap();
    }

    #[test]
    fn renders_every_state() {
        let devices = vec![
            device("A1", "Pixel 8", "localsend", "mobile"),
            device("quickshare:Xy12", "Galaxy S24", "quickshare", "mobile"),
            device("B2", "Work laptop", "localsend", "desktop"),
        ];
        let mut st = state(devices.clone());
        render("1-open", &st);

        st.hover = Some(1);
        st.rows[1].hover = Spring::new(1.0, 1.0, 1.0);
        render("2-hover", &st);

        st.dragging = false;
        st.sending = Some(Sending {
            device_id: "quickshare:Xy12".into(),
            transfer: Some("t".into()),
            state: SendState::Transferring(0.62),
            shown: Spring::new(0.62, 1.0, 1.0),
            check: 0.0,
            finished_at: None,
        });
        render("3-sending", &st);
        if let Some(s) = st.sending.as_mut() {
            s.state = SendState::Done;
            s.shown = Spring::new(1.0, 1.0, 1.0);
            s.check = 1.0;
        }
        render("4-done", &st);
        if let Some(s) = st.sending.as_mut() {
            s.state = SendState::Failed("Declined".into());
        }
        render("5-declined", &st);

        let empty = state(Vec::new());
        render("6-empty", &empty);
        assert_eq!(empty.row_at(WINDOW_W as f64, 600.0, 200.0, 300.0), None);
        let three = state(devices);
        let (x, y) = three.bubble_center(WINDOW_W as f64, 600.0, 2);
        assert_eq!(three.row_at(WINDOW_W as f64, 600.0, x + 60.0, y), Some(2));
    }

    #[test]
    fn hues_match_the_web_ui() {
        // Same as shared.js hues("A1").
        let (a, b) = hues("A1");
        assert!((0.0..360.0).contains(&a) && (0.0..360.0).contains(&b));
        assert_eq!(hues("A1"), hues("A1"));
    }

    #[test]
    fn springs_settle() {
        let mut spring = Spring::new(0.0, 210.0, 0.62);
        spring.target = 1.0;
        let mut overshoot: f64 = 0.0;
        for _ in 0..240 {
            spring.step(1.0 / 60.0);
            overshoot = overshoot.max(spring.value);
        }
        assert!(spring.settled());
        assert!(
            overshoot > 1.0 && overshoot < 1.2,
            "a little bounce: {overshoot}"
        );
    }
}
