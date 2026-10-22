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

use super::api::{Api, Command, Device, Progress, Reply};
use super::drop::{self, Received};

const WINDOW_W: i32 = 400;
const CARD_W: f64 = 292.0;
const CARD_MARGIN: f64 = 16.0;
const STRIP: f64 = 3.0;
const HEADER_H: f64 = 72.0;
const ROW_H: f64 = 68.0;
const EMPTY_H: f64 = 120.0;
/// The card slides out with just a hint of overshoot.
const OPEN_DAMPING: f64 = 0.85;

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
    /// Follows the desktop's light or dark style.
    dark: bool,
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

    #[cfg(test)]
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
                        appear: Spring::new(0.0, 260.0, 0.9),
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
            || self.rows.iter().any(|r| !r.appear.settled() || !r.hover.settled())
            || self.sending.as_ref().is_some_and(|s| !s.shown.settled() || s.check < 1.0)
            // The "looking for devices" waves keep moving while visible.
            || (self.rows.is_empty() && self.open.value > 0.01)
            || (self.dragging && self.open.value > 0.01)
    }
}

// ------------------------------------------------------------------ colors

type Rgb = (f64, f64, f64);

/// The Apple-like palette of the AirBridge window: a frosted card, gray
/// device avatars and one blue accent, in a light and a dark variant.
struct Palette {
    card: Rgb,
    hairline: (f64, f64, f64, f64),
    separator: (f64, f64, f64, f64),
    fill: (f64, f64, f64, f64),
    text: Rgb,
    muted: Rgb,
    accent: Rgb,
    danger: Rgb,
    avatar_top: Rgb,
    avatar_bottom: Rgb,
    shadow: f64,
}

const LIGHT: Palette = Palette {
    card: (0.976, 0.976, 0.98),
    hairline: (0.0, 0.0, 0.0, 0.1),
    separator: (0.235, 0.235, 0.263, 0.16),
    fill: (0.47, 0.47, 0.5, 0.12),
    text: (0.114, 0.114, 0.122),
    muted: (0.431, 0.431, 0.451),
    accent: (0.0, 0.478, 1.0),
    danger: (1.0, 0.231, 0.188),
    avatar_top: (0.718, 0.733, 0.769),
    avatar_bottom: (0.545, 0.565, 0.6),
    shadow: 0.05,
};

const DARK: Palette = Palette {
    card: (0.173, 0.173, 0.18),
    hairline: (1.0, 1.0, 1.0, 0.1),
    separator: (0.33, 0.33, 0.345, 0.65),
    fill: (0.47, 0.47, 0.5, 0.24),
    text: (0.96, 0.96, 0.97),
    muted: (0.596, 0.596, 0.616),
    accent: (0.039, 0.518, 1.0),
    danger: (1.0, 0.271, 0.227),
    avatar_top: (0.49, 0.49, 0.514),
    avatar_bottom: (0.333, 0.333, 0.353),
    shadow: 0.09,
};

impl State {
    fn palette(&self) -> &'static Palette {
        if self.dark {
            &DARK
        } else {
            &LIGHT
        }
    }
}

fn rgba(c: Rgb, a: f64) -> (f64, f64, f64, f64) {
    (c.0, c.1, c.2, a)
}

/// Whether the desktop uses a dark style (Breeze Dark, Adwaita-dark, …).
fn desktop_is_dark() -> bool {
    let Some(settings) = gtk::Settings::default() else {
        return false;
    };
    settings.is_gtk_application_prefer_dark_theme()
        || settings
            .gtk_theme_name()
            .is_some_and(|name| name.to_lowercase().contains("dark"))
}

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

    let pal = st.palette();
    let open = st.open.value;
    let alpha = open.clamp(0.0, 1.0);
    if !st.composited {
        // No transparency without a compositor: an opaque panel or a thin line.
        let c = if alpha > 0.05 { pal.card } else { pal.accent };
        cr.set_source_rgb(c.0, c.1, c.2);
        cr.paint().ok();
    }

    // A soft blue light along the screen edge where the drag arrived.
    if st.dragging && alpha > 0.0 {
        let edge_x = if st.side == Side::Right { width } else { 0.0 };
        let glow = cairo::LinearGradient::new(
            edge_x,
            0.0,
            edge_x + if st.side == Side::Right { -48.0 } else { 48.0 },
            0.0,
        );
        let a = pal.accent;
        glow.add_color_stop_rgba(0.0, a.0, a.1, a.2, 0.28 * alpha);
        glow.add_color_stop_rgba(1.0, a.0, a.1, a.2, 0.0);
        let mask = cairo::LinearGradient::new(0.0, st.anchor_y - 240.0, 0.0, st.anchor_y + 240.0);
        mask.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
        mask.add_color_stop_rgba(0.5, 0.0, 0.0, 0.0, 1.0);
        mask.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
        cr.set_source(&glow).ok();
        cr.mask(&mask).ok();
    }

    if alpha > 0.002 {
        draw_card(st, cr, width, height, open, alpha);
    }
}

/// The app's mark: a blue rounded square with the waves glyph.
fn draw_mark(cr: &cairo::Context, x: f64, y: f64, size: f64, color: Rgb, alpha: f64) {
    rounded(cr, x, y, size, size, size * 0.225);
    cr.set_source_rgba(color.0, color.1, color.2, alpha);
    cr.fill().ok();
    let (cx, cy) = (x + size / 2.0, y + size * 0.6);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.set_line_width(size * 0.075);
    cr.set_line_cap(cairo::LineCap::Round);
    for (radius, sweep) in [(size * 0.17, 0.85), (size * 0.32, 0.75)] {
        cr.new_sub_path();
        cr.arc(cx, cy, radius, -FRAC_PI_2 - sweep, -FRAC_PI_2 + sweep);
        cr.stroke().ok();
    }
    cr.arc(cx, cy, size * 0.065, 0.0, TAU);
    cr.fill().ok();
}

fn draw_card(st: &State, cr: &cairo::Context, width: f64, height: f64, open: f64, alpha: f64) {
    let pal = st.palette();
    let (x0, y, w, h) = st.card(width, height);
    let slide = (1.0 - open) * (CARD_W + CARD_MARGIN + 30.0);
    let x = match st.side {
        Side::Right => x0 + slide,
        Side::Left => x0 - slide,
    } + st.shake.value;

    // A soft, neutral shadow like a macOS popover.
    for i in (1..=6).rev() {
        let spread = i as f64 * 3.0;
        rounded(
            cr,
            x - spread,
            y - spread + 6.0,
            w + spread * 2.0,
            h + spread * 2.0,
            14.0 + spread,
        );
        cr.set_source_rgba(0.0, 0.0, 0.0, pal.shadow * alpha);
        cr.fill().ok();
    }
    rounded(cr, x, y, w, h, 14.0);
    let (r, g, b, _) = rgba(pal.card, 0.0);
    cr.set_source_rgba(r, g, b, 0.96 * alpha);
    cr.fill_preserve().ok();
    let l = pal.hairline;
    cr.set_source_rgba(l.0, l.1, l.2, l.3 * alpha);
    cr.set_line_width(1.0);
    cr.stroke().ok();

    // Header: the AirBridge mark, a title and what is going on.
    draw_mark(cr, x + 16.0, y + 20.0, 28.0, pal.accent, alpha);
    let (title, subtitle) = header_text(st);
    text(
        cr,
        x + 54.0,
        y + 17.0,
        &title,
        "Sans Bold 11",
        rgba(pal.text, alpha),
        w - 72.0,
    );
    let subtitle_color = match st.sending.as_ref().map(|s| &s.state) {
        Some(SendState::Failed(_)) => pal.danger,
        Some(SendState::Done | SendState::Transferring(_)) => pal.accent,
        _ => pal.muted,
    };
    text(
        cr,
        x + 54.0,
        y + 37.0,
        &subtitle,
        "Sans 9",
        rgba(subtitle_color, alpha),
        w - 72.0,
    );
    let sep = pal.separator;
    cr.set_source_rgba(sep.0, sep.1, sep.2, sep.3 * alpha);
    cr.rectangle(x + 16.0, y + HEADER_H - 6.0, w - 32.0, 0.6);
    cr.fill().ok();

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
            x + 18.0,
            y + h - 22.0,
            message,
            "Sans 8.5",
            rgba(pal.danger, fade),
            w - 36.0,
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
            SendState::Waiting => format!("Waiting for {name}…"),
            SendState::Transferring(_) => format!("Sending to {name}…"),
            SendState::Done => format!("Sent to {name}"),
            SendState::Failed(message) => message.clone(),
        };
        let title = match sending.state {
            SendState::Done => "Sent",
            SendState::Failed(_) => "Not Sent",
            _ => "AirBridge",
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
    ("AirBridge".into(), subtitle)
}

fn draw_empty(st: &State, cr: &cairo::Context, x: f64, y: f64, w: f64, alpha: f64) {
    let pal = st.palette();
    let (cx, cy) = (x + w / 2.0, y + HEADER_H + 40.0);
    // Slow waves around the mark, like the window's "visible" beacon.
    let phase = st.born.elapsed().as_secs_f64();
    if !st.offline {
        for i in 0..2 {
            let t = (phase / 4.0 + i as f64 / 2.0) % 1.0;
            let a = pal.accent;
            cr.set_source_rgba(a.0, a.1, a.2, (1.0 - t) * 0.32 * alpha);
            cr.set_line_width(1.5);
            cr.arc(cx, cy, 16.0 * (1.0 + 1.2 * t), 0.0, TAU);
            cr.stroke().ok();
        }
    }
    let mark = if st.offline { pal.muted } else { pal.accent };
    cr.arc(cx, cy, 16.0, 0.0, TAU);
    cr.set_source_rgba(mark.0, mark.1, mark.2, alpha);
    cr.fill().ok();
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    for (radius, sweep) in [(3.6, 0.85), (7.0, 0.75)] {
        cr.new_sub_path();
        cr.arc(cx, cy + 3.5, radius, -FRAC_PI_2 - sweep, -FRAC_PI_2 + sweep);
        cr.stroke().ok();
    }
    cr.arc(cx, cy + 3.5, 1.4, 0.0, TAU);
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
    cr.set_source_rgba(pal.muted.0, pal.muted.1, pal.muted.2, alpha);
    cr.move_to(x + 20.0, cy + 44.0);
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
    let pal = st.palette();
    let appear = row.appear.value.max(0.0);
    if appear <= 0.001 {
        return;
    }
    let hover = row.hover.value.clamp(0.0, 1.0);
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
    // Rows fade in and rise a few pixels.
    let ry = ry + (1.0 - appear.min(1.0)) * 6.0;

    // The row under the pointer gets a blue selection, like a macOS list.
    if hover > 0.01 {
        rounded(cr, x + 8.0, ry + 4.0, w - 16.0, ROW_H - 8.0, 9.0);
        let c = pal.accent;
        cr.set_source_rgba(c.0, c.1, c.2, 0.13 * hover * alpha);
        cr.fill().ok();
    }

    let (bx, by) = (x + 46.0, ry + ROW_H / 2.0);
    let radius = 22.0 * (1.0 + 0.05 * hover);
    if hover > 0.01 {
        let c = pal.accent;
        cr.set_source_rgba(c.0, c.1, c.2, hover * alpha);
        cr.set_line_width(2.0);
        cr.arc(bx, by, radius + 3.5, 0.0, TAU);
        cr.stroke().ok();
    }
    let fill = cairo::LinearGradient::new(bx, by - radius, bx, by + radius);
    let (t, b) = (pal.avatar_top, pal.avatar_bottom);
    fill.add_color_stop_rgba(0.0, t.0, t.1, t.2, a);
    fill.add_color_stop_rgba(1.0, b.0, b.1, b.2, a);
    cr.set_source(&fill).ok();
    cr.arc(bx, by, radius, 0.0, TAU);
    cr.fill().ok();
    glyph(cr, bx, by, 1.0 + 0.05 * hover, &row.device);

    let status = sending.map(|s| match &s.state {
        SendState::Preparing | SendState::Waiting => ("Waiting…".to_string(), pal.muted),
        SendState::Transferring(_) => ("Sending…".to_string(), pal.accent),
        SendState::Done => ("Sent".to_string(), pal.accent),
        SendState::Failed(_) => ("Not Sent".to_string(), pal.danger),
    });
    let tx = bx + 36.0;
    text(
        cr,
        tx,
        by - 17.0,
        &row.device.alias,
        "Sans Bold 10.5",
        rgba(pal.text, a),
        x + w - tx - 18.0,
    );
    let (line, color) = status.unwrap_or_else(|| (row.device.subtitle().to_string(), pal.muted));
    text(
        cr,
        tx,
        by + 2.0,
        &line,
        "Sans 8.5",
        rgba(color, a),
        x + w - tx - 18.0,
    );

    if let Some(sending) = sending {
        draw_progress(
            cr,
            pal,
            bx,
            by,
            radius + 5.0,
            sending,
            alpha,
            st.born.elapsed().as_secs_f64(),
        );
    }
}

/// The ring around the device being sent to, like the iOS share sheet.
#[allow(clippy::too_many_arguments)]
fn draw_progress(
    cr: &cairo::Context,
    pal: &Palette,
    x: f64,
    y: f64,
    radius: f64,
    sending: &Sending,
    alpha: f64,
    phase: f64,
) {
    cr.set_line_width(3.0);
    cr.set_line_cap(cairo::LineCap::Round);
    let f = pal.fill;
    cr.set_source_rgba(f.0, f.1, f.2, f.3 * 2.0 * alpha);
    cr.arc(x, y, radius, 0.0, TAU);
    cr.stroke().ok();
    let accent = pal.accent;
    if matches!(sending.state, SendState::Preparing | SendState::Waiting) {
        // Indeterminate: a short arc turning calmly.
        let spin = phase * 4.5;
        cr.set_source_rgba(accent.0, accent.1, accent.2, alpha);
        cr.arc(x, y, radius, spin, spin + 1.0);
        cr.stroke().ok();
        return;
    }
    let (color, fraction) = match &sending.state {
        SendState::Failed(_) => (pal.danger, 0.0),
        _ => (accent, sending.shown.value.clamp(0.0, 1.0)),
    };
    if fraction > 0.0 {
        cr.set_source_rgba(color.0, color.1, color.2, alpha);
        cr.arc(x, y, radius, -FRAC_PI_2, -FRAC_PI_2 + TAU * fraction);
        cr.stroke().ok();
    }
    if matches!(sending.state, SendState::Done | SendState::Failed(_)) {
        // A small badge fades in: a check mark, or a cross.
        let t = sending.check.clamp(0.0, 1.0);
        cr.save().ok();
        cr.translate(x + radius * 0.72, y + radius * 0.72);
        cr.set_source_rgba(pal.card.0, pal.card.1, pal.card.2, alpha * t);
        cr.arc(0.0, 0.0, 10.5, 0.0, TAU);
        cr.fill().ok();
        cr.set_source_rgba(color.0, color.1, color.2, alpha * t);
        cr.arc(0.0, 0.0, 9.0, 0.0, TAU);
        cr.fill().ok();
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha * t);
        cr.set_line_width(2.0);
        if sending.state == SendState::Done {
            cr.move_to(-4.0, 0.2);
            cr.line_to(-1.2, 3.0);
            cr.line_to(4.2, -2.8);
        } else {
            cr.move_to(-3.2, -3.2);
            cr.line_to(3.2, 3.2);
            cr.move_to(3.2, -3.2);
            cr.line_to(-3.2, 3.2);
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
        if st
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(3))
        {
            st.notice = None;
        }
        st.animating() || st.notice.is_some()
    }

    fn show(self: &Rc<Self>, open: bool) {
        {
            let mut st = self.state.borrow_mut();
            if open && st.open.value < 0.3 {
                // Devices pop in one after another each time the card opens.
                let start = Instant::now() + Duration::from_millis(60);
                for (i, row) in st.rows.iter_mut().enumerate() {
                    row.appear_at = start + Duration::from_millis(45 * i as u64);
                    row.appear = Spring::new(0.0, 260.0, 0.9);
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
        {
            let mut st = self.state.borrow_mut();
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
            }
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
            open: Spring::new(0.0, 210.0, OPEN_DAMPING),
            shake: Spring::new(0.0, 600.0, 0.25),
            anchor_y: 400.0,
            rows: Vec::new(),
            hover: None,
            offline: false,
            sending: None,
            session: None,
            dark: desktop_is_dark(),
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
            open: Spring::new(1.0, 210.0, OPEN_DAMPING),
            shake: Spring::new(0.0, 600.0, 0.25),
            anchor_y: 300.0,
            rows: Vec::new(),
            hover: None,
            offline: false,
            sending: None,
            session: None,
            dark: false,
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

    /// Renders the card over a desktop wallpaper into target/edge-snapshots,
    /// once in the light style and once in the dark one.
    fn render(name: &str, st: &mut State) {
        st.dark = false;
        render_one(name, st);
        st.dark = true;
        render_one(&format!("dark-{name}"), st);
        st.dark = false;
    }

    fn render_one(name: &str, st: &State) {
        let (w, h) = (WINDOW_W, 600);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w + 160, h).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        let bg = cairo::LinearGradient::new(0.0, 0.0, (w + 160) as f64, h as f64);
        bg.add_color_stop_rgb(0.0, 0.36, 0.55, 0.78);
        bg.add_color_stop_rgb(1.0, 0.22, 0.33, 0.55);
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
        render("1-open", &mut st);

        st.hover = Some(1);
        st.rows[1].hover = Spring::new(1.0, 1.0, 1.0);
        render("2-hover", &mut st);

        st.dragging = false;
        st.sending = Some(Sending {
            device_id: "quickshare:Xy12".into(),
            transfer: Some("t".into()),
            state: SendState::Transferring(0.62),
            shown: Spring::new(0.62, 1.0, 1.0),
            check: 0.0,
            finished_at: None,
        });
        render("3-sending", &mut st);
        if let Some(s) = st.sending.as_mut() {
            s.state = SendState::Done;
            s.shown = Spring::new(1.0, 1.0, 1.0);
            s.check = 1.0;
        }
        render("4-done", &mut st);
        if let Some(s) = st.sending.as_mut() {
            s.state = SendState::Failed("Declined".into());
        }
        render("5-declined", &mut st);

        let mut empty = state(Vec::new());
        render("6-empty", &mut empty);
        assert_eq!(empty.row_at(WINDOW_W as f64, 600.0, 200.0, 300.0), None);
        let three = state(devices);
        let (x, y) = three.bubble_center(WINDOW_W as f64, 600.0, 2);
        assert_eq!(three.row_at(WINDOW_W as f64, 600.0, x + 60.0, y), Some(2));
    }

    #[test]
    fn springs_settle() {
        let mut spring = Spring::new(0.0, 210.0, OPEN_DAMPING);
        spring.target = 1.0;
        let mut overshoot: f64 = 0.0;
        for _ in 0..240 {
            spring.step(1.0 / 60.0);
            overshoot = overshoot.max(spring.value);
        }
        assert!(spring.settled());
        assert!(overshoot < 1.02, "calm, barely any bounce: {overshoot}");
    }
}
